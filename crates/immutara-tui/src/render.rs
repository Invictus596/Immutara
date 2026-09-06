//! Ratatui rendering for the Immutara pipeline view.
//!
//! This module is purely presentational: it reads the derived `App` state
//! and produces `ratatui` widgets. It performs no pipeline or business
//! computation.

use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};

use crate::app::{App, Stage, StageId, StageStatus};

const STAGE_TICK: &str = "\u{2713}"; // ✓
const STAGE_RUN: &str = "\u{25b8}"; // ▸
const STAGE_WARN: &str = "\u{2717}"; // ✗

/// Render the full application frame.
pub fn render(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Ratio(1, 2),
            Constraint::Length(1),
        ])
        .split(frame.area());

    render_header(frame, chunks[0], app);
    render_middle(frame, chunks[1], app);
    render_log(frame, chunks[2], app);
    render_footer(frame, chunks[3], app);
}

fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let title = Line::from(vec![
        Span::styled(
            " IMMUTARA ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  visual provenance & evidence verification  "),
        Span::styled("run", Style::default().fg(Color::Gray)),
        Span::raw(format!("  {}", app.run_count.saturating_add(1))),
    ]);

    let status = overall_status(app);
    let status_line = Line::from(Span::styled(
        format!("  {status}"),
        Style::default().fg(Color::Black).bg(status_color(app)),
    ));

    let block = Block::default().borders(Borders::NONE);
    let inner = block.inner(area);
    frame.render_widget(Paragraph::new(vec![title, status_line]), inner);
    frame.render_widget(block, area);
}

fn render_middle(frame: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Ratio(1, 3), Constraint::Ratio(2, 3)])
        .split(area);

    render_stages(frame, chunks[0], app);
    render_details(frame, chunks[1], app);
}

fn render_stages(frame: &mut Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = app
        .stages
        .iter()
        .map(|(id, stage)| stage_item(*id, stage))
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(" PIPELINE STAGES "),
    );
    frame.render_widget(list, area);
}

fn stage_item(id: StageId, stage: &Stage) -> ListItem<'static> {
    let (glyph, color, status_text) = match stage.status {
        StageStatus::Pending => ("  ", Color::DarkGray, "pending"),
        StageStatus::Running => (STAGE_RUN, Color::Yellow, "running"),
        StageStatus::Completed => (STAGE_TICK, Color::Green, "completed"),
        StageStatus::Failed => (STAGE_WARN, Color::Red, "failed"),
    };

    let elapsed = format_elapsed(stage_elapsed(stage));

    let mut line = vec![
        Span::styled(format!(" {glyph} "), Style::default().fg(color)),
        Span::styled(
            format!("{:<16}", id.label()),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::styled(status_text, Style::default().fg(color)),
    ];
    if stage.status == StageStatus::Running || stage.status == StageStatus::Completed {
        line.push(Span::styled(
            format!("  ({elapsed})"),
            Style::default().fg(Color::Gray),
        ));
    }
    if let Some(err) = &stage.error {
        line.push(Span::styled(
            format!("  {err}"),
            Style::default().fg(Color::Red),
        ));
    }

    ListItem::new(Line::from(line))
}

fn render_details(frame: &mut Frame, area: Rect, app: &App) {
    let mut lines: Vec<Line<'static>> = Vec::new();

    lines.push(title_line("EVIDENCE"));
    lines.extend(evidence_lines(app));

    lines.push(title_line("ANALYSIS"));
    lines.extend(analysis_lines(app));

    lines.push(title_line("SEARCH"));
    lines.extend(search_lines(app));

    lines.push(title_line("VERIFICATION"));
    lines.extend(verification_lines(app));

    lines.push(title_line("ATTESTATION"));
    lines.extend(attestation_lines(app));

    let paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(" DETAILS "),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn evidence_lines(app: &App) -> Vec<Line<'static>> {
    match app.evidence.as_ref() {
        Some(ev) => {
            let mut v = vec![
                kv("ID", &ev.id.to_string()),
                kv("SHA-256", &ev.content_hash.0),
                kv("MIME", &ev.metadata.mime_type),
                kv("Size", &format_bytes(ev.metadata.file_size)),
            ];
            if let Some((w, h)) = ev.metadata.dimensions {
                v.push(kv("Dimensions", &format!("{w} × {h}")));
            }
            v
        }
        None => vec![small("waiting for evidence…")],
    }
}

