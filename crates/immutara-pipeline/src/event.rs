//! Re-export of the `PipelineEvent` contract.
//!
//! The event type lives in `immutara-core` so that both the pipeline
//! (producer) and the TUI (consumer) share it without a direct dependency
//! on each other.

pub use immutara_core::domain::event::PipelineEvent;
