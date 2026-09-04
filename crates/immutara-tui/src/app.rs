//! TUI application state, derived purely from `PipelineEvent`s.
//!
//! Every field is the result of reducing the `PipelineEvent` stream produced
//! by the pipeline. This module contains **no** business/pipeline logic — it
//! only translates events into presentation state. The rendering layer reads
//! this state; it never mutates pipeline or domain objects.

use std::time::Instant;

use chrono::{DateTime, Utc};
use immutara_core::PipelineEvent;
use immutara_core::domain::attestation::AttestationRecord;
use immutara_core::domain::evidence::{ContentHash, EvidenceId, EvidenceMetadata};
use immutara_core::domain::verification::{VerificationCheck, VerificationResult};
use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::scroll::EventLog;

/// The ordered pipeline funnel shown in the stage panel.
///
/// `Evidence` and `Ingest` are the single ingestion phase; the pipeline
/// completes it atomically, so they share one completion signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StageId {
    /// Evidence is being read and hashed.
    Evidence,
    /// Analysis (computer-vision) results.
    Analyze,
    /// Reverse-image search results.
    Search,
    /// Policy verification.
    Verify,
    /// On-chain attestation.
    Attest,
}

pub const STAGE_IDS: [StageId; 5] = [
    StageId::Evidence,
    StageId::Analyze,
    StageId::Search,
    StageId::Verify,
    StageId::Attest,
];

impl StageId {
    /// Short human label describing the stage.
    pub fn label(self) -> &'static str {
        match self {
            StageId::Evidence => "Evidence / Ingest",
            StageId::Analyze => "Analyze",
            StageId::Search => "Search",
            StageId::Verify => "Verify",
            StageId::Attest => "Attest",
        }
    }
}

/// Lifecycle status of a single stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

/// A single stage's derived state.
#[derive(Debug, Clone)]
pub struct Stage {
    /// Current lifecycle status.
    pub status: StageStatus,
    /// When the stage began running, if it has.
    pub started_at: Option<Instant>,
    /// When the stage reached a terminal state (completed/failed).
    pub finished_at: Option<Instant>,
    /// Failure message, when `status == Failed`.
    pub error: Option<String>,
}

impl Stage {
    fn new() -> Self {
        Self {
            status: StageStatus::Pending,
            started_at: None,
            finished_at: None,
            error: None,
        }
    }

    fn start(&mut self) {
        if self.started_at.is_none() {
            self.started_at = Some(Instant::now());
        }
        self.status = StageStatus::Running;
        self.error = None;
    }

    fn complete(&mut self) {
        if self.status == StageStatus::Running {
            self.finished_at = Some(Instant::now());
        }
        self.status = StageStatus::Completed;
    }

    fn fail(&mut self, error: String) {
        if self.status != StageStatus::Failed {
            self.finished_at = Some(Instant::now());
        }
        self.status = StageStatus::Failed;
        self.error = Some(error);
    }
}

/// Evidence summary derived from `PipelineStarted` / `EvidenceIngested`.
#[derive(Debug, Clone)]
pub struct EvidenceInfo {
    pub id: EvidenceId,
    pub content_hash: ContentHash,
    pub metadata: EvidenceMetadata,
}

/// Analysis summary derived from `AnalysisCompleted`.
#[derive(Debug, Clone)]
pub struct AnalysisInfo {
    pub provider_id: String,
    pub model_version: Option<String>,
    pub objects: usize,
    pub text_regions: usize,
    /// Min/mean confidence across detected objects.
    pub min_confidence: Option<f64>,
    /// Restrained face-analysis summary (never raw embeddings).
    pub face: Option<FaceInfo>,
}

/// Presentation-safe summary of face analysis.
///
/// Deliberately holds no biometric data: no embedding, no embedding hash —
/// only counts, the selected-face confidence, embedding dimensionality, and
/// model identifiers.
#[derive(Debug, Clone)]
pub struct FaceInfo {
    pub face_count: u32,
    pub selected_confidence: Option<f64>,
    pub embedding_dim: Option<usize>,
    pub detector_model: String,
    pub recognizer_model: String,
}

