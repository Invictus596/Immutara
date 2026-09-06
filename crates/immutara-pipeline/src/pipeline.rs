//! The pipeline orchestrator.
//!
//! Owns a single authoritative `PipelineEvent` stream published through a
//! `tokio::sync::mpsc::Sender`. It wires the individual stages together and
//! runs them for a given piece of evidence, emitting an event for each
//! transition. It holds no rendering concerns.
//!
//! The Search stage runs FACE CROP first when a face is available and falls
//! back to a FULL IMAGE search (emitting `SearchFallback`) when a media
//! validating provider's crop result did not reach `SOCIAL_MATCH_VERIFIED`.

use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use immutara_core::CanonicalSerialize;
use immutara_core::CanonicalSerializeForHashing;
use immutara_core::ImmutaraError;
use immutara_core::domain::analysis::AnalysisResult;
use immutara_core::domain::attestation::{AttestationRecord, BlockchainVerification};
use immutara_core::domain::evidence::{Evidence, EvidenceId, SchemaVersion};
use immutara_core::domain::search::{SearchInput, SearchInputKind, SearchMatchState, SearchResult};
use immutara_core::domain::verification::{VerificationPolicy, VerificationResult};
use immutara_core::providers::{AnalysisProvider, AttestationProvider, ImageSearchProvider};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::event::PipelineEvent;
use crate::face_crop;
use crate::hashing::hash_canonical;
use crate::media_validation::{MediaMatchValidator, ValidationQuery};
use crate::stages::{self, ingest::IngestInput};

/// Input describing a single evidence item to process.
pub struct PipelineInput {
    /// Raw bytes of the evidence file.
    pub raw_bytes: Vec<u8>,
    /// MIME type of the evidence file.
    pub mime_type: String,
    /// Size of the evidence file in bytes.
    pub file_size: u64,
    /// Optional on-disk path of the evidence file. When present it is recorded
    /// in `EvidenceMetadata.source_path`, which real providers (e.g. the
    /// OpenCV worker) use to locate the image.
    pub source_path: Option<PathBuf>,
    /// The verification policy to apply.
    pub policy: VerificationPolicy,
}

/// A fully-wired pipeline ready to process evidence.
pub struct Pipeline {
    /// Send half of the event channel; the TUI holds the receiver.
    event_tx: mpsc::Sender<PipelineEvent>,
    analysis: Arc<dyn AnalysisProvider>,
    search: Arc<dyn ImageSearchProvider>,
    attestation: Arc<dyn AttestationProvider>,
    pipeline_version: String,
    media_validator: Option<MediaMatchValidator>,
}

impl Pipeline {
    /// Create a pipeline from the three provider implementations.
    ///
    /// Media validation is off unless enabled with
    /// [`Self::with_media_validator`].
    pub fn new(
        event_tx: mpsc::Sender<PipelineEvent>,
        analysis: Arc<dyn AnalysisProvider>,
        search: Arc<dyn ImageSearchProvider>,
        attestation: Arc<dyn AttestationProvider>,
    ) -> Self {
        Self {
            event_tx,
            analysis,
            search,
            attestation,
            pipeline_version: env!("CARGO_PKG_VERSION").to_string(),
            media_validator: None,
        }
    }

    /// Enable independent media-match validation for providers that opt in.
    pub fn with_media_validator(mut self, validator: Option<MediaMatchValidator>) -> Self {
        self.media_validator = validator;
        self
    }

    async fn emit(&self, event: PipelineEvent) {
        // If the receiver has dropped, the TUI is gone; ignore.
        let _ = self.event_tx.send(event).await;
    }

    /// Process a single evidence item through all stages.
    ///
    /// Emits an event for each transition. Analysis and search failures are
    /// recorded as events but are non-fatal (they simply contribute nothing
    /// to verification). Attestation failures are fatal to the run.
    pub async fn process(&self, input: PipelineInput) -> Result<(), ImmutaraError> {
        // ---- Ingest (sync) ----
        let evidence = match stages::ingest::ingest(IngestInput {
            raw_bytes: input.raw_bytes,
            mime_type: input.mime_type,
            file_size: input.file_size,
            source_path: input.source_path,
        }) {
            Ok(out) => out.evidence,
            Err(e) => {
                let id = EvidenceId(Uuid::nil());
                self.emit(PipelineEvent::PipelineFailed {
                    evidence_id: id,
                    error: e.to_string(),
                })
                .await;
                return Err(e);
            }
        };

        self.emit(PipelineEvent::PipelineStarted {
            evidence_id: evidence.id,
            metadata: evidence.metadata.clone(),
        })
        .await;
        self.emit(PipelineEvent::EvidenceIngested {
            evidence_id: evidence.id,
            content_hash: evidence.content_hash.clone(),
            metadata: evidence.metadata.clone(),
        })
        .await;

        // ---- Analyze (async I/O) ----
        let analysis = self.run_analysis(&evidence).await;

        // ---- Search (async I/O) ----
        let search = self.run_search(&evidence, analysis.as_ref()).await;

        // ---- Verify (sync, deterministic) ----
        self.emit(PipelineEvent::VerificationStarted {
            evidence_id: evidence.id,
        })
        .await;
        let verification = match stages::verify::verify(stages::verify::VerifyInput {
            evidence: &evidence,
            analysis: analysis.as_ref(),
            search: search.as_ref(),
            policy: &input.policy,
        }) {
            Ok(result) => result,
            Err(e) => {
                self.emit(PipelineEvent::PipelineFailed {
                    evidence_id: evidence.id,
                    error: e.to_string(),
                })
                .await;
                return Err(e);
            }
        };
        self.emit(PipelineEvent::VerificationCompleted {
            evidence_id: evidence.id,
            result: verification.clone(),
        })
        .await;

        // ---- Attest (async I/O) ----
        self.run_attest(&evidence, &verification, search.as_ref(), &input.policy)
            .await?;

        self.emit(PipelineEvent::PipelineCompleted {
            evidence_id: evidence.id,
        })
        .await;
        Ok(())
    }

