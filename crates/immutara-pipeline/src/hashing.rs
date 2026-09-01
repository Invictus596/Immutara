//! Cryptographic and content hashing utilities.

use immutara_core::ImmutaraError;
use immutara_core::domain::evidence::ContentHash;
use sha2::{Digest, Sha256};

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
