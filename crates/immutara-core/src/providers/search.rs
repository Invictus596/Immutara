//! Image search provider trait.

use async_trait::async_trait;

use crate::ImmutaraError;
use crate::domain::evidence::Evidence;
use crate::domain::search::SearchResult;

/// Contract for a reverse-image-search provider.
///
/// Concrete integrations (stub, genuine search APIs) are implemented in
/// `immutara-pipeline`, not in core.
#[async_trait]
pub trait ImageSearchProvider: Send + Sync {
    /// Search for visually similar images given evidence.
    async fn search(&self, evidence: &Evidence) -> Result<SearchResult, ImmutaraError>;

    /// Stable identifier for this provider.
    fn provider_id(&self) -> &str;
}
