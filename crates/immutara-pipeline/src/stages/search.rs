//! Search stage: invokes an `ImageSearchProvider`.

use immutara_core::ImmutaraError;
use immutara_core::domain::evidence::Evidence;
use immutara_core::domain::search::SearchResult;
use immutara_core::providers::ImageSearchProvider;

/// Run reverse-image search for a piece of evidence.
///
/// Async because search performs I/O (network/API).
pub async fn run_search<T: ImageSearchProvider + ?Sized>(
    provider: &T,
    evidence: &Evidence,
) -> Result<SearchResult, ImmutaraError> {
    provider.search(evidence).await
}
