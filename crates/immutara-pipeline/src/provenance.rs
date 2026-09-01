//! Construction helpers for provenance chain entries.

use std::collections::HashMap;

use chrono::Utc;
use immutara_core::domain::evidence::ContentHash;
use immutara_core::domain::provenance::{Operation, Operator, ProvenanceChain, ProvenanceEntry};
use serde_json::Value;

/// The operator recorded for pipeline-generated provenance entries.
pub fn pipeline_operator() -> Operator {
    Operator {
        name: "immutara-pipeline".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

/// Build a provenance entry from its parts.
///
/// If `metadata` is empty an empty map is stored, keeping the entry shape
/// deterministic.
pub fn build_entry(
    operation: Operation,
    operator: Operator,
    input_hash: ContentHash,
    output_hash: ContentHash,
    metadata: HashMap<String, Value>,
) -> ProvenanceEntry {
    ProvenanceEntry {
        timestamp: Utc::now(),
        operation,
        operator,
        input_hash,
        output_hash,
        metadata,
    }
}

/// Append an entry to the chain and return the chain.
pub fn push_entry(chain: &mut ProvenanceChain, entry: ProvenanceEntry) {
    chain.push(entry);
}

#[cfg(test)]
mod tests {
    use super::*;
    use immutara_core::domain::evidence::ContentHash;
    use std::collections::HashMap;

    #[test]
    fn build_entry_records_operation_and_operator() {
        let entry = build_entry(
            Operation::Ingest,
            pipeline_operator(),
            ContentHash("in".to_string()),
            ContentHash("out".to_string()),
            HashMap::new(),
        );
        assert_eq!(entry.operation, Operation::Ingest);
        assert_eq!(entry.operator.name, "immutara-pipeline");
        assert_eq!(entry.input_hash, ContentHash("in".to_string()));
        assert_eq!(entry.output_hash, ContentHash("out".to_string()));
        assert!(entry.metadata.is_empty());
    }

    #[test]
    fn push_entry_appends() {
        let mut chain = ProvenanceChain::new();
        push_entry(
            &mut chain,
            build_entry(
                Operation::Search {
                    provider_id: "mock".to_string(),
                },
                pipeline_operator(),
                ContentHash("i".to_string()),
                ContentHash("o".to_string()),
                HashMap::new(),
            ),
        );
        assert_eq!(chain.len(), 1);
    }
}
