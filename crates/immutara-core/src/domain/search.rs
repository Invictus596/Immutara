//! Reverse-image-search result domain types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::evidence::EvidenceId;

/// A single match returned by a reverse-image-search provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchMatch {
    pub source_url: Option<String>,
    pub source_description: Option<String>,
    pub similarity_score: f64,
    pub first_seen: Option<DateTime<Utc>>,
    pub thumbnail_url: Option<String>,
}

/// The structured output of a reverse-image-search stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchResult {
    pub evidence_id: EvidenceId,
    pub provider_id: String,
    pub matches: Vec<SearchMatch>,
    pub searched_at: DateTime<Utc>,
}
