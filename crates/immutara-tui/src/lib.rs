//! Immutara TUI crate.
//!
//! Strictly a presentation layer. It consumes `PipelineEvent`s from the
//! pipeline's `mpsc` channel and renders them. It contains NO pipeline or
//! business logic — only state needed to draw the current view.

pub mod app;
pub mod events;
pub mod screens;
pub mod ui;

pub use app::App;
