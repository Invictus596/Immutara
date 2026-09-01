//! TUI screens.
//!
//! Each screen renders a specific view. Screens receive only read-only
//! references to `App` state and produce `ratatui` widgets; they never
//! mutate pipeline or business state.

pub mod attestation;
pub mod pipeline;

use ratatui::Frame;

use crate::app::App;

/// The set of switchable screens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// Renders the pipeline event log.
    Pipeline,
    /// Renders attestation records.
    Attestation,
}

/// Render the currently active screen.
pub fn render(screen: Screen, app: &App, frame: &mut Frame) {
    match screen {
        Screen::Pipeline => pipeline::render(app, frame),
        Screen::Attestation => attestation::render(app, frame),
    }
}
