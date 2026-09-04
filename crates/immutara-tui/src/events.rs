//! Terminal and pipeline event handling.
//!
//! The TUI owns a `mpsc::Receiver<PipelineEvent>` provided by the binary and
//! merges terminal key events with pipeline events into a single loop,
//! delegating each to the `App` state. This file is purely input/rendering
//! plumbing — no business logic.

use std::time::Duration;

use crate::app::{App, TuiOutcome};
use crate::ui::Tui;
use immutara_core::PipelineEvent;
use ratatui::crossterm::event::{self, Event as CrosstermEvent, KeyEventKind};
use tokio::sync::mpsc;

/// Run the TUI event loop, returning how the session ended.
///
/// `receiver` is the pipe side owned by the TUI; the pipeline (or caller)
/// writes into the other end. The terminal is always restored before this
/// returns, including on error.
pub async fn run(
    app: &mut App,
    receiver: &mut mpsc::Receiver<PipelineEvent>,
) -> Result<TuiOutcome, std::io::Error> {
    let mut tui = Tui::open()?;
    let outcome = run_inner(&mut tui, app, receiver).await;
    tui.close();
    outcome
}

async fn run_inner(
    tui: &mut Tui,
    app: &mut App,
    receiver: &mut mpsc::Receiver<PipelineEvent>,
) -> Result<TuiOutcome, std::io::Error> {
    loop {
        if !app.running {
            break;
        }

        // Prefer terminal input, falling back to pipeline events.
        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                CrosstermEvent::Key(key) if key.kind == KeyEventKind::Press => app.on_key(key),
                _ => {}
            }
        }

        // Drain any available pipeline events (non-blocking).
        while let Ok(event) = receiver.try_recv() {
            app.on_pipeline_event(event);
        }

        tui.draw(app)?;

        // Yield so the pipeline task can run on a single-threaded runtime.
        tokio::time::sleep(Duration::from_millis(30)).await;
    }

    Ok(if app.restart_requested {
        TuiOutcome::Restart
    } else {
        TuiOutcome::Quit
    })
}
