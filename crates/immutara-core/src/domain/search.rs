//! Reverse-image-search result domain types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::evidence::EvidenceId;

/// A single match returned by a reverse-image-search provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchMatch {
    /// Canonical URL where the matching image appears publicly.
    pub source_url: Option<String>,
    /// Provider-supplied description of the source (e.g. domain or format).
    pub source_description: Option<String>,
    /// Provider relevance score normalized to `[0, 1]` where available.
    ///
    /// This is the provider's real signal, never fabricated. When a provider
    /// omits a score, the value is `NaN` ("unavailable") rather than a made-up
    /// number; renderers should treat non-finite values as "n/a".
    pub provider_score: f64,
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
