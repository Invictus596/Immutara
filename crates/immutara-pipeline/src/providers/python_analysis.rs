//! Real face-analysis provider that talks to a Python CV worker over an
//! NDJSON subprocess protocol.
//!
//! The provider spawns `python -m immutara_cv.face_worker`, sends a single
//! line-delimited JSON request, and parses the single line-delimited JSON
//! response. The worker performs YuNet face detection + SFace embedding using
//! OpenCV's DNN module (CPU).
//!
//! Privacy: the worker returns the raw 128-d embedding over the pipe, but
//! this provider immediately reduces it to a SHA-256 fingerprint. Only the
//! fingerprint, its dimensionality, and the selected-face structural detail
//! are recorded in the domain model — raw biometric-derived vectors are never
//! stored, logged, or surfaced to the TUI.
//!
//! Failures (missing models, missing image, no face, worker crash, malformed
//! response, timeout) are all mapped to typed `ImmutaraError` values; the
//! pipeline treats analysis failure as non-fatal.

use std::process::Stdio;

use async_trait::async_trait;
use immutara_core::ImmutaraError;
use immutara_core::config::OpenCvAnalysisConfig;
use immutara_core::domain::analysis::{AnalysisResult, BoundingBox, FaceAnalysis, SelectedFace};
use immutara_core::domain::evidence::{ContentHash, Evidence};
use immutara_core::providers::AnalysisProvider;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::time::{Duration, timeout};

/// Protocol version shared with the Python worker.
const SCHEMA_VERSION: u32 = 1;

#[derive(Serialize)]
struct AnalysisRequest {
    schema_version: u32,
    image_path: String,
}

#[derive(Default, Deserialize)]
struct AnalysisResponse {
    schema_version: u32,
    status: String,
    face_count: Option<u32>,
    selected_face: Option<SelectedFacePayload>,
    embedding: Option<EmbeddingPayload>,
    model: Option<ModelPayload>,
    error: Option<ErrorPayload>,
}

#[derive(Deserialize)]
struct SelectedFacePayload {
    confidence: f64,
    bounding_box: BoundingBoxPayload,
}

#[derive(Deserialize)]
struct BoundingBoxPayload {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[derive(Deserialize)]
struct EmbeddingPayload {
    dimensions: usize,
    values: Vec<f64>,
}

#[derive(Deserialize)]
struct ModelPayload {
    detector: String,
    recognizer: String,
    version: String,
}

#[derive(Deserialize)]
struct ErrorPayload {
    code: String,
    message: String,
}

/// Provider implementation name reported back through the domain model.
const OPENCV_PROVIDER_ID: &str = "open-cv";

/// A real analysis provider backed by the Python OpenCV worker.
pub struct PyAnalysisProvider {
    provider_id: String,
    config: OpenCvAnalysisConfig,
}

impl PyAnalysisProvider {
    pub fn new(config: OpenCvAnalysisConfig) -> Self {
        Self {
            provider_id: OPENCV_PROVIDER_ID.to_string(),
            config,
        }
    }

    /// Spawn the worker subprocess with the required environment.
    fn spawn_worker(&self) -> Result<Child, ImmutaraError> {
        let mut cmd = Command::new(&self.config.python);
        cmd.arg("-m")
            .arg(&self.config.worker_module)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .env("IMMUTARA_CV_MODELS_DIR", &self.config.models_dir)
            .env(
                "IMMUTARA_CV_DETECT_THRESHOLD",
                self.config.detection_threshold.to_string(),
            )
            .env(
                "IMMUTARA_CV_MAX_IMAGE_DIM",
                self.config.max_image_dimension.to_string(),
            )
            .env("PYTHONPATH", &self.config.python_package_dir);

        cmd.spawn().map_err(|e| ImmutaraError::Provider {
            provider: self.provider_id.clone(),
            message: format!(
                "failed to launch python worker ({}): {e}",
                self.config.python
            ),
        })
    }

    /// Perform one request/response exchange, returning the parsed response.
    async fn exchange(&self, image_path: &str) -> Result<AnalysisResponse, ImmutaraError> {
        let mut child = self.spawn_worker()?;
        let timeout_ms = Duration::from_secs(self.config.timeout_seconds.max(1));

        match timeout(timeout_ms, self.exchange_inner(&mut child, image_path)).await {
            Ok(result) => result,
            Err(_elapsed) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                Err(ImmutaraError::Provider {
                    provider: self.provider_id.clone(),
                    message: format!(
                        "analysis worker timed out after {}s",
                        self.config.timeout_seconds
                    ),
                })
            }
        }
    }

