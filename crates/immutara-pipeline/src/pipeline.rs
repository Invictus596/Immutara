//! The pipeline orchestrator.
//!
//! Owns a single authoritative `PipelineEvent` stream published through a
//! `tokio::sync::mpsc::Sender`. It wires the individual stages together and
//! runs them for a given piece of evidence, emitting an event for each
//! transition. It holds no rendering concerns.

use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use immutara_core::CanonicalSerialize;
use immutara_core::CanonicalSerializeForHashing;
use immutara_core::ImmutaraError;
use immutara_core::domain::analysis::AnalysisResult;
use immutara_core::domain::attestation::{AttestationRecord, BlockchainVerification};
use immutara_core::domain::evidence::{Evidence, EvidenceId, SchemaVersion};
use immutara_core::domain::search::SearchResult;
use immutara_core::domain::verification::{VerificationPolicy, VerificationResult};
use immutara_core::providers::{AnalysisProvider, AttestationProvider, ImageSearchProvider};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::event::PipelineEvent;
use crate::hashing::hash_canonical;
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
}

impl Pipeline {
    /// Create a pipeline from the three provider implementations.
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
        }
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
        let search = self.run_search(&evidence).await;

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
        self.run_attest(&evidence, &verification, &input.policy)
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

    async fn run_search(&self, evidence: &Evidence) -> Option<SearchResult> {
        self.emit(PipelineEvent::SearchStarted {
            evidence_id: evidence.id,
            provider_id: self.search.provider_id().to_string(),
        })
        .await;
        match self.search.search(evidence).await {
            Ok(result) => {
                self.emit(PipelineEvent::SearchCompleted {
                    evidence_id: evidence.id,
                    result: result.clone(),
                })
                .await;
                Some(result)
            }
            Err(e) => {
                self.emit(PipelineEvent::SearchFailed {
                    evidence_id: evidence.id,
                    provider_id: self.search.provider_id().to_string(),
                    error: e.to_string(),
                })
                .await;
                None
            }
        }
    }

    async fn run_attest(
        &self,
        evidence: &Evidence,
        verification: &VerificationResult,
        policy: &VerificationPolicy,
    ) -> Result<(), ImmutaraError> {
        self.emit(PipelineEvent::AttestationStarted {
            evidence_id: evidence.id,
        })
        .await;

        let metadata_hash = hash_canonical(&CanonicalSerializeForHashing(&evidence.metadata))?;
        let verification_hash = hash_canonical(&CanonicalSerializeForHashing(verification))?;

        let record = AttestationRecord {
            schema_version: SchemaVersion(1),
            pipeline_version: self.pipeline_version.clone(),
            evidence_id: evidence.id,
            content_hash: evidence.content_hash.clone(),
            metadata_hash,
            verification_result_hash: verification_hash,
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
