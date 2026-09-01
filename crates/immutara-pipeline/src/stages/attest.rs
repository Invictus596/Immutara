//! Attestation stage: canonicalize and submit an `AttestationRecord`.

use immutara_core::ImmutaraError;
use immutara_core::domain::attestation::{AttestationReceipt, AttestationRecord};
use immutara_core::providers::AttestationProvider;

/// Submit an attestation record via a provider.
///
/// Canonical serialization is the caller's responsibility (the pipeline
/// hashes the canonical bytes before/at submission). Async because provider
/// submission performs I/O (chain RPC).
pub async fn run_attest<T: AttestationProvider + ?Sized>(
    provider: &T,
    record: &AttestationRecord,
) -> Result<AttestationReceipt, ImmutaraError> {
    provider.attest(record).await
}