/// Search summary derived from `SearchCompleted`.
#[derive(Debug, Clone)]
pub struct SearchMatchInfo {
    pub source_url: Option<String>,
    pub similarity: f64,
}

#[derive(Debug, Clone)]
pub struct SearchInfo {
    pub provider_id: String,
    pub matches: Vec<SearchMatchInfo>,
}

/// Verification summary derived from `VerificationCompleted`.
#[derive(Debug, Clone)]
pub struct VerificationInfo {
    pub policy_version: u32,
    pub passed: bool,
    pub checks: Vec<VerificationCheck>,
    pub verified_at: DateTime<Utc>,
}

/// Attestation summary derived from `AttestationCompleted` / `AttestationFailed`.
#[derive(Debug, Clone)]
pub struct AttestationInfo {
    pub record: Option<AttestationRecord>,
    pub provider_id: Option<String>,
    pub chain_id: Option<String>,
    pub tx_hash: Option<String>,
    pub block_number: Option<u64>,
    pub status: StageStatus,
    pub error: Option<String>,
}

/// The outcome of a TUI session, signalling how it ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuiOutcome {
    /// User quit; the process should exit.
    Quit,
    /// User requested a restart; the caller should re-run the pipeline.
    Restart,
}

/// The full presentation state of the TUI, reduced from `PipelineEvent`s.
#[derive(Debug)]
pub struct App {
    /// Ordered stage states.
    pub stages: Vec<(StageId, Stage)>,
    /// Evidence summary.
    pub evidence: Option<EvidenceInfo>,
    /// Analysis summary.
    pub analysis: Option<AnalysisInfo>,
    /// Search summary.
    pub search: Option<SearchInfo>,
    /// Verification summary.
    pub verification: Option<VerificationInfo>,
    /// Attestation summary.
    pub attestation: Option<AttestationInfo>,
    /// Scrollable event log.
    pub log: EventLog,
    /// Whether the session is still live (false after q/Esc or restart).
    pub running: bool,
    /// True when the user requested a restart of the pipeline.
    pub restart_requested: bool,
    /// True once the pipeline fully completed.
    pub finished: bool,
    /// Overall fatal error message, if the pipeline failed.
    pub overall_error: Option<String>,
    /// Incremented each time the pipeline is (re)started.
    pub run_count: u64,
}

impl Default for App {
    fn default() -> Self {
        Self {
            stages: STAGE_IDS.iter().map(|id| (*id, Stage::new())).collect(),
            evidence: None,
            analysis: None,
            search: None,
            verification: None,
            attestation: None,
            log: EventLog::new(),
            running: true,
            restart_requested: false,
            finished: false,
            overall_error: None,
            run_count: 0,
        }
    }
}