fn analysis_lines(app: &App) -> Vec<Line<'static>> {
    match app.analysis.as_ref() {
        Some(a) => {
            let mut v = vec![
                kv("Provider", &a.provider_id),
                kv("Model", a.model_version.as_deref().unwrap_or("n/a")),
                kv("Objects", &a.objects.to_string()),
                kv("Text regions", &a.text_regions.to_string()),
            ];
            if let Some(c) = a.min_confidence {
                v.push(kv("Min confidence", &format!("{:.0}%", c * 100.0)));
            }
            if let Some(face) = a.face.as_ref() {
                v.push(kv("Detector", &face.detector_model));
                v.push(kv("Recognizer", &face.recognizer_model));
                v.push(kv("Faces detected", &face.face_count.to_string()));
                if let Some(c) = face.selected_confidence {
                    v.push(kv("Selected confidence", &format!("{:.1}%", c * 100.0)));
                }
                if let Some(d) = face.embedding_dim {
                    v.push(kv("Embedding dim", &format!("{d}")));
                }
            }
            v
        }
        None => vec![small("waiting for analysis…")],
    }
}

fn search_lines(app: &App) -> Vec<Line<'static>> {
    match app.search.as_ref() {
        Some(s) => {
            let mut v = vec![
                kv("Provider", &s.provider_id),
                kv("Input", &s.search_input),
                kv(
                    "Results",
                    &format!(
                        "{} ({} exact, {} visual)",
                        s.matches.len(),
                        s.exact_count,
                        s.visual_count
                    ),
                ),
                kv("Social", &s.social_state),
            ];
            if let Some(fb) = &app.search_fallback {
                v.push(indented(format!("  fallback: {fb}")));
            }
            if let Some((si, m)) = s
                .selected_index
                .and_then(|si| s.matches.get(si).map(|m| (si, m)))
            {
                let url = m.source_url.as_deref().unwrap_or("(no url)");
                let domain = m.source_domain.as_deref().unwrap_or("unknown");
                let rank = m.position.unwrap_or((si + 1) as u32);
                let kind = if m.match_kind == "exact" {
                    "EXACT"
                } else {
                    "VISUAL"
                };
                v.push(kv(
                    "Selected result",
                    &format!("[{kind}] #{rank} {}  ({domain})", truncate(url, 40)),
                ));
                if let Some(label) = m.media_match.as_deref() {
                    v.push(kv("Media validation", label));
                }
            }
            if s.matches.is_empty() {
                v.push(small("no search matches"));
            }
            for (idx, m) in s.matches.iter().take(4).enumerate() {
                let url = m.source_url.as_deref().unwrap_or("(no url)");
                let domain = m.source_domain.as_deref().unwrap_or("unknown");
                let rank = m.position.unwrap_or((idx + 1) as u32);
                let score = if m.similarity.is_finite() {
                    format!("{:.0}% match", m.similarity * 100.0)
                } else {
                    "match (score n/a)".to_string()
                };
                let kind = if m.match_kind == "exact" {
                    "exact"
                } else {
                    "visual"
                };
                let marker = if Some(idx) == s.selected_index {
                    "*"
                } else {
                    "·"
                };
                v.push(indented(format!(
                    "  {marker} [{kind}] #{rank} {}  ({domain})  {score}",
                    truncate(url, 36)
                )));
                if let Some(label) = m.media_match.as_deref() {
                    v.push(media_note(label));
                }
            }
            if s.matches.len() > 4 {
                v.push(small(format!("… and {} more", s.matches.len() - 4)));
            }
            v
        }
        None => vec![small("waiting for search…")],
    }
}

fn verification_lines(app: &App) -> Vec<Line<'static>> {
    match app.verification.as_ref() {
        Some(v) => {
            let verdict = if v.passed { "PASS" } else { "FAIL" };
            let color = if v.passed { Color::Green } else { Color::Red };
            let mut lines = vec![
                kv("Policy", &format!("v{}", v.policy_version)),
                Line::from(Span::styled(
                    format!("  Verdict: {verdict}"),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                )),
                kv("Verified at", &v.verified_at.format("%H:%M:%S").to_string()),
            ];
            if v.checks.is_empty() {
                lines.push(small("no checkable criteria in policy"));
            }
            for check in &v.checks {
                let cs = if check.passed { "PASS" } else { "FAIL" };
                let cc = if check.passed {
                    Color::Green
                } else {
                    Color::Red
                };
                lines.push(Line::from(Span::styled(
                    format!("   [{cs}] {}", check.name),
                    Style::default().fg(cc),
                )));
                lines.push(indented(format!("        {}", check.details)));
            }
            lines
        }
        None => vec![small("waiting for verification…")],
    }
}

