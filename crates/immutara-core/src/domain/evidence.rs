//! Evidence domain type.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use uuid::Uuid;

use super::provenance::ProvenanceChain;

/// Version tag applied to schema-shaped data to allow evolution over time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SchemaVersion(pub u32);

/// Unique identifier for a piece of evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EvidenceId(pub Uuid);

impl EvidenceId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for EvidenceId {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for EvidenceId {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Hex-encoded SHA-256 content or metadata hash.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContentHash(pub String);

/// Structural metadata about evidence.
///
/// No raw pixel data or biometric content is ever stored here; only
/// non-sensitive structural descriptors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceMetadata {
    pub source_path: Option<PathBuf>,
    pub mime_type: String,
    pub file_size: u64,
    pub dimensions: Option<(u32, u32)>,
    pub captured_at: Option<DateTime<Utc>>,
    pub schema_version: SchemaVersion,
}

/// A piece of visual evidence flowing through the pipeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    pub id: EvidenceId,
    pub content_hash: ContentHash,
    pub metadata: EvidenceMetadata,
    pub provenance: ProvenanceChain,
    pub ingested_at: DateTime<Utc>,
}
