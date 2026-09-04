//! Application configuration.
//!
//! Provider selection is driven here: each stage names a provider
//! implementation (e.g. `mock` vs `opencv`), optionally with
//! provider-specific settings.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domain::evidence::SchemaVersion;
use crate::errors::ImmutaraError;

/// Top-level configuration loaded from TOML (with future env overrides).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub pipeline: PipelineConfig,
    pub verification: VerificationConfig,
    pub search: SearchConfig,
    pub analysis: AnalysisConfig,
    pub attestation: ProviderConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PipelineConfig {
    pub evidence_dir: String,
    pub log_level: String,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            evidence_dir: "./evidence".to_string(),
            log_level: "info".to_string(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct VerificationConfig {
    pub policy: VerificationPolicyConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VerificationPolicyConfig {
    pub version: SchemaVersion,
    pub min_search_matches: usize,
    pub min_provider_score: f64,
    pub require_analysis: bool,
    pub min_analysis_confidence: f64,
    pub required_providers: Vec<String>,
}

impl Default for VerificationPolicyConfig {
    fn default() -> Self {
        Self {
            version: SchemaVersion(1),
            min_search_matches: 0,
            min_provider_score: 0.0,
            require_analysis: false,
            min_analysis_confidence: 0.0,
            required_providers: Vec::new(),
        }
    }
}

/// Identifies which provider implementation a stage should use.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderConfig {
    pub provider: String,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            provider: "mock".to_string(),
        }
    }
}

/// Search stage configuration.
///
/// `provider` selects the implementation: `"mock"` (deterministic, no
/// external dependencies) or `"tineye"` (real reverse-image-search over the
/// public web via the TinEye API).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchConfig {
    pub provider: String,
    pub tineye: TineyeSearchConfig,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            provider: "mock".to_string(),
            tineye: TineyeSearchConfig::default(),
        }
    }
}

/// Settings for the `"tineye"` reverse-image-search provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TineyeSearchConfig {
    /// Base URL of the TinEye REST API (without trailing slash).
    ///
    /// Intended for testing against a mock server; production uses the
    /// default `https://api.tineye.com/rest`.
    pub api_url: String,
    /// TinEye API key. Committed config must never hold a real key; resolve
    /// it from an environment variable (`TINEYE_API_KEY`) at run time via
    /// [`TineyeSearchConfig::resolve_api_key`].
    pub api_key: String,
    /// When true, a real `TINEYE_API_KEY` environment variable or explicit
    /// `api_key` is **required**; the public sandbox key is never used.
    /// Set this to `true` for genuine submission validation E2E runs.
    pub require_real_key: bool,
    /// Timeout (seconds) for a single search request.
    pub timeout_seconds: u64,
}

impl Default for TineyeSearchConfig {
    fn default() -> Self {
        Self {
            api_url: "https://api.tineye.com/rest".to_string(),
            api_key: String::new(),
            require_real_key: false,
            timeout_seconds: 60,
        }
    }
}

impl TineyeSearchConfig {
    /// Resolve the effective API key.
    ///
    /// When [`TineyeSearchConfig::require_real_key`] is `true`, only an
    /// explicitly set `api_key` or the `TINEYE_API_KEY` environment variable
    /// are accepted; the public sandbox key is never used. When `false`
    /// (the default), the sandbox key is used as a last resort.
    pub fn resolve_api_key(&self) -> Result<String, ImmutaraError> {
        if !self.api_key.is_empty() {
            return Ok(self.api_key.clone());
        }
        if let Ok(key) = std::env::var("TINEYE_API_KEY") {
            return Ok(key);
        }
        if self.require_real_key {
            return Err(ImmutaraError::Config(
                "TINEYE_API_KEY environment variable not set; \
                 a real TinEye API key is required for genuine search. \
                 Set TINEYE_API_KEY or provide api_key in [search.tineye]."
                    .to_string(),
            ));
        }
        // Sandbox fallback: always returns results for the "melon cat" sample
        // image, regardless of the uploaded image.
        Ok("6mm60lsCNIBqFwOWjJqA80QZHh9BMwc-ber4u=t^".to_string())
    }
}

/// Analysis stage configuration.
///
/// `provider` selects the implementation: `"mock"` (deterministic, no
/// external dependencies) or `"opencv"` (real Python face-analysis worker).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AnalysisConfig {
    /// Provider implementation: `"mock"` or `"opencv"`.
    pub provider: String,
    /// Settings for the `"opencv"` Python worker.
    pub opencv: OpenCvAnalysisConfig,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            provider: "mock".to_string(),
            opencv: OpenCvAnalysisConfig::default(),
        }
    }
}

/// Settings for the real (OpenCV/Python) face-analysis provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenCvAnalysisConfig {
    /// Root directory containing the ONNX model subdirectories.
    ///
    /// Expects `face_detection_yunet/<file>` and `face_recognition_sface/<file>`.
    pub models_dir: PathBuf,
    /// Directory containing the `immutara_cv` Python package (its parent is
    /// added to `PYTHONPATH` when launching the worker).
    pub python_package_dir: PathBuf,
    /// Python interpreter used to launch the worker.
    pub python: String,
    /// Worker module path (must be importable, via `python -m <worker_module>`).
    pub worker_module: String,
    /// YuNet detection confidence threshold in `[0, 1]`.
    pub detection_threshold: f32,
    /// Maximum image dimension after downscaling (0 disables downscaling).
    pub max_image_dimension: u32,
    /// Timeout (seconds) for a single analysis request.
    pub timeout_seconds: u64,
}

impl Default for OpenCvAnalysisConfig {
    fn default() -> Self {
        Self {
            models_dir: PathBuf::from("./models"),
            python_package_dir: PathBuf::from("./py"),
            python: "python3".to_string(),
            worker_module: "immutara_cv.face_worker".to_string(),
            detection_threshold: 0.9,
            max_image_dimension: 4096,
            timeout_seconds: 30,
        }
    }
}

impl Config {
    /// Load configuration from a TOML file path.
    pub fn from_path(path: &str) -> Result<Self, ImmutaraError> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| ImmutaraError::Config(format!("failed to read {path}: {e}")))?;
        let config: Config = toml::from_str(&raw)
            .map_err(|e| ImmutaraError::Config(format!("failed to parse {path}: {e}")))?;
        Ok(config)
    }
}