impl App {
    /// Reduce a single incoming pipeline event into view state.
    pub fn on_pipeline_event(&mut self, event: PipelineEvent) {
        self.log_event(&event);
        match &event {
            PipelineEvent::PipelineStarted { metadata, .. } => {
                self.run_count = self.run_count.saturating_add(1);
                // Guard against partial resets: start fresh per run.
                self.reset_stages();
                self.stage(StageId::Evidence).start();
                self.evidence = Some(EvidenceInfo {
                    id: event_evidence_id(&event),
                    content_hash: ContentHash("<pending>".into()),
                    metadata: metadata.clone(),
                });
            }
            PipelineEvent::EvidenceIngested { content_hash, .. } => {
                let ev = self.evidence.get_or_insert_with(|| EvidenceInfo {
                    id: event_evidence_id(&event),
                    content_hash: content_hash.clone(),
                    metadata: EvidenceMetadata {
                        source_path: None,
                        mime_type: String::new(),
                        file_size: 0,
                        dimensions: None,
                        captured_at: None,
                        schema_version: immutara_core::domain::evidence::SchemaVersion(0),
                    },
                });
                ev.content_hash = content_hash.clone();
                self.stage(StageId::Evidence).complete();
            }
            PipelineEvent::AnalysisStarted { .. } => self.stage(StageId::Analyze).start(),
            PipelineEvent::AnalysisCompleted { result, .. } => {
                let objects = result.objects.as_slice();
                let min_conf = nonempty_min(objects.iter().map(|o| o.confidence));
                let face = result.face_analysis.as_ref().map(|fa| FaceInfo {
                    face_count: fa.face_count,
                    selected_confidence: fa.selected_face.as_ref().map(|s| s.confidence),
                    embedding_dim: fa.selected_face.as_ref().map(|s| s.embedding_dimension),
                    detector_model: fa.detector_model.clone(),
                    recognizer_model: fa.recognizer_model.clone(),
                });
                self.analysis = Some(AnalysisInfo {
                    provider_id: result.provider_id.clone(),
                    model_version: result.model_version.clone(),
                    objects: result.objects.len(),
                    text_regions: result.text_regions.len(),
                    min_confidence: min_conf,
                    face,
                });
                self.stage(StageId::Analyze).complete();
            }
            PipelineEvent::AnalysisFailed {
                provider_id, error, ..
            } => {
                self.analysis = Some(AnalysisInfo {
                    provider_id: provider_id.clone(),
                    model_version: None,
                    objects: 0,
                    text_regions: 0,
                    min_confidence: None,
                    face: None,
                });
                self.stage(StageId::Analyze).fail(error.clone());
            }
            PipelineEvent::SearchStarted { .. } => self.stage(StageId::Search).start(),
            PipelineEvent::SearchCompleted { result, .. } => {
                self.search = Some(SearchInfo {
                    provider_id: result.provider_id.clone(),
                    matches: result
                        .matches
                        .iter()
                        .map(|m| SearchMatchInfo {
                            source_url: m.source_url.clone(),
                            similarity: m.provider_score,
                        })
                        .collect(),
                });
                self.stage(StageId::Search).complete();
            }
            PipelineEvent::SearchFailed {
                provider_id, error, ..
            } => {
                self.search = Some(SearchInfo {
                    provider_id: provider_id.clone(),
                    matches: Vec::new(),
                });
                self.stage(StageId::Search).fail(error.clone());
            }
            PipelineEvent::VerificationStarted { .. } => self.stage(StageId::Verify).start(),
            PipelineEvent::VerificationCompleted { result, .. } => {
                self.verification = Some(self.verification_info(result));
                self.stage(StageId::Verify).complete();
            }
            PipelineEvent::AttestationStarted { .. } => self.stage(StageId::Attest).start(),
            PipelineEvent::AttestationCompleted { record, .. } => {
                self.attestation = Some(AttestationInfo {
                    record: Some(record.clone()),
                    provider_id: Some(record.provider_id.clone()),
                    chain_id: Some(record.chain_id.clone()),
                    tx_hash: record.tx_hash.clone(),
                    block_number: record.block_number,
                    status: StageStatus::Completed,
                    error: None,
                });
                self.stage(StageId::Attest).complete();
            }
            PipelineEvent::AttestationFailed { error, .. } => {
                let info = self.attestation.get_or_insert(AttestationInfo {
                    record: None,
                    provider_id: None,
                    chain_id: None,
                    tx_hash: None,
                    block_number: None,
                    status: StageStatus::Failed,
                    error: None,
                });
                info.status = StageStatus::Failed;
                info.error = Some(error.clone());
                self.overall_error = Some(error.clone());
                self.stage(StageId::Attest).fail(error.clone());
            }
            PipelineEvent::PipelineCompleted { .. } => {
                self.finished = true;
            }
            PipelineEvent::PipelineFailed { error, .. } => {
                self.overall_error = Some(error.clone());
                // Mark any still-running stage as failed.
                for (_, stage) in self.stages.iter_mut() {
                    if stage.status == StageStatus::Running {
                        stage.fail(error.clone());
                    }
                }
            }
        }
    }

