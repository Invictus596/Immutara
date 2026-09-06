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
/// and its bytes are what gets committed on-chain. It holds only facts that
/// exist **before** submission; delivery details (`tx_hash`, `block_number`)
/// live on the [`AttestationReceipt`], never in the record. That keeps the
/// record stable so the exact bytes hashed locally can be recomputed and
/// re-verified against the chain at any later time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttestationRecord {
    pub schema_version: SchemaVersion,
    pub pipeline_version: String,
    pub evidence_id: EvidenceId,
    pub content_hash: ContentHash,
    pub metadata_hash: ContentHash,
    pub verification_result_hash: ContentHash,
    /// Canonical hash of the single most-relevant discovered result. This
    /// cryptographically binds the concrete public source/social-media URL
    /// found by the search stage into the on-chain record, so the anchor
    /// commits to what the search actually discovered at runtime.
    pub search_result_hash: ContentHash,
    pub verification_policy_version: SchemaVersion,
    pub provider_id: String,
    pub chain_id: String,
    pub attested_at: DateTime<Utc>,
}

/// Outcome of the on-chain read-back re-verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockchainVerification {
    /// The hash read back from the chain equals the locally recomputed
    /// hash of the canonical record.
    Verified,
    /// The chain returned something different from the locally recomputed
    /// hash (or the slot was never set). The anchor must NOT be trusted.
    Failed,
}

/// The confirmation returned by an attestation provider after submission.
///
/// Carries both the delivery metadata (transaction, block, contract) and the
/// on-chain read-back used to re-verify the anchor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttestationReceipt {
    pub tx_hash: String,
    pub block_number: u64,
    pub chain_id: String,
    pub contract_address: String,
    pub attestation_id: String,
    pub on_chain_record_hash: String,
    pub blockchain_verification: BlockchainVerification,
}