    async fn run_analysis(&self, evidence: &Evidence) -> Option<AnalysisResult> {
        self.emit(PipelineEvent::AnalysisStarted {
            evidence_id: evidence.id,
            provider_id: self.analysis.provider_id().to_string(),
        })
        .await;
        match self.analysis.analyze(evidence).await {
            Ok(result) => {
                self.emit(PipelineEvent::AnalysisCompleted {
                    evidence_id: evidence.id,
                    result: result.clone(),
                })
                .await;
                Some(result)
            }
            Err(e) => {
                self.emit(PipelineEvent::AnalysisFailed {
                    evidence_id: evidence.id,
                    provider_id: self.analysis.provider_id().to_string(),
                    error: e.to_string(),
                })
                .await;
                None
            }
        }
    }

    async fn run_search(
        &self,
        evidence: &Evidence,
        analysis: Option<&AnalysisResult>,
    ) -> Option<SearchResult> {
        self.emit(PipelineEvent::SearchStarted {
            evidence_id: evidence.id,
            provider_id: self.search.provider_id().to_string(),
        })
        .await;

        // FACE CROP first: with a detected face we search the local, padded,
        // re-encoded crop because the face is the discriminative signal when
        // looking for a public repost of someone's photo.
        let input = self.build_search_input(evidence, analysis);
        let crop = match self.search_once(evidence, &input).await {
            Ok(result) => {
                self.emit(PipelineEvent::SearchCompleted {
                    evidence_id: evidence.id,
                    result: result.clone(),
                })
                .await;
                result
            }
            Err(e) => {
                // The crop search itself failed; nothing was learned, so no
                // other input is attempted — report the failure honestly.
                self.emit(PipelineEvent::SearchFailed {
                    evidence_id: evidence.id,
                    provider_id: self.search.provider_id().to_string(),
                    error: e.to_string(),
                })
                .await;
                return None;
            }
        };

        if !self.should_fallback_to_full(&crop) {
            return Some(crop);
        }

        // The crop did not reach SOCIAL_MATCH_VERIFIED (zero matches, or
        // candidates that failed perceptual media validation). Fall back to a
        // FULL IMAGE search before conceding: the content-based signal often
        // matches where the isolated face cannot.
        self.emit(PipelineEvent::SearchFallback {
            evidence_id: evidence.id,
            attempted_input: crop.search_input,
            attempted_state: crop.social_state,
            reason: if crop.matches.is_empty() {
                "the face crop yielded no search matches".to_string()
            } else {
                "the face crop yielded candidates but none reached \
                 SOCIAL_MATCH_VERIFIED"
                    .to_string()
            },
        })
        .await;
        self.emit(PipelineEvent::SearchStarted {
            evidence_id: evidence.id,
            provider_id: self.search.provider_id().to_string(),
        })
        .await;

        match self.search_once(evidence, &SearchInput::FullImage).await {
            Ok(full) => {
                self.emit(PipelineEvent::SearchCompleted {
                    evidence_id: evidence.id,
                    result: full.clone(),
                })
                .await;
                Some(full)
            }
            Err(e) => {
                // The full-image fallback failed; the honest outcome is the
                // already-reported crop result, not a fabricated score.
                self.emit(PipelineEvent::SearchFailed {
                    evidence_id: evidence.id,
                    provider_id: self.search.provider_id().to_string(),
                    error: e.to_string(),
                })
                .await;
                Some(crop)
            }
        }
    }

    /// Whether the pipeline should retry the Search stage with the FULL IMAGE
    /// after a FACE CROP search. Only providers that opt into media validation
    /// participate: for them `social_state` reflects perceptual validation, and
    /// anything short of `SOCIAL_MATCH_VERIFIED` does not justify attestation —
    /// but it also should not stop the pipeline from trying the full image.
    fn should_fallback_to_full(&self, crop: &SearchResult) -> bool {
        self.search.supports_media_validation()
            && crop.search_input == SearchInputKind::FaceCrop
            && crop.social_state != SearchMatchState::SocialMatchVerified
    }

