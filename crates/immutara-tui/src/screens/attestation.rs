//! Attestation screen: renders attestation records from received events.

use immutara_core::PipelineEvent;
use immutara_core::domain::attestation::AttestationRecord;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::App;

/// Render the attestation screen.
pub fn render(app: &App, frame: &mut Frame) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(frame.area());

    let title = Paragraph::new(Span::styled(
        " Immutara — Attestation  [q] quit  [p] pipeline  [a] attestation ",
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
    ));
    frame.render_widget(title, chunks[0]);

    let records: Vec<ListItem> = collect_records(app)
        .into_iter()
        .map(ListItem::new)
        .collect();

    let list = List::new(records).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Attestation Records"),
    );
    frame.render_widget(list, chunks[1]);
}

/// Extract attestation records from the app event log (read-only).
fn collect_records(app: &App) -> Vec<String> {
    app.event_log
        .iter()
        .filter_map(|event| match event {
            PipelineEvent::AttestationCompleted { record, .. } => Some(format_record(record)),
            _ => None,
        })
        .collect()
}

fn format_record(record: &AttestationRecord) -> String {
    format!(
        "evidence {} | tx {} | block {:?} | chain {}",
        record.evidence_id,
        record.tx_hash.as_deref().unwrap_or("n/a"),
        record.block_number,
        record.chain_id
    )
}
