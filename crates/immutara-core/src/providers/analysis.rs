//! Analysis provider trait.
//!
//! Implemented by the Python computer-vision subprocess and by mock
//! providers for testing. Async because it performs I/O (subprocess,
//! model inference).

use async_trait::async_trait;

use crate::ImmutaraError;
use crate::domain::analysis::AnalysisResult;
use crate::domain::evidence::Evidence;

/// Contract for a computer-vision analysis provider.
#[async_trait]
pub trait AnalysisProvider: Send + Sync {
    /// Analyze a piece of evidence and return structured analysis results.
    async fn analyze(&self, evidence: &Evidence) -> Result<AnalysisResult, ImmutaraError>;

    /// Stable identifier for this provider.
    fn provider_id(&self) -> &str;
}
