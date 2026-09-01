//! Immutara core library.
//!
//! Defines the domain contracts (types), provider traits, error hierarchy,
//! configuration, and canonical serialization used across the workspace.
//! This crate is intentionally free of UI and orchestration concerns.

pub mod config;
pub mod domain;
pub mod errors;
pub mod providers;
pub mod serialization;

pub use config::Config;
pub use domain::*;
pub use errors::ImmutaraError;
pub use providers::{AnalysisProvider, AttestationProvider, ImageSearchProvider};
pub use serialization::{CanonicalSerialize, CanonicalSerializeForHashing};

pub use domain::event::PipelineEvent;
