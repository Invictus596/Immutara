//! Image search provider trait.

use async_trait::async_trait;

use crate::ImmutaraError;
use crate::domain::evidence::Evidence;
use crate::domain::search::{SearchInput, SearchResult};

/// Contract for a reverse-image-search provider.
///
/// Concrete integrations (stub, genuine search APIs) are implemented in
/// `immutara-pipeline`, not in core.
#[async_trait]
pub trait ImageSearchProvider: Send + Sync {
    /// Search for visually similar images given evidence.
    async fn search(&self, evidence: &Evidence) -> Result<SearchResult, ImmutaraError>;

    /// Search using an explicit input (face crop first, full image otherwise).
    ///
    /// The default implementation searches the full image. Providers that
    /// recognize a face crop (e.g. the SerpApi Google Lens provider) override
    /// this to submit the crop first and fall back to the full image when the
    /// crop yields no useful result. The pipeline additionally retries the
    /// full image when a media-validating provider's crop result did not reach
    /// `SOCIAL_MATCH_VERIFIED`, regardless of candidate count.
    async fn search_with_input(
        &self,
        evidence: &Evidence,
        _input: &SearchInput,
    ) -> Result<SearchResult, ImmutaraError> {
        self.search(evidence).await
    }

    /// Stable identifier for this provider.
    fn provider_id(&self) -> &str;

    /// Whether this provider's results are eligible for independent media
    /// validation. Real reverse-image providers opt in; mock/harness
    /// providers leave this off so tests stay offline and deterministic.
    fn supports_media_validation(&self) -> bool {
        false
    }
}