    /// Handle a terminal key event.
    ///
    /// Mutates the app flags (`running`, `restart_requested`) and log scroll
    /// state. It never touches pipeline or domain data.
    pub fn on_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.running = false,
            KeyCode::Char('r') => {
                self.restart_requested = true;
                self.running = false;
            }
            KeyCode::Up => self.log.scroll_up(1),
            KeyCode::Down => self.log.scroll_down(1),
            KeyCode::PageUp => self.log.scroll_up(10),
            KeyCode::PageDown => self.log.scroll_down(10),
            KeyCode::Home => self.log.to_oldest(),
            KeyCode::End => self.log.reset(),
            _ => {}
        }
    }

    /// Stage statuses unaffected by a fatal `PipelineFailed` (used in tests).
    fn reset_stages(&mut self) {
        for (_, stage) in self.stages.iter_mut() {
            *stage = Stage::new();
        }
    }

    fn stage(&mut self, id: StageId) -> &mut Stage {
        self.stages
            .iter_mut()
            .find(|(sid, _)| *sid == id)
            .map(|(_, s)| s)
            .expect("stage must exist")
    }

    fn verification_info(&self, result: &VerificationResult) -> VerificationInfo {
        VerificationInfo {
            policy_version: result.policy_version.0,
            passed: result.passed,
            checks: result.checks.clone(),
            verified_at: result.verified_at,
        }
    }

    /// Append a plain logging line mirroring the event (used by renderer logs).
    fn log_event(&mut self, event: &PipelineEvent) {
        let (msg, is_error) = describe(event);
        self.log.push(msg, is_error);
    }
}

/// Extract the evidence id from any event variant (present in all).
fn event_evidence_id(event: &PipelineEvent) -> EvidenceId {
    match event {
        PipelineEvent::PipelineStarted { evidence_id, .. }
        | PipelineEvent::PipelineCompleted { evidence_id }
        | PipelineEvent::EvidenceIngested { evidence_id, .. }
        | PipelineEvent::AnalysisStarted { evidence_id, .. }
        | PipelineEvent::AnalysisCompleted { evidence_id, .. }
        | PipelineEvent::AnalysisFailed { evidence_id, .. }
        | PipelineEvent::SearchStarted { evidence_id, .. }
        | PipelineEvent::SearchCompleted { evidence_id, .. }
        | PipelineEvent::SearchFailed { evidence_id, .. }
        | PipelineEvent::VerificationStarted { evidence_id }
        | PipelineEvent::VerificationCompleted { evidence_id, .. }
        | PipelineEvent::AttestationStarted { evidence_id }
        | PipelineEvent::AttestationCompleted { evidence_id, .. }
        | PipelineEvent::AttestationFailed { evidence_id, .. }
        | PipelineEvent::PipelineFailed { evidence_id, .. } => *evidence_id,
    }
}

/// Non-fatal minimum of an iterator, or `None` when empty.
fn nonempty_min<I>(iter: I) -> Option<f64>
where
    I: Iterator<Item = f64>,
{
    iter.fold(None, |acc, v| {
        Some(match acc {
            Some(a) => a.min(v),
            None => v,
        })
    })
}

