//! Pipeline screen: renders the ordered event log.

use immutara_core::PipelineEvent;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::App;

/// Render the pipeline screen.
pub fn render(app: &App, frame: &mut Frame) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(frame.area());

    let title = Paragraph::new(Span::styled(
        " Immutara — Pipeline  [q] quit  [p] pipeline  [a] attestation ",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ));

    frame.render_widget(title, chunks[0]);

    let events: Vec<ListItem> = app
        .event_log
        .iter()
        .map(|event| ListItem::new(describe(event)))
        .collect();

    let list = List::new(events).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Pipeline Events"),
    );

    frame.render_widget(list, chunks[1]);
}

/// Produce a single-line textual description of an event.
///
/// This is presentation-only formatting; the pipeline's structured data is
/// never computed here.
fn describe(event: &PipelineEvent) -> String {
    match event {
        PipelineEvent::PipelineStarted { .. } => "pipeline started".to_string(),
        PipelineEvent::PipelineCompleted { .. } => "pipeline completed".to_string(),
        PipelineEvent::PipelineFailed { error, .. } => {
            format!("pipeline failed: {error}")
        }
        PipelineEvent::EvidenceIngested { content_hash, .. } => {
            format!(
                "evidence ingested (sha256 {})",
                &content_hash.0[..8.min(content_hash.0.len())]
            )
        }
        PipelineEvent::AnalysisStarted { provider_id, .. } => {
            format!("analysis started ({provider_id})")
        }
        PipelineEvent::AnalysisCompleted { .. } => "analysis completed".to_string(),
        PipelineEvent::AnalysisFailed { error, .. } => {
            format!("analysis failed: {error}")
        }
        PipelineEvent::SearchStarted { provider_id, .. } => {
            format!("search started ({provider_id})")
        }
        PipelineEvent::SearchCompleted { .. } => "search completed".to_string(),
        PipelineEvent::SearchFailed { error, .. } => format!("search failed: {error}"),
        PipelineEvent::VerificationStarted { .. } => "verification started".to_string(),
        PipelineEvent::VerificationCompleted {
            result,
            evidence_id,
            ..
        } => format!(
            "verification {} for {}",
            if result.passed { "PASSED" } else { "FAILED" },
            evidence_id
        ),
        PipelineEvent::AttestationStarted { .. } => "attestation started".to_string(),
        PipelineEvent::AttestationCompleted { record, .. } => {
            format!(
                "attestation complete (tx {})",
                record.tx_hash.as_deref().unwrap_or("pending")
            )
        }
        PipelineEvent::AttestationFailed { error, .. } => {
            format!("attestation failed: {error}")
        }
    }
}
