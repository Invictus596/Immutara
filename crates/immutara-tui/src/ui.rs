//! Terminal rendering.
//!
//! Owns the `ratatui::Terminal` and exposes open/draw/close. Presentation-only.

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use std::io;

use crate::app::App;

/// A live terminal session for rendering TUI frames.
pub struct Tui {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
}

impl Tui {
    /// Initialize the terminal and create a fresh session.
    pub fn open() -> io::Result<Self> {
        enable_raw_mode()?;
        let stdout = io::stdout();
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok(Self { terminal })
    }

    /// Render one frame of the active screen.
    pub fn draw(&mut self, app: &App) -> io::Result<()> {
        self.terminal
            .draw(|frame| crate::render::render(frame, app))?;
        Ok(())
    }

    /// Show cursor and restore the terminal.
    pub fn close(&mut self) {
        let _ = self.terminal.show_cursor();
        let _ = disable_raw_mode();
    }
}
