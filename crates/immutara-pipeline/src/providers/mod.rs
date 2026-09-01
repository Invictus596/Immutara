//! Provider implementations.
//!
//! Concrete provider integrations live here (outside `immutara-core`, which
//! only defines the traits).
//!
//! Currently only *mock* providers are implemented — sufficient to exercise
//! the full pipeline flow deterministically. Real providers (Python CV
//! subprocess, EVM attestation, genuine reverse-image search) will be added
//! as their features are implemented.
//!
//! TODO: `python_analysis`, `stub_search`, `evm_attestation` land here later.

pub mod mocks;

pub use mocks::{MockAnalysisProvider, MockAttestationProvider, MockSearchProvider};
