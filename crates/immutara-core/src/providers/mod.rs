//! External provider abstraction layer.
//!
//! `immutara-core` defines the *contracts* only. Concrete implementations
//! (mock, Python subprocess, EVM, etc.) live in `immutara-pipeline`.

pub mod analysis;
pub mod attestation;
pub mod search;

pub use analysis::AnalysisProvider;
pub use attestation::AttestationProvider;
pub use search::ImageSearchProvider;
