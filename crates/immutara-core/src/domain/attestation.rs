//! Attestation domain types.
//!
//! The blockchain is treated strictly as an integrity/attestation layer.
//! Only content hashes and metadata hashes are recorded — never raw images
//! or biometric data.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::evidence::{ContentHash, EvidenceId, SchemaVersion};

/// The canonical, hashable record of an on-chain attestation.
///
/// This type is serialized deterministically (see `CanonicalSerialize`)
/// and its bytes are what gets committed on-chain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttestationRecord {
    pub schema_version: SchemaVersion,
    pub pipeline_version: String,
    pub evidence_id: EvidenceId,
    pub content_hash: ContentHash,
    pub metadata_hash: ContentHash,
    pub verification_result_hash: ContentHash,
    pub verification_policy_version: SchemaVersion,
    pub provider_id: String,
    pub chain_id: String,
    pub tx_hash: Option<String>,
    pub block_number: Option<u64>,
    pub attested_at: DateTime<Utc>,
}

/// The confirmation returned by an attestation provider after submission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttestationReceipt {
    pub tx_hash: String,
    pub block_number: u64,
    pub chain_id: String,
}
