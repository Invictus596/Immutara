//! Immutara TUI crate.
//!
//! Strictly a presentation layer. It consumes `PipelineEvent`s from the
//! pipeline's `mpsc` channel, reduces them into view state, and renders a
//! single polished pipeline view. It contains NO pipeline or business
//! logic — only state needed to draw the current view.

pub mod app;
pub mod events;
pub mod render;
pub mod scroll;
pub mod ui;

pub use app::{App, StageId, StageStatus, TuiOutcome};
