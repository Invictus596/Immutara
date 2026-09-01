//! Pipeline stage boundaries.
//!
//! Each stage is a distinct, composable unit with a narrow input/output
//! contract. This module defines the *interfaces*; concrete pipeline wiring
//! lives in the `Pipeline` orchestrator. Stages are intentionally decoupled
//! from concrete providers.

pub mod analyze;
pub mod attest;
pub mod ingest;
pub mod search;
pub mod verify;
