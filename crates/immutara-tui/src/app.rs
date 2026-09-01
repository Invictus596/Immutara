//! TUI application state.
//!
//! Holds only the state required to render views: a log of received
//! `PipelineEvent`s and which screen is currently active. No business logic.

use crate::screens::Screen;
use immutara_core::PipelineEvent;
use ratatui::crossterm::event::{KeyCode, KeyEvent};

/// The application view state driving the TUI.
pub struct App {
    /// Received pipeline events, in order, rendered on the pipeline screen.
    pub event_log: Vec<PipelineEvent>,
    /// The currently active screen.
    pub screen: Screen,
    /// Should the app keep running?
    pub running: bool,
}

impl Default for App {
    fn default() -> Self {
        Self {
            event_log: Vec::new(),
            screen: Screen::Pipeline,
            running: true,
        }
    }
}

impl App {
    /// Record a pipeline event into the view state.
    pub fn on_pipeline_event(&mut self, event: PipelineEvent) {
        self.event_log.push(event);
    }

    /// Handle a terminal key event.
    pub fn on_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.running = false,
            KeyCode::Char('p') => self.screen = Screen::Pipeline,
            KeyCode::Char('a') => self.screen = Screen::Attestation,
            _ => {}
        }
    }
}
