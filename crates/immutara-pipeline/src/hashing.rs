//! Cryptographic and content hashing utilities.
//!
//! Attestation anchoring reuses the canonical serialization from
//! `immutara-core` — there is deliberately no second, independent
//! canonicalization here.

use alloy::primitives::B256;
use immutara_core::CanonicalSerialize;
use immutara_core::ImmutaraError;
use immutara_core::domain::attestation::AttestationRecord;
use immutara_core::domain::evidence::ContentHash;
use sha2::{Digest, Sha256};

/// Stable domain-separation context for attestation ids.
///
/// Attestation ids must be deterministic across runs: the same content hash
/// always anchors to the same on-chain slot, which is what makes
/// re-verification meaningful.
const ATTESTATION_ID_CONTEXT: &[u8] = b"immutara-attestation:v1";

/// Compute the hex-encoded SHA-256 digest of a byte slice.
pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Compute a `ContentHash` from raw bytes.
pub fn content_hash(data: &[u8]) -> ContentHash {
    ContentHash(sha256_hex(data))
}

/// Compute a `ContentHash` from canonical serialized bytes.
pub fn hash_canonical<T: immutara_core::CanonicalSerialize>(
    value: &T,
) -> Result<ContentHash, ImmutaraError> {
    Ok(content_hash(&value.canonical_bytes()?))
}

/// Compute the raw 32-byte SHA-256 digest of a byte slice.
pub fn sha256_bytes(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

/// Hash of the canonical attestation record bytes.
///
/// `record_hash = SHA-256(canonical_bytes(AttestationRecord))`. This is the
/// exact value submitted to and read back from the `AttestationRegistry`
/// contract, so the two can be compared for re-verification.
pub fn record_hash(record: &AttestationRecord) -> Result<B256, ImmutaraError> {
    Ok(B256::from(sha256_bytes(&record.canonical_bytes()?)))
}

/// Deterministically derive the on-chain attestation id for a content hash.
///
/// `attestation_id = SHA-256(content_hash_bytes || "immutara-attestation:v1")`
/// truncated to 32 bytes. Deterministic and auditable; never a random UUID.
pub fn derive_attestation_id(content_hash: &ContentHash) -> B256 {
    let mut hasher = Sha256::new();
    hasher.update(content_hash.0.as_bytes());
    hasher.update(ATTESTATION_ID_CONTEXT);
    let mut digest = [0u8; 32];
    digest.copy_from_slice(&hasher.finalize());
    B256::from(digest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use immutara_core::domain::evidence::ContentHash;

    #[test]
    fn sha256_matches_known_vector() {
        // SHA-256 of the empty string.
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_of_abc_is_known() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn content_hash_is_hex_encoded() {
        let hash = content_hash(b"immutara");
        assert_eq!(hash.0.len(), 64);
        assert!(hash.0.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn same_bytes_deterministic_hash() {
        assert_eq!(content_hash(b"data"), ContentHash(sha256_hex(b"data")));
    }

    #[test]
    fn hash_canonical_is_deterministic() {
        let a = immutara_core::serialization::CanonicalSerializeForHashing(
            serde_json::json!({"b":1,"a":2}),
        );
        let b = immutara_core::serialization::CanonicalSerializeForHashing(
            serde_json::json!({"a":2,"b":1}),
        );
        assert_eq!(hash_canonical(&a).unwrap(), hash_canonical(&b).unwrap());
    }
}