fn attestation_lines(app: &App) -> Vec<Line<'static>> {
    match app.attestation.as_ref() {
        Some(a) => {
            let mut v = Vec::new();
            if let Some(p) = &a.provider_id {
                v.push(kv("Provider", p));
            }
            if let Some(c) = &a.chain_id {
                v.push(kv("Chain ID", c));
            }
            match &a.contract_address {
                Some(addr) => v.push(kv("Contract", &truncate(addr, 24))),
                None => v.push(kv("Contract", "n/a")),
            }
            match &a.attestation_id {
                Some(id) => v.push(kv("Attestation ID", &truncate(id, 24))),
                None => v.push(kv("Attestation ID", "n/a")),
            }
            if let Some(rec) = &a.record {
                v.push(kv(
                    "Search fingerprint",
                    &truncate(&rec.search_result_hash.0, 24),
                ));
            }
            match &a.tx_hash {
                Some(tx) => v.push(kv("Tx hash", &truncate(tx, 24))),
                None => v.push(kv("Tx hash", "n/a")),
            }
            match a.block_number {
                Some(b) => v.push(kv("Block", &b.to_string())),
                None => v.push(kv("Block", "n/a")),
            }
            match a.status {
                StageStatus::Completed => v.push(Line::from(Span::styled(
                    "  Status: VERIFIED",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ))),
                StageStatus::Failed => v.push(Line::from(Span::styled(
                    format!(
                        "  Status: FAILED — {}",
                        a.error
                            .as_deref()
                            .unwrap_or("on-chain re-verification failed")
                    ),
                    Style::default().fg(Color::Red),
                ))),
                _ => v.push(small("waiting for attestation…")),
            }
            v
        }
        None => vec![small("waiting for attestation…")],
    }
}

fn render_log(frame: &mut Frame, area: Rect, app: &App) {
    let entries = app.log.entries();
    let offset = app.log.offset();
    let start = entries.len().saturating_sub(offset);
    let slice = &entries[..start];

    let items: Vec<ListItem> = slice
        .iter()
        .map(|e| {
            ListItem::new(Line::from(Span::styled(
                if e.is_error {
                    format!(" ! {e}", e = e.message)
                } else {
                    format!("  {}", e.message)
                },
                Style::default().fg(if e.is_error { Color::Red } else { Color::Gray }),
            )))
        })
        .collect();

    let has_old = start > 0 && app.log.offset() > 0;
    let title = if has_old {
        " EVENT LOG  (scrolled up — press End to follow) "
    } else {
        " EVENT LOG "
    };

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(title),
    );
    frame.render_widget(list, area);
}

fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let state_hint = if app.finished {
        "pipeline complete"
    } else {
        "pipeline running"
    };
    let footer = Paragraph::new(Line::from(vec![
        Span::styled(
            " [q]uit  [r]estart  ↑/↓ scroll  PgUp/PgDn  Home/End  ",
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(state_hint, Style::default().fg(Color::DarkGray)),
    ]));
    frame.render_widget(footer, area);
}

// ---- helpers ---------------------------------------------------------

fn overall_status(app: &App) -> String {
    if let Some(e) = &app.overall_error {
        format!("  STATUS: FAILED — {e}")
    } else if app.finished {
        "  STATUS: COMPLETE ".to_string()
    } else if app
        .stages
        .iter()
        .any(|(_, s)| s.status == StageStatus::Running)
    {
        "  STATUS: RUNNING ".to_string()
    } else {
        "  STATUS: IDLE ".to_string()
    }
}

fn status_color(app: &App) -> Color {
    if app.overall_error.is_some() {
        Color::Red
    } else if app.finished {
        Color::Green
    } else if app
        .stages
        .iter()
        .any(|(_, s)| s.status == StageStatus::Running)
    {
        Color::Yellow
    } else {
        Color::DarkGray
    }
}

fn title_line(text: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!(" {text} "),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))
}

fn kv(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {label}: "), Style::default().fg(Color::DarkGray)),
        Span::raw(value.to_string()),
    ])
}

