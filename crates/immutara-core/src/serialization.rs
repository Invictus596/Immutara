//! Deterministic (canonical) serialization.
//!
//! The exact bytes produced here are what get hashed for on-chain
//! attestation and for content/metadata hashes. To ensure reproducibility,
//! the serialization must be stable across processes and versions:
//!
//! - Object keys are sorted alphabetically at every nesting level.
//! - The output is compact JSON (no insignificant whitespace).
//! - Numbers, strings, etc. are serialized by `serde_json` in their
//!   canonical forms.
//!
//! Any type that participates in hashing/attestation should implement
//! `CanonicalSerialize`.

use serde::Serialize;
use serde_json::{Map, Value};

use crate::errors::ImmutaraError;

/// A value that can be serialized to deterministic bytes.
pub trait CanonicalSerialize {
    /// Produce deterministic bytes suitable for hashing.
    fn canonical_bytes(&self) -> Result<Vec<u8>, ImmutaraError>;
}

impl CanonicalSerialize for Value {
    fn canonical_bytes(&self) -> Result<Vec<u8>, ImmutaraError> {
        let sorted = canonical_sort(self.clone());
        let bytes = serde_json::to_vec(&sorted)?;
        Ok(bytes)
    }
}

impl CanonicalSerialize for crate::domain::attestation::AttestationRecord {
    fn canonical_bytes(&self) -> Result<Vec<u8>, ImmutaraError> {
        serde_json::to_value(self)?.canonical_bytes()
    }
}

/// Convenience adapter to canonicalize any `Serialize` type.
pub struct CanonicalSerializeForHashing<T>(pub T);

impl<T: Serialize> CanonicalSerialize for CanonicalSerializeForHashing<T> {
    fn canonical_bytes(&self) -> Result<Vec<u8>, ImmutaraError> {
        let value = serde_json::to_value(&self.0)?;
        value.canonical_bytes()
    }
}

/// Recursively sort object keys so serialization is order-independent.
///
/// Object keys are sorted; array order and scalar values are preserved.
fn canonical_sort(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sorted = Map::with_capacity(map.len());
            let mut keys: Vec<_> = map.keys().cloned().collect();
            keys.sort();
            for key in keys {
                let inner = map.get(&key).cloned().unwrap_or(Value::Null);
                sorted.insert(key, canonical_sort(inner));
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(canonical_sort).collect()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::evidence::SchemaVersion;
    use std::collections::HashMap;

    #[test]
    fn canonical_bytes_are_order_independent_for_object_keys() {
        let mut a = Map::new();
        a.insert("z".to_string(), Value::from(1));
        a.insert("a".to_string(), Value::from(2));
        let json_a = Value::Object(a);

        let mut b = Map::new();
        b.insert("a".to_string(), Value::from(2));
        b.insert("z".to_string(), Value::from(1));
        let json_b = Value::Object(b);

        assert_eq!(
            json_a.canonical_bytes().unwrap(),
            json_b.canonical_bytes().unwrap()
        );
    }

    #[test]
    fn canonical_bytes_are_stable_across_calls() {
        let value = serde_json::json!({
            "b": [3, 1, 2],
            "a": {"nested": true, "x": null}
        });
        assert_eq!(
            value.canonical_bytes().unwrap(),
            value.canonical_bytes().unwrap()
        );
    }

    #[test]
    fn canonical_bytes_are_deterministic_content() {
        let value = serde_json::json!({"b": 2, "a": 1});
        assert_eq!(
            std::str::from_utf8(&value.canonical_bytes().unwrap()).unwrap(),
            r#"{"a":1,"b":2}"#
        );
    }

    #[test]
    fn schema_version_serializes_canonically() {
        let v = CanonicalSerializeForHashing(SchemaVersion(3));
        assert_eq!(v.canonical_bytes().unwrap(), b"3");
    }

    #[test]
    fn nested_maps_are_sorted() {
        let mut outer = Map::new();
        outer.insert("y".to_string(), Value::from(1));

        let mut inner = Map::new();
        inner.insert("b".to_string(), Value::from(2));
        inner.insert("a".to_string(), Value::from(3));
        inner.insert("outer_key".to_string(), Value::Object(outer));
        let json = Value::Object(inner);

        let bytes = json.canonical_bytes().unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert_eq!(text, r#"{"a":3,"b":2,"outer_key":{"y":1}}"#);
    }

    #[test]
    fn hashmap_canonicalization_is_order_independent() {
        let mut hm1 = HashMap::new();
        hm1.insert("key1".to_string(), "value1".to_string());
        hm1.insert("key2".to_string(), "value2".to_string());

        let mut hm2 = HashMap::new();
        hm2.insert("key2".to_string(), "value2".to_string());
        hm2.insert("key1".to_string(), "value1".to_string());

        assert_eq!(
            CanonicalSerializeForHashing(&hm1)
                .canonical_bytes()
                .unwrap(),
            CanonicalSerializeForHashing(&hm2)
                .canonical_bytes()
                .unwrap()
        );
    }
}
