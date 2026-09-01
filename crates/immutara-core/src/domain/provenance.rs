//! Provenance chain domain types.
//!
//! Every transformation and external operation performed on evidence is
//! recorded as a `ProvenanceEntry`. Entries are linked by input/output
//! content hashes, making the chain auditable and tamper-evident.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::evidence::{ContentHash, SchemaVersion};

/// The kind of operation recorded in a provenance entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operation {
    Ingest,
    Normalize,
    Analyze {
        provider_id: String,
    },
    Search {
        provider_id: String,
    },
    Verify {
        policy_version: SchemaVersion,
    },
    Attest {
        provider_id: String,
        chain_id: String,
    },
}

/// The actor that performed an operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Operator {
    pub name: String,
    pub version: String,
}

/// A single entry in the provenance chain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProvenanceEntry {
    pub timestamp: DateTime<Utc>,
    pub operation: Operation,
    pub operator: Operator,
    pub input_hash: ContentHash,
    pub output_hash: ContentHash,
    pub metadata: HashMap<String, Value>,
}

/// An ordered, append-only chain of provenance entries.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProvenanceChain {
    pub entries: Vec<ProvenanceEntry>,
}

impl ProvenanceChain {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Append an entry to the chain.
    pub fn push(&mut self, entry: ProvenanceEntry) {
        self.entries.push(entry);
    }

    /// Number of entries in the chain.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True if the chain has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::evidence::ContentHash;

    fn sample_entry() -> ProvenanceEntry {
        ProvenanceEntry {
            timestamp: Utc::now(),
            operation: Operation::Ingest,
            operator: Operator {
                name: "test".to_string(),
                version: "0.1.0".to_string(),
            },
            input_hash: ContentHash("in".to_string()),
            output_hash: ContentHash("out".to_string()),
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn chain_starts_empty() {
        let chain = ProvenanceChain::new();
        assert!(chain.is_empty());
        assert_eq!(chain.len(), 0);
    }

    #[test]
    fn entries_are_appended_in_order() {
        let mut chain = ProvenanceChain::new();
        chain.push(sample_entry());
        chain.push(sample_entry());
        assert_eq!(chain.len(), 2);
        assert!(!chain.is_empty());
    }

    #[test]
    fn chain_preserves_entry_hashes() {
        let mut chain = ProvenanceChain::new();
        let entry = ProvenanceEntry {
            operation: Operation::Ingest,
            ..sample_entry()
        };
        chain.push(entry);
        let first = &chain.entries[0];
        assert_eq!(first.operation, Operation::Ingest);
        assert_eq!(first.input_hash, ContentHash("in".to_string()));
        assert_eq!(first.output_hash, ContentHash("out".to_string()));
    }
}
