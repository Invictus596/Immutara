//! The authoritative event stream contract shared between the pipeline
//! (producer) and the TUI (consumer).
//!
//! Living in `immutara-core` keeps `immutara-pipeline` and `immutara-tui`
//! decoupled: both depend only on core, and neither depends on the other.

use super::analysis::AnalysisResult;
use super::attestation::{AttestationReceipt, AttestationRecord};
use super::evidence::{ContentHash, EvidenceId, EvidenceMetadata};
use super::search::SearchResult;
use super::verification::VerificationResult;

/// An event describing a transition in the pipeline lifecycle.
#[derive(Debug, Clone, PartialEq)]
pub enum PipelineEvent {
    /// Pipeline run started for a piece of evidence.
    PipelineStarted {
        evidence_id: EvidenceId,
        metadata: EvidenceMetadata,
    },
    /// Pipeline run completed successfully.
    PipelineCompleted {
        evidence_id: EvidenceId,
    },

    /// Evidence was successfully ingested.
    EvidenceIngested {
        evidence_id: EvidenceId,
        content_hash: ContentHash,
        metadata: EvidenceMetadata,
    },

    // ---- Analysis stage ----
    AnalysisStarted {
        evidence_id: EvidenceId,
        provider_id: String,
    },
    AnalysisCompleted {
        evidence_id: EvidenceId,
        result: AnalysisResult,
    },
    AnalysisFailed {
        evidence_id: EvidenceId,
        provider_id: String,
        error: String,
    },

    // ---- Search stage ----
    SearchStarted {
        evidence_id: EvidenceId,
        provider_id: String,
    },
    SearchCompleted {
        evidence_id: EvidenceId,
        result: SearchResult,
    },
    SearchFailed {
        evidence_id: EvidenceId,
        provider_id: String,
        error: String,
    },

    // ---- Verification stage ----
    VerificationStarted {
        evidence_id: EvidenceId,
    },
    VerificationCompleted {
        evidence_id: EvidenceId,
        result: VerificationResult,
    },

    // ---- Attestation stage ----
    AttestationStarted {
        evidence_id: EvidenceId,
    },
    AttestationCompleted {
        evidence_id: EvidenceId,
        record: AttestationRecord,
        receipt: AttestationReceipt,
    },
    AttestationFailed {
        evidence_id: EvidenceId,
        error: String,
    },

    /// An unrecoverable failure occurred during the pipeline run.
    PipelineFailed {
        evidence_id: EvidenceId,
        error: String,
    },
}