    async fn exchange_inner(
        &self,
        child: &mut Child,
        image_path: &str,
    ) -> Result<AnalysisResponse, ImmutaraError> {
        let mut stdin = child.stdin.take().ok_or_else(|| ImmutaraError::Provider {
            provider: self.provider_id.clone(),
            message: "worker stdin unavailable".to_string(),
        })?;

        let request = AnalysisRequest {
            schema_version: SCHEMA_VERSION,
            image_path: image_path.to_string(),
        };
        let line = serde_json::to_string(&request)?;
        stdin.write_all(line.as_bytes()).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;
        // Closing stdin signals EOF so the worker exits after one request.
        drop(stdin);

        let mut stdout = child.stdout.take().ok_or_else(|| ImmutaraError::Provider {
            provider: self.provider_id.clone(),
            message: "worker stdout unavailable".to_string(),
        })?;

        let mut reader = BufReader::new(&mut stdout);
        let mut response_line = String::new();
        let bytes = reader.read_line(&mut response_line).await?;

        let status = child.wait().await;

        if bytes == 0 {
            return Err(ImmutaraError::Provider {
                provider: self.provider_id.clone(),
                message: format!(
                    "analysis worker exited without a response (exit: {:?}); check that models are downloaded",
                    status.ok().and_then(|s| s.code())
                ),
            });
        }

        let response: AnalysisResponse = serde_json::from_str(response_line.trim())?;
        Ok(response)
    }
}

#[async_trait]
impl AnalysisProvider for PyAnalysisProvider {
    async fn analyze(&self, evidence: &Evidence) -> Result<AnalysisResult, ImmutaraError> {
        let source_path =
            evidence
                .metadata
                .source_path
                .clone()
                .ok_or_else(|| ImmutaraError::Provider {
                    provider: self.provider_id.clone(),
                    message: "evidence has no on-disk source_path; cannot run real CV analysis"
                        .to_string(),
                })?;

        if !source_path.is_file() {
            return Err(ImmutaraError::Provider {
                provider: self.provider_id.clone(),
                message: format!("analysis image does not exist: {}", source_path.display()),
            });
        }

        let response = self.exchange(&source_path.display().to_string()).await?;

        validate_schema_version(&response, &self.provider_id)?;

        if response.status == "error" {
            let err = response.error.unwrap_or(ErrorPayload {
                code: "unknown".to_string(),
                message: "analysis worker returned an error".to_string(),
            });
            return Err(ImmutaraError::Provider {
                provider: self.provider_id.clone(),
                message: format!("analysis failed [{}]: {}", err.code, err.message),
            });
        }

        if response.status != "ok" {
            return Err(ImmutaraError::Provider {
                provider: self.provider_id.clone(),
                message: format!("unexpected analysis status: {}", response.status),
            });
        }

        let embedding = response
            .embedding
            .ok_or_else(|| missing_field("embedding", &self.provider_id))?;
        let model = response
            .model
            .ok_or_else(|| missing_field("model", &self.provider_id))?;
        let selected = response
            .selected_face
            .ok_or_else(|| missing_field("selected_face", &self.provider_id))?;
        let face_count = response.face_count.unwrap_or(0);

        let (x, y, w, h) = (
            selected.bounding_box.x,
            selected.bounding_box.y,
            selected.bounding_box.width,
            selected.bounding_box.height,
        );

        let embedding_hash = hash_embedding(&embedding.values);

        let selected_face = SelectedFace {
            confidence: selected.confidence,
            bounding_box: BoundingBox {
                x,
                y,
                width: w,
                height: h,
            },
            embedding_dimension: embedding.dimensions,
            embedding_hash,
        };

        let face_analysis = FaceAnalysis {
            provider_id: self.provider_id.clone(),
            face_count,
            selected_face: Some(selected_face),
            detector_model: model.detector,
            recognizer_model: model.recognizer,
            model_version: model.version,
        };

        Ok(AnalysisResult {
            evidence_id: evidence.id,
            provider_id: self.provider_id.clone(),
            model_version: Some(face_analysis.model_version.clone()),
            objects: Vec::new(),
            text_regions: Vec::new(),
            face_analysis: Some(face_analysis),
            metadata_hash: ContentHash("".to_string()),
            analyzed_at: chrono::Utc::now(),
        })
    }

    fn provider_id(&self) -> &str {
        &self.provider_id
    }
}