fn indented(text: String) -> Line<'static> {
    Line::from(Span::raw(text))
}

fn small(text: impl Into<String>) -> Line<'static> {
    Line::from(Span::styled(
        format!("  {}", text.into()),
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    ))
}

/// Dimmed note line for a per-match media-validation summary.
fn media_note(label: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!("      media: {label}"),
        Style::default().fg(Color::DarkGray),
    ))
}

fn stage_elapsed(stage: &Stage) -> Option<Duration> {
    match (stage.started_at, stage.finished_at) {
        (Some(s), Some(f)) => Some(f.duration_since(s)),
        (Some(s), None) => Some(s.elapsed()),
        _ => None,
    }
}

fn format_elapsed(d: Option<Duration>) -> String {
    match d {
        Some(d) => format!("{} ms", d.as_millis()),
        None => "—".to_string(),
    }
}

fn format_bytes(n: u64) -> String {
    if n >= 1_048_576 {
        format!("{:.1} MB ({n} B)", n as f64 / 1_048_576.0)
    } else if n >= 1024 {
        format!("{:.1} KB ({n} B)", n as f64 / 1024.0)
    } else {
        format!("{n} B")
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use immutara_core::PipelineEvent;
    use immutara_core::domain::analysis::{
        AnalysisResult, BoundingBox, DetectedObject, FaceAnalysis, SelectedFace,
    };
    use immutara_core::domain::attestation::{AttestationReceipt, BlockchainVerification};
    use immutara_core::domain::evidence::{
        ContentHash, EvidenceId, EvidenceMetadata, SchemaVersion,
    };
    use immutara_core::domain::search::{SearchMatch, SearchResult};
    use immutara_core::domain::verification::{VerificationCheck, VerificationResult};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn build_completed_app() -> App {
        let id = EvidenceId::new();
        let mut app = App::default();
        let meta = EvidenceMetadata {
            source_path: None,
            mime_type: "image/png".into(),
            file_size: 2048,
            dimensions: Some((320, 240)),
            captured_at: None,
            schema_version: SchemaVersion(1),
        };
        use chrono::Utc;
        app.on_pipeline_event(PipelineEvent::PipelineStarted {
            evidence_id: id,
            metadata: meta.clone(),
        });
        app.on_pipeline_event(PipelineEvent::EvidenceIngested {
            evidence_id: id,
            content_hash: ContentHash("c".repeat(64)),
            metadata: meta.clone(),
        });
        app.on_pipeline_event(PipelineEvent::AnalysisStarted {
            evidence_id: id,
            provider_id: "mock-analysis".into(),
        });
        app.on_pipeline_event(PipelineEvent::AnalysisCompleted {
            evidence_id: id,
            result: AnalysisResult {
                evidence_id: id,
                provider_id: "mock-analysis".into(),
                model_version: Some("0.1.0".into()),
                objects: vec![DetectedObject {
                    label: "lens".into(),
                    confidence: 0.92,
                    bounding_box: BoundingBox {
                        x: 0,
                        y: 0,
                        width: 5,
                        height: 5,
                    },
                }],
                text_regions: vec![],
                face_analysis: Some(FaceAnalysis {
                    provider_id: "mock-analysis".into(),
                    face_count: 1,
                    selected_face: Some(SelectedFace {
                        confidence: 0.99,
                        bounding_box: BoundingBox {
                            x: 100,
                            y: 80,
                            width: 220,
                            height: 220,
                        },
                        embedding_dimension: 128,
                        embedding_hash: ContentHash("e".repeat(64)),
                    }),
                    detector_model: "YuNet".into(),
                    recognizer_model: "SFace".into(),
                    model_version: "2021jan".into(),
                }),
                metadata_hash: ContentHash("m".repeat(64)),
                analyzed_at: Utc::now(),
            },
        });
        app.on_pipeline_event(PipelineEvent::SearchStarted {
            evidence_id: id,
            provider_id: "mock-search".into(),
        });
        app.on_pipeline_event(PipelineEvent::SearchCompleted {
            evidence_id: id,
            result: SearchResult {
                evidence_id: id,
                provider_id: "mock-search".into(),
                search_input: immutara_core::domain::search::SearchInputKind::FaceCrop,
                matches: vec![
                    SearchMatch {
                        match_kind: immutara_core::domain::search::SearchMatchKind::Exact,
                        source_url: Some("https://example.net/photo".into()),
                        source_domain: Some("example.net".into()),
                        source_title: None,
                        source_description: Some("example".into()),
                        provider_score: f64::NAN,
                        position: Some(1),
                        first_seen: None,
                        thumbnail_url: None,
                        media_match: None,
                    },
                    SearchMatch {
                        match_kind: immutara_core::domain::search::SearchMatchKind::Visual,
                        source_url: Some("https://example.com/photo".into()),
                        source_domain: Some("example.com".into()),
                        source_title: None,
                        source_description: None,
                        provider_score: 0.85,
                        position: Some(2),
                        first_seen: None,
                        thumbnail_url: None,
                        media_match: None,
                    },
                ],
                searched_at: Utc::now(),
                social_state: immutara_core::domain::search::SearchMatchState::WebMatch,
            },
        });
        app.on_pipeline_event(PipelineEvent::VerificationStarted { evidence_id: id });
        app.on_pipeline_event(PipelineEvent::VerificationCompleted {
            evidence_id: id,
            result: VerificationResult {
                evidence_id: id,
                policy_version: SchemaVersion(1),
                passed: true,
                checks: vec![VerificationCheck {
                    name: "min_search_matches".into(),
                    passed: true,
                    details: "1 matches (min 0)".into(),
                }],
                verified_at: Utc::now(),
            },
        });
        app.on_pipeline_event(PipelineEvent::AttestationStarted { evidence_id: id });
        app.on_pipeline_event(PipelineEvent::AttestationCompleted {
            evidence_id: id,
            record: immutara_core::domain::attestation::AttestationRecord {
                schema_version: SchemaVersion(1),
                pipeline_version: "0.1.0".into(),
                evidence_id: id,
                content_hash: ContentHash("c".repeat(64)),
                metadata_hash: ContentHash("m".repeat(64)),
                verification_result_hash: ContentHash("v".repeat(64)),
                search_result_hash: ContentHash("s".repeat(64)),
                verification_policy_version: SchemaVersion(1),
                provider_id: "mock-attestation".into(),
                chain_id: "0x1".into(),
                attested_at: Utc::now(),
            },
            receipt: AttestationReceipt {
                tx_hash: "0xabc".into(),
                block_number: 42,
                chain_id: "0x1".into(),
                contract_address: "0x5FbDB2315678afecb367f032d93F642f64180aa3".into(),
                attestation_id: "0x11".repeat(32),
                on_chain_record_hash: "0x22".repeat(32),
                blockchain_verification: BlockchainVerification::Verified,
            },
        });
        app.on_pipeline_event(PipelineEvent::PipelineCompleted { evidence_id: id });
        app
    }

    fn render_text(app: &App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| render(f, app))
            .expect("render must not panic");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn completed_run_renders_all_sections() {
        let app = build_completed_app();
        let buf = render_text(&app, 160, 110);

        assert!(buf.contains("IMMUTARA"), "header title");
        assert!(buf.contains("PIPELINE STAGES"), "stages panel");
        assert!(buf.contains("EVIDENCE"), "evidence section");
        assert!(buf.contains("ANALYSIS"), "analysis section");
        assert!(buf.contains("SEARCH"), "search section");
        assert!(buf.contains("VERIFICATION"), "verification section");
        assert!(buf.contains("ATTESTATION"), "attestation section");
        assert!(buf.contains("EVENT LOG"), "event log");
        assert!(buf.contains("image/png"), "mime type rendered");
        // Completed stage glyph present.
        assert!(buf.contains("completed"), "stage status");
        // Attestation shows the verification verdict, not a generic message.
        assert!(buf.contains("VERIFIED"), "attestation verdict rendered");
        assert!(buf.contains("Contract"), "contract address rendered");
        assert!(buf.contains("Attestation ID"), "anchor id rendered");
    }

    #[test]
    fn empty_app_renders_placeholders_without_panic() {
        let app = App::default();
        let buf = render_text(&app, 120, 40);
        assert!(buf.contains("IMMUTARA"));
        assert!(buf.contains("pending"));
    }

    #[test]
    fn renders_gracefully_at_small_sizes() {
        // Small unusual sizes must not panic (simulates resize churn).
        let app = build_completed_app();
        for (w, h) in [(80u16, 20u16), (120, 24), (200, 60), (60, 15)] {
            let _ = render_text(&app, w, h);
        }
        let empty = App::default();
        let _ = render_text(&empty, 40, 10);
    }
}
