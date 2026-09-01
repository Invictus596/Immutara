//! Attestation provider trait.

use async_trait::async_trait;

use crate::ImmutaraError;
use crate::domain::attestation::{AttestationReceipt, AttestationRecord};

/// Contract for writing attestation records to an immutable ledger.
///
/// The blockchain is an integrity/attestation layer only. Providers receive
/// an already-canonical, hashable `AttestationRecord` and must commit only
/// that record's bytes (which contain hashes, never raw media/biometrics).
#[async_trait]
pub trait AttestationProvider: Send + Sync {
    /// Submit an attestation record to the ledger.
    async fn attest(&self, record: &AttestationRecord)
    -> Result<AttestationReceipt, ImmutaraError>;

    /// Stable identifier for this provider.
    fn provider_id(&self) -> &str;

    /// The chain identifier this provider targets (e.g. an EVM chain id).
    fn chain_id(&self) -> &str;
}