    /// Run a single provider search for `input` and, when the provider and
    /// validator support it, run independent media validation on the matches.
    /// Event emission is the caller's responsibility.
    async fn search_once(
        &self,
        evidence: &Evidence,
        input: &SearchInput,
    ) -> Result<SearchResult, ImmutaraError> {
        let mut result = self.search.search_with_input(evidence, input).await?;
        if self.search.supports_media_validation()
            && let Some(validator) = &self.media_validator
            && let Some(photo) = Self::photo_bytes(evidence)
        {
            let query = ValidationQuery {
                searched: Self::query_bytes(input, evidence).unwrap_or_else(|| photo.clone()),
                photo,
            };
            validator.validate_matches(&mut result, &query).await;
        }
        Ok(result)
    }

    /// Build the search input: prefer the selected face crop (local, padded,
    /// re-encoded) and fall back to the full image when no face was detected
    /// or the crop cannot be generated.
    fn build_search_input(
        &self,
        evidence: &Evidence,
        analysis: Option<&AnalysisResult>,
    ) -> SearchInput {
        let Some(selected_face) = analysis
            .and_then(|a| a.face_analysis.as_ref())
            .and_then(|f| f.selected_face.as_ref())
        else {
            return SearchInput::FullImage;
        };
        let Some(source_path) = &evidence.metadata.source_path else {
            return SearchInput::FullImage;
        };

        match std::fs::read(source_path) {
            Ok(bytes) => {
                match face_crop::generate_face_crop(&bytes, selected_face.bounding_box, 500_000) {
                    Ok(crop) => SearchInput::FaceCrop(crop),
                    Err(_) => SearchInput::FullImage,
                }
            }
            Err(_) => SearchInput::FullImage,
        }
    }

    /// The exact image bytes that were searched: the face-crop bytes when a
    /// crop was submitted, otherwise the full image on disk. Used as the query
    /// for media-match validation.
    fn query_bytes(input: &SearchInput, evidence: &Evidence) -> Option<Vec<u8>> {
        match input {
            SearchInput::FaceCrop(crop) => Some(crop.image_bytes.clone()),
            SearchInput::FullImage => {
                let source_path = evidence.metadata.source_path.as_ref()?;
                std::fs::read(source_path).ok()
            }
        }
    }

    /// The full evidence photograph bytes on disk (the validation baseline for
    /// perceptual media comparison when a face crop was searched).
    fn photo_bytes(evidence: &Evidence) -> Option<Vec<u8>> {
        let source_path = evidence.metadata.source_path.as_ref()?;
        std::fs::read(source_path).ok()
    }

    async fn run_attest(
        &self,
        evidence: &Evidence,
        verification: &VerificationResult,
        search: Option<&SearchResult>,
        policy: &VerificationPolicy,
    ) -> Result<(), ImmutaraError> {
        self.emit(PipelineEvent::AttestationStarted {
            evidence_id: evidence.id,
        })
        .await;

        let metadata_hash = hash_canonical(&CanonicalSerializeForHashing(&evidence.metadata))?;
        let verification_hash = hash_canonical(&CanonicalSerializeForHashing(verification))?;
        // Bind the single most-relevant discovered result into the record via
        // its canonical hash: the first exact Lens match when one exists,
        // otherwise the first visual match (deterministic; `None` when no
        // result exists).
        let selected_result = search.and_then(|s| s.selected());
        let search_result_hash = crate::hashing::search_result_hash(selected_result)?;

        let record = AttestationRecord {
            schema_version: SchemaVersion(1),
            pipeline_version: self.pipeline_version.clone(),
            evidence_id: evidence.id,
            content_hash: evidence.content_hash.clone(),
            metadata_hash,
            verification_result_hash: verification_hash,
            search_result_hash,
            verification_policy_version: policy.version,
            provider_id: self.attestation.provider_id().to_string(),
            chain_id: self.attestation.chain_id().to_string(),
            attested_at: Utc::now(),
        };

        // Canonicalize before submission so the exact bytes hashed are
        // reproducible. Only hashes travel to the ledger, never raw media
        // or biometric data.
        let _canonical_bytes = record.canonical_bytes()?;

        match self.attestation.attest(&record).await {
            Ok(receipt) => {
                // AttestationCompleted always carries the receipt so the
                // consumer has the full delivery + verification picture. When
                // the on-chain read-back disagrees with the local hash the
                // receipt reports `Failed` and the run is still fatal — a
                // successful transaction must never be presented as verified.
                self.emit(PipelineEvent::AttestationCompleted {
                    evidence_id: evidence.id,
                    record: record.clone(),
                    receipt: receipt.clone(),
                })
                .await;
                if receipt.blockchain_verification == BlockchainVerification::Failed {
                    return Err(ImmutaraError::Provider {
                        provider: self.attestation.provider_id().to_string(),
                        message: format!(
                            "on-chain re-verification FAILED for anchor {}: \
                             the hash stored on-chain does not match the locally \
                             recomputed record hash (evidence {})",
                            receipt.attestation_id, evidence.id
                        ),
                    });
                }
                Ok(())
            }
            Err(e) => {
                self.emit(PipelineEvent::AttestationFailed {
                    evidence_id: evidence.id,
                    error: e.to_string(),
                })
                .await;
                Err(e)
            }
        }
    }
}