fn validate_schema_version(
    response: &AnalysisResponse,
    provider_id: &str,
) -> Result<(), ImmutaraError> {
    if response.schema_version != SCHEMA_VERSION {
        return Err(ImmutaraError::Provider {
            provider: provider_id.to_string(),
            message: format!(
                "worker protocol schema_version mismatch: got {}, expected {SCHEMA_VERSION}",
                response.schema_version
            ),
        });
    }
    Ok(())
}

fn missing_field(name: &str, provider_id: &str) -> ImmutaraError {
    ImmutaraError::Provider {
        provider: provider_id.to_string(),
        message: format!("analysis response missing `{name}` field"),
    }
}

/// SHA-256 (hex) of the float32 little-endian bytes of an embedding.
///
/// Fingers the exact vector without persisting the raw values. The worker
/// emits float32-derived JSON; casting each value back to f32 recovers the
/// original 32-bit payload losslessly for hashing.
fn hash_embedding(values: &[f64]) -> ContentHash {
    let mut hasher = Sha256::new();
    for v in values {
        hasher.update((*v as f32).to_le_bytes());
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(hex, "{byte:02x}");
    }
    ContentHash(hex)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::process::Command as StdCommand;

    use chrono::Utc;
    use immutara_core::domain::evidence::{
        ContentHash, Evidence, EvidenceId, EvidenceMetadata, SchemaVersion,
    };
    use immutara_core::domain::provenance::ProvenanceChain;

    use super::*;

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../")
    }

    fn fixture(name: &str) -> PathBuf {
        repo_root()
            .join("py")
            .join("tests")
            .join("fixtures")
            .join(name)
    }

    fn env_python() -> String {
        std::env::var("IMMUTARA_CV_PYTHON").unwrap_or_else(|_| "python3".to_string())
    }

    fn test_config() -> OpenCvAnalysisConfig {
        let root = repo_root();
        OpenCvAnalysisConfig {
            models_dir: root.join("models"),
            python_package_dir: root.join("py"),
            python: env_python(),
            worker_module: "immutara_cv.face_worker".to_string(),
            detection_threshold: 0.9,
            max_image_dimension: 4096,
            timeout_seconds: 30,
        }
    }

    fn evidence_at(source_path: Option<PathBuf>) -> Evidence {
        Evidence {
            id: EvidenceId::new(),
            content_hash: ContentHash("e".repeat(64)),
            metadata: EvidenceMetadata {
                source_path,
                mime_type: "image/jpeg".to_string(),
                file_size: 0,
                dimensions: None,
                captured_at: None,
                schema_version: SchemaVersion(1),
            },
            provenance: ProvenanceChain::new(),
            ingested_at: Utc::now(),
        }
    }

    /// True when the configured interpreter can import OpenCV AND the models
    /// are present. Real-worker tests are skipped otherwise.
    fn cv_ready(reason: &mut String) -> bool {
        let models = repo_root().join("models");
        let detector = models
            .join("face_detection_yunet")
            .join("face_detection_yunet_2023mar.onnx");
        let recognizer = models
            .join("face_recognition_sface")
            .join("face_recognition_sface_2021dec.onnx");
        if !detector.exists() || !recognizer.exists() {
            *reason = "models not downloaded (run `python3 models/download_models.py`)".to_string();
            return false;
        }
        let python = env_python();
        let ok = StdCommand::new(&python)
            .arg("-c")
            .arg("import cv2")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !ok {
            *reason =
                format!("`{python}` cannot import OpenCV (`pip install -r py/requirements.txt`)");
        }
        ok
    }

    // ---- Pure / protocol-logic tests (no worker, no CV environment) ----

    #[test]
    fn embedding_hash_is_sha256_of_f32_bytes() {
        let values = vec![0.1_f64, -2.0, 0.5];
        let mut hasher = Sha256::new();
        for v in &values {
            hasher.update((*v as f32).to_le_bytes());
        }
        let expected = format!("{:x}", hasher.finalize());
        assert_eq!(hash_embedding(&values).0, expected);
        assert_eq!(hash_embedding(&values).0.len(), 64);
        // Distinct vectors hash differently.
        assert_ne!(hash_embedding(&[0.1]), hash_embedding(&[0.2]));
    }

    #[test]
    fn request_serializes_to_expected_ndjson() {
        let req = AnalysisRequest {
            schema_version: SCHEMA_VERSION,
            image_path: "/tmp/img.jpg".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert_eq!(json, r#"{"schema_version":1,"image_path":"/tmp/img.jpg"}"#);
    }

    #[test]
    fn malformed_response_json_does_not_panic() {
        // A worker that returns non-JSON must surface as a typed error.
        let raw = "this is not json {";
        let parsed: Result<AnalysisResponse, _> = serde_json::from_str(raw);
        assert!(parsed.is_err());
    }

    #[test]
    fn schema_version_mismatch_is_provider_error() {
        let provider = PyAnalysisProvider::new(test_config());
        let parsed: AnalysisResponse =
            serde_json::from_str(r#"{"schema_version":0,"status":"ok"}"#).unwrap();
        let err = validate_schema_version(&parsed, &provider.provider_id).unwrap_err();
        match err {
            ImmutaraError::Provider { ref message, .. } => {
                assert!(message.contains("schema_version"));
            }
            other => panic!("expected Provider error, got {other:?}"),
        }
    }

    // ---- Error mapping tests (no worker needed; async runtime only) ----

    #[tokio::test]
    async fn analyze_missing_source_path_is_error() {
        let provider = PyAnalysisProvider::new(test_config());
        let evidence = evidence_at(None);
        let err = provider.analyze(&evidence).await.unwrap_err();
        match err {
            ImmutaraError::Provider { ref message, .. } => {
                assert!(message.contains("source_path"));
            }
            other => panic!("expected Provider error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn analyze_nonexistent_image_is_error() {
        let provider = PyAnalysisProvider::new(test_config());
        let evidence = evidence_at(Some(fixture("does_not_exist.jpg")));
        let err = provider.analyze(&evidence).await.unwrap_err();
        match err {
            ImmutaraError::Provider { ref message, .. } => {
                assert!(message.contains("does not exist"));
            }
            other => panic!("expected Provider error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unexpected_worker_exit_is_provider_error() {
        // Point worker_module at a module that cannot be imported: the worker
        // process exits immediately with no stdout, which must map to a typed
        // Provider error (no panic), regardless of whether OpenCV is present.
        let mut config = test_config();
        config.worker_module = "immutara_cv.does_not_exist".to_string();
        let provider = PyAnalysisProvider::new(config);
        let evidence = evidence_at(Some(fixture("face_lena.jpg")));
        let err = provider.analyze(&evidence).await.unwrap_err();
        assert!(matches!(err, ImmutaraError::Provider { .. }));
    }

    // ---- Real-worker integration tests (require models + OpenCV) ----

    #[tokio::test]
    async fn real_worker_analyzes_face_image() {
        let mut skip = String::new();
        if !cv_ready(&mut skip) {
            eprintln!("SKIP real_worker_analyzes_face_image: {skip}");
            return;
        }
        let provider = PyAnalysisProvider::new(test_config());
        let evidence = evidence_at(Some(fixture("face_lena.jpg")));
        let result = provider
            .analyze(&evidence)
            .await
            .expect("analysis should succeed");

        // The AnalysisResult carries no raw embedding vector, only the hash.
        let serialized = serde_json::to_string(&result).unwrap();
        assert!(!serialized.contains("values"));

        assert_eq!(result.evidence_id, evidence.id);
        assert_eq!(result.provider_id, "open-cv");

        let face = result.face_analysis.expect("face_analysis present");
        assert!(face.face_count >= 1);
        assert_eq!(face.detector_model, "YuNet");
        assert_eq!(face.recognizer_model, "SFace");
        let selected = face.selected_face.expect("selected face present");
        assert!((0.0..=1.0).contains(&selected.confidence));
        assert!(selected.embedding_dimension >= 1);
        assert_eq!(selected.embedding_hash.0.len(), 64);
        assert!(selected.bounding_box.width > 0);
        assert!(selected.bounding_box.height > 0);
    }

    #[tokio::test]
    async fn real_worker_no_face_is_error() {
        let mut skip = String::new();
        if !cv_ready(&mut skip) {
            eprintln!("SKIP real_worker_no_face_is_error: {skip}");
            return;
        }
        let provider = PyAnalysisProvider::new(test_config());
        let evidence = evidence_at(Some(fixture("no_face_blank.png")));
        let err = provider.analyze(&evidence).await.unwrap_err();
        match err {
            ImmutaraError::Provider { ref message, .. } => {
                assert!(
                    message.to_lowercase().contains("no face")
                        || message.to_lowercase().contains("no_face")
                );
            }
            other => panic!("expected Provider error, got {other:?}"),
        }
    }
}
