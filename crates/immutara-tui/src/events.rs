//! Terminal and pipeline event handling.
//!
//! The TUI owns a `mpsc::Receiver<PipelineEvent>` provided by the binary and
//! merges terminal key events with pipeline events into a single loop,
//! delegating each to the `App` state. This file is purely input/rendering
//! plumbing — no business logic.

use std::time::Duration;

use crate::app::App;
use crate::ui::Tui;
use immutara_core::PipelineEvent;
use ratatui::crossterm::event::{self, Event as CrosstermEvent, KeyEventKind};
use tokio::sync::mpsc;

/// Run the TUI event loop until the app signals shutdown.
///
/// `receiver` is the pipe side owned by the TUI; the pipeline (or caller)
/// writes into the other end.
pub async fn run(
    app: &mut App,
    receiver: &mut mpsc::Receiver<PipelineEvent>,
) -> Result<(), std::io::Error> {
    let mut tui = Tui::open()?;

    loop {
        if !app.running {
            break;
        }

        // Prefer terminal input, falling back to pipeline events.
        if event::poll(Duration::from_millis(100))? {
            if let CrosstermEvent::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    app.on_key(key);
                }
            }
        }

        // Drain any available pipeline events (non-blocking).
        while let Ok(event) = receiver.try_recv() {
            app.on_pipeline_event(event);
        }

        tui.draw(app)?;
        std::thread::sleep(Duration::from_millis(50));
    }

    tui.close();
    Ok(())
}
