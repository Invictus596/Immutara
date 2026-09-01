//! Analysis stage: invokes an `AnalysisProvider`.

use immutara_core::ImmutaraError;
use immutara_core::domain::analysis::AnalysisResult;
use immutara_core::domain::evidence::Evidence;
use immutara_core::providers::AnalysisProvider;

/// Analyze a piece of evidence using the provided analysis provider.
///
/// This is async because provider analysis performs I/O (subprocess/model
/// inference). Provenance linking is the caller's responsibility so that
/// provider dependencies stay out of the stage's core loop.
pub async fn run_analysis<T: AnalysisProvider + ?Sized>(
    provider: &T,
    evidence: &Evidence,
) -> Result<AnalysisResult, ImmutaraError> {
    provider.analyze(evidence).await
}