/// Produce a single-line description and severity for the log.
fn describe(event: &PipelineEvent) -> (String, bool) {
    match event {
        PipelineEvent::PipelineStarted { .. } => ("pipeline started".into(), false),
        PipelineEvent::PipelineCompleted { .. } => ("pipeline completed".into(), false),
        PipelineEvent::PipelineFailed { error, .. } => (format!("pipeline failed: {error}"), true),
        PipelineEvent::EvidenceIngested { content_hash, .. } => (
            format!(
                "evidence ingested (sha256 {})",
                &content_hash.0[..8.min(content_hash.0.len())]
            ),
            false,
        ),
        PipelineEvent::AnalysisStarted { provider_id, .. } => {
            (format!("analysis started ({provider_id})"), false)
        }
        PipelineEvent::AnalysisCompleted { .. } => ("analysis completed".into(), false),
        PipelineEvent::AnalysisFailed { error, .. } => (format!("analysis failed: {error}"), true),
        PipelineEvent::SearchStarted { provider_id, .. } => {
            (format!("search started ({provider_id})"), false)
        }
        PipelineEvent::SearchCompleted { .. } => ("search completed".into(), false),
        PipelineEvent::SearchFailed { error, .. } => (format!("search failed: {error}"), true),
        PipelineEvent::VerificationStarted { .. } => ("verification started".into(), false),
        PipelineEvent::VerificationCompleted { result, .. } => (
            format!(
                "verification {}",
                if result.passed { "PASSED" } else { "FAILED" }
            ),
            !result.passed,
        ),
        PipelineEvent::AttestationStarted { .. } => ("attestation started".into(), false),
        PipelineEvent::AttestationCompleted { record, .. } => (
            format!(
                "attestation complete (tx {})",
                record.tx_hash.as_deref().unwrap_or("pending")
            ),
            false,
        ),
        PipelineEvent::AttestationFailed { error, .. } => {
            (format!("attestation failed: {error}"), true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use immutara_core::domain::analysis::{AnalysisResult, BoundingBox, DetectedObject};
    use immutara_core::domain::attestation::AttestationRecord;
    use immutara_core::domain::evidence::{
        ContentHash, EvidenceId, EvidenceMetadata, SchemaVersion,
    };
    use immutara_core::domain::search::{SearchMatch, SearchResult};
    use immutara_core::domain::verification::{VerificationCheck, VerificationResult};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn eid() -> EvidenceId {
        EvidenceId::new()
    }

    fn metadata() -> EvidenceMetadata {
        EvidenceMetadata {
            source_path: None,
            mime_type: "image/jpeg".into(),
            file_size: 1234,
            dimensions: Some((1920, 1080)),
            captured_at: None,
            schema_version: SchemaVersion(1),
        }
    }

    fn started(id: EvidenceId) -> PipelineEvent {
        PipelineEvent::PipelineStarted {
            evidence_id: id,
            metadata: metadata(),
        }
    }

    fn ingested(id: EvidenceId) -> PipelineEvent {
        PipelineEvent::EvidenceIngested {
            evidence_id: id,
            content_hash: ContentHash("a".repeat(64)),
            metadata: metadata(),
        }
    }

    fn analysis(id: EvidenceId, failed: Option<&str>) -> PipelineEvent {
        if let Some(err) = failed {
            return PipelineEvent::AnalysisFailed {
                evidence_id: id,
                provider_id: "mock-analysis".into(),
                error: err.into(),
            };
        }
        PipelineEvent::AnalysisCompleted {
            evidence_id: id,
            result: AnalysisResult {
                evidence_id: id,
                provider_id: "mock-analysis".into(),
                model_version: Some("0.1.0".into()),
                objects: vec![DetectedObject {
                    label: "lens".into(),
                    confidence: 0.9,
                    bounding_box: BoundingBox {
                        x: 0,
                        y: 0,
                        width: 10,
                        height: 10,
                    },
                }],
                text_regions: vec![],
                face_analysis: None,
                metadata_hash: ContentHash("m".repeat(64)),
                analyzed_at: Utc::now(),
            },
        }
    }

    fn search(id: EvidenceId, failed: Option<&str>) -> PipelineEvent {
        if let Some(err) = failed {
            return PipelineEvent::SearchFailed {
                evidence_id: id,
                provider_id: "mock-search".into(),
                error: err.into(),
            };
        }
        PipelineEvent::SearchCompleted {
            evidence_id: id,
            result: SearchResult {
                evidence_id: id,
                provider_id: "mock-search".into(),
                matches: vec![SearchMatch {
                    source_url: Some("https://example.com/x".into()),
                    source_description: None,
                    provider_score: 0.85,
                    first_seen: None,
                    thumbnail_url: None,
                }],
                searched_at: Utc::now(),
            },
        }
    }

    fn verification(id: EvidenceId, passed: bool) -> PipelineEvent {
        PipelineEvent::VerificationCompleted {
            evidence_id: id,
            result: VerificationResult {
                evidence_id: id,
                policy_version: SchemaVersion(1),
                passed,
                checks: vec![VerificationCheck {
                    name: "min_search_matches".into(),
                    passed,
                    details: "1 matches (min 0)".into(),
                }],
                verified_at: Utc::now(),
            },
        }
    }

    fn attestation(id: EvidenceId, failed: Option<&str>) -> PipelineEvent {
        if let Some(err) = failed {
            return PipelineEvent::AttestationFailed {
                evidence_id: id,
                error: err.into(),
            };
        }
        PipelineEvent::AttestationCompleted {
            evidence_id: id,
            record: AttestationRecord {
                schema_version: SchemaVersion(1),
                pipeline_version: "0.1.0".into(),
                evidence_id: id,
                content_hash: ContentHash("a".repeat(64)),
                metadata_hash: ContentHash("m".repeat(64)),
                verification_result_hash: ContentHash("v".repeat(64)),
                verification_policy_version: SchemaVersion(1),
                provider_id: "mock-attestation".into(),
                chain_id: "0x1".into(),
                tx_hash: Some("0xabc".into()),
                block_number: Some(42),
                attested_at: Utc::now(),
            },
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn full_sequence_transitions_all_stages_to_completed() {
        let id = eid();
        let mut app = App::default();

        app.on_pipeline_event(started(id));
        app.on_pipeline_event(ingested(id));
        app.on_pipeline_event(PipelineEvent::AnalysisStarted {
            evidence_id: id,
            provider_id: "mock-analysis".into(),
        });
        app.on_pipeline_event(analysis(id, None));
        app.on_pipeline_event(PipelineEvent::SearchStarted {
            evidence_id: id,
            provider_id: "mock-search".into(),
        });
        app.on_pipeline_event(search(id, None));
        app.on_pipeline_event(PipelineEvent::VerificationStarted { evidence_id: id });
        app.on_pipeline_event(verification(id, true));
        app.on_pipeline_event(PipelineEvent::AttestationStarted { evidence_id: id });
        app.on_pipeline_event(attestation(id, None));
        app.on_pipeline_event(PipelineEvent::PipelineCompleted { evidence_id: id });

        for (_, stage) in &app.stages {
            assert_eq!(
                stage.status,
                StageStatus::Completed,
                "stage should complete"
            );
        }
        assert!(app.finished);
        assert!(app.overall_error.is_none());

        // Evidence info populated.
        let ev = app.evidence.as_ref().expect("evidence present");
        assert_eq!(ev.metadata.mime_type, "image/jpeg");
        assert_eq!(ev.metadata.file_size, 1234);

        // Detail summaries populated.
        assert_eq!(app.analysis.as_ref().unwrap().objects, 1);
        assert_eq!(app.search.as_ref().unwrap().matches.len(), 1);
        assert_eq!(app.verification.as_ref().unwrap().policy_version, 1);
        assert_eq!(app.attestation.as_ref().unwrap().block_number, Some(42));
    }

    #[test]
    fn running_states_observed_between_start_and_complete() {
        let id = eid();
        let mut app = App::default();
        app.on_pipeline_event(started(id));
        app.on_pipeline_event(ingested(id));
        app.on_pipeline_event(PipelineEvent::AnalysisStarted {
            evidence_id: id,
            provider_id: "mock-analysis".into(),
        });

        let analyze = &app.stages[1];
        assert_eq!(analyze.0, StageId::Analyze);
        assert_eq!(analyze.1.status, StageStatus::Running);
    }

    #[test]
    fn analysis_failure_is_non_fatal_and_pipeline_still_completes() {
        let id = eid();
        let mut app = App::default();
        app.on_pipeline_event(started(id));
        app.on_pipeline_event(ingested(id));
        app.on_pipeline_event(PipelineEvent::AnalysisStarted {
            evidence_id: id,
            provider_id: "mock-analysis".into(),
        });
        app.on_pipeline_event(analysis(id, Some("model load failed")));

        assert_eq!(app.stages[1].1.status, StageStatus::Failed);
        // Pipeline continues: later stages can still complete.
        app.on_pipeline_event(PipelineEvent::SearchStarted {
            evidence_id: id,
            provider_id: "mock-search".into(),
        });
        app.on_pipeline_event(search(id, None));
        app.on_pipeline_event(PipelineEvent::VerificationStarted { evidence_id: id });
        app.on_pipeline_event(verification(id, true));
        app.on_pipeline_event(PipelineEvent::AttestationStarted { evidence_id: id });
        app.on_pipeline_event(attestation(id, None));
        app.on_pipeline_event(PipelineEvent::PipelineCompleted { evidence_id: id });

        assert!(app.finished);
        assert!(app.overall_error.is_none());
        // The failed stage stays failed while the rest completed.
        assert_eq!(app.stages[1].1.status, StageStatus::Failed);
        assert_eq!(app.stages[3].1.status, StageStatus::Completed);
    }

    #[test]
    fn attestation_failure_sets_overall_error() {
        let id = eid();
        let mut app = App::default();
        app.on_pipeline_event(started(id));
        app.on_pipeline_event(ingested(id));
        app.on_pipeline_event(PipelineEvent::AnalysisStarted {
            evidence_id: id,
            provider_id: "mock-analysis".into(),
        });
        app.on_pipeline_event(analysis(id, None));
        app.on_pipeline_event(PipelineEvent::SearchStarted {
            evidence_id: id,
            provider_id: "mock-search".into(),
        });
        app.on_pipeline_event(search(id, None));
        app.on_pipeline_event(PipelineEvent::VerificationStarted { evidence_id: id });
        app.on_pipeline_event(verification(id, true));
        app.on_pipeline_event(PipelineEvent::AttestationStarted { evidence_id: id });
        app.on_pipeline_event(attestation(id, Some("chain unreachable")));

        assert_eq!(app.stages[4].1.status, StageStatus::Failed);
        assert!(app.overall_error.is_some());
        assert!(!app.finished);
        // State preceding the failure is preserved.
        assert_eq!(app.analysis.as_ref().unwrap().objects, 1);
        assert_eq!(app.verification.as_ref().unwrap().policy_version, 1);
    }

    #[test]
    fn pipeline_failed_marks_still_running_stage_failed() {
        let id = eid();
        let mut app = App::default();
        app.on_pipeline_event(started(id));
        app.on_pipeline_event(ingested(id));
        app.on_pipeline_event(PipelineEvent::VerificationStarted { evidence_id: id });
        app.on_pipeline_event(PipelineEvent::PipelineFailed {
            evidence_id: id,
            error: "verify exploded".into(),
        });

        let verify = &app.stages[3];
        assert_eq!(verify.1.status, StageStatus::Failed);
        assert!(app.overall_error.is_some());
    }

    #[test]
    fn restart_resets_stage_state() {
        let id = eid();
        let mut app = App::default();
        app.on_pipeline_event(started(id));
        app.on_pipeline_event(ingested(id));
        app.on_pipeline_event(PipelineEvent::AnalysisStarted {
            evidence_id: id,
            provider_id: "mock-analysis".into(),
        });
        app.on_pipeline_event(analysis(id, None));
        assert_eq!(app.stages[1].1.status, StageStatus::Completed);

        // A fresh run starts cleanly.
        app.on_pipeline_event(started(id));
        assert_eq!(app.stages[1].1.status, StageStatus::Pending);
        assert_eq!(app.stages[0].1.status, StageStatus::Running);
        assert_eq!(app.run_count, 2);
    }

    #[test]
    fn quit_key_stops_live_loop() {
        let mut app = App::default();
        assert!(app.running);
        app.on_key(key(KeyCode::Char('q')));
        assert!(!app.running);
    }

    #[test]
    fn restart_key_requests_restart() {
        let mut app = App::default();
        app.on_key(key(KeyCode::Char('r')));
        assert!(app.restart_requested);
        assert!(!app.running);
    }

    #[test]
    fn scroll_keys_move_the_log_viewport() {
        let mut app = App::default();
        for i in 0..20 {
            app.log.push(format!("line {i}"), false);
        }
        app.on_key(key(KeyCode::Up));
        assert_eq!(app.log.offset(), 1);
        app.on_key(key(KeyCode::Down));
        assert_eq!(app.log.offset(), 0);
        app.on_key(key(KeyCode::PageUp));
        assert_eq!(app.log.offset(), 10);
        app.on_key(key(KeyCode::Home));
        assert_eq!(app.log.offset(), 20);
        app.on_key(key(KeyCode::End));
        assert_eq!(app.log.offset(), 0);
    }

    #[test]
    fn log_records_errors_and_info_entry_types() {
        let id = eid();
        let mut app = App::default();
        app.on_pipeline_event(started(id));
        app.on_pipeline_event(attestation(id, Some("boom")));

        let entries = app.log.entries();
        assert!(!entries[0].is_error);
        assert!(entries.last().unwrap().is_error);
    }
}
