//! Pipeline orchestration for Immutara.
//!
//! The pipeline owns a single authoritative `PipelineEvent` stream
//! published through a `tokio::sync::mpsc` channel. The TUI consumes this
//! stream. Business logic lives here; the TUI contains only presentation.

pub mod event;
pub mod hashing;
pub mod pipeline;
pub mod provenance;
pub mod providers;
pub mod stages;

pub use event::PipelineEvent;
pub use pipeline::{Pipeline, PipelineInput};
