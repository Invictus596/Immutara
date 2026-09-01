//! Ingest stage: turns raw evidence bytes into `Evidence`.
//!
//! Image normalization and EXIF extraction are NOT implemented yet; this is
//! a structural skeleton. The stage only computes a content hash from raw
//! bytes and records an `Ingest` provenance entry.

use chrono::Utc;
use immutara_core::ImmutaraError;
use immutara_core::domain::evidence::{Evidence, EvidenceId, EvidenceMetadata, SchemaVersion};
use immutara_core::domain::provenance::Operation;

use crate::hashing::content_hash;
use crate::provenance::{build_entry, pipeline_operator};

/// Input for the ingest stage.
pub struct IngestInput {
    pub raw_bytes: Vec<u8>,
    pub mime_type: String,
    pub file_size: u64,
}

/// Output of the ingest stage.
pub struct IngestOutput {
    pub evidence: Evidence,
}

/// Skeletal ingest stage.
///
/// TODO: image normalization and EXIF extraction land here later.
pub fn ingest(input: IngestInput) -> Result<IngestOutput, ImmutaraError> {
    let id = EvidenceId::new();
    let content_hash = content_hash(&input.raw_bytes);

    let metadata = EvidenceMetadata {
        source_path: None,
        mime_type: input.mime_type,
        file_size: input.file_size,
        dimensions: None,
        captured_at: None,
        schema_version: SchemaVersion(1),
    };

    let entry = build_entry(
        Operation::Ingest,
        pipeline_operator(),
        content_hash.clone(),
        content_hash.clone(),
        Default::default(),
    );

    let mut chain = immutara_core::domain::provenance::ProvenanceChain::new();
    chain.push(entry);

    let evidence = Evidence {
        id,
        content_hash,
        metadata,
        provenance: chain,
        ingested_at: Utc::now(),
    };

    Ok(IngestOutput { evidence })
}
