//! Shared error hierarchy for Immutara.

use thiserror::Error;

/// Top-level error type covering both core and pipeline failures.
///
/// This is deliberately structured and typed (rather than a boxed dynamic
/// error) so that pipeline stages and provider implementations can match on
/// specific failure categories without downcasting.
#[derive(Debug, Error)]
pub enum ImmutaraError {
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("configuration error: {0}")]
    Config(String),

    #[error("invalid domain value: {0}")]
    Validation(String),

    #[error("hashing error: {0}")]
    Hashing(String),

    #[error("provider error ({provider}): {message}")]
    Provider { provider: String, message: String },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("pipeline error: {0}")]
    Pipeline(String),

    #[error("unsupported: {0}")]
    Unsupported(String),
}
