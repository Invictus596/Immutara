//! Integration tests exercising the full pipeline flow with mock providers.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use immutara_core::ImmutaraError;
use immutara_core::domain::analysis::{BoundingBox, FaceAnalysis, SelectedFace};
use immutara_core::domain::attestation::AttestationRecord;
use immutara_core::domain::evidence::{ContentHash, Evidence, EvidenceId, SchemaVersion};
use immutara_core::domain::search::{
    SearchInput, SearchInputKind, SearchMatch, SearchMatchKind, SearchMatchState, SearchResult,
};
use immutara_core::domain::verification::VerificationPolicy;
use immutara_core::providers::ImageSearchProvider;
use immutara_pipeline::event::PipelineEvent;
use immutara_pipeline::hashing::search_result_hash;
use immutara_pipeline::providers::mocks::{
    MockAnalysisProvider, MockAttestationProvider, MockSearchProvider,
};
use immutara_pipeline::{Pipeline, PipelineInput};
use tokio::sync::{Mutex, mpsc};

fn default_policy() -> VerificationPolicy {
    VerificationPolicy {
        version: SchemaVersion(1),
        min_search_matches: 0,
        min_provider_score: 0.0,
        require_analysis: false,
        min_analysis_confidence: 0.0,
        required_providers: vec![],
        max_evidence_age: None,
        social_match_verified: false,
    }
}

#[tokio::test]
async fn full_pipeline_emits_lifecycle_events_and_succeeds() {
    let (tx, mut rx) = mpsc::channel(64);

    let analysis = Arc::new(MockAnalysisProvider::default());
    let search = Arc::new(MockSearchProvider::default());
    let attestation = Arc::new(MockAttestationProvider::default());

    let pipeline = Pipeline::new(tx, analysis, search, attestation);

    pipeline
        .process(PipelineInput {
            raw_bytes: b"fake-image-bytes".to_vec(),
            mime_type: "image/jpeg".to_string(),
            file_size: 16,
            source_path: None,
            policy: default_policy(),
        })
        .await
        .expect("pipeline should succeed");

    let mut seen: Vec<String> = Vec::new();
    while let Some(event) = rx.recv().await {
        let tag = match &event {
            PipelineEvent::PipelineStarted { .. } => "started",
            PipelineEvent::EvidenceIngested { .. } => "ingested",
            PipelineEvent::AnalysisStarted { .. } => "analysis_started",
            PipelineEvent::AnalysisCompleted { .. } => "analysis_completed",
            PipelineEvent::SearchStarted { .. } => "search_started",
            PipelineEvent::SearchCompleted { .. } => "search_completed",
            PipelineEvent::VerificationStarted { .. } => "verification_started",
            PipelineEvent::VerificationCompleted { .. } => "verification_completed",
            PipelineEvent::AttestationStarted { .. } => "attestation_started",
            PipelineEvent::AttestationCompleted { .. } => "attestation_completed",
            PipelineEvent::PipelineCompleted { .. } => "completed",
            _ => "other",
        };
        seen.push(tag.to_string());
        if matches!(event, PipelineEvent::PipelineCompleted { .. }) {
            break;
        }
    }

    // Ordered lifecycle expectation.
    let expected = vec![
        "started",
        "ingested",
        "analysis_started",
        "analysis_completed",
        "search_started",
        "search_completed",
        "verification_started",
        "verification_completed",
        "attestation_started",
        "attestation_completed",
        "completed",
    ];
    assert_eq!(seen, expected);
}

#[tokio::test]
async fn attestation_failure_is_fatal_and_emits_failed_event() {
    let (tx, mut rx) = mpsc::channel(64);

    let attestation = MockAttestationProvider {
        fail_with: Some("chain unreachable".to_string()),
        ..Default::default()
    };

    let pipeline = Pipeline::new(
        tx,
        Arc::new(MockAnalysisProvider::default()),
        Arc::new(MockSearchProvider::default()),
        Arc::new(attestation),
    );

    let result = pipeline
        .process(PipelineInput {
            raw_bytes: b"bytes".to_vec(),
            mime_type: "image/png".to_string(),
            file_size: 5,
            source_path: None,
            policy: default_policy(),
        })
        .await;

    assert!(result.is_err());

    let mut saw_failed = false;
    while let Some(event) = rx.recv().await {
        if matches!(event, PipelineEvent::AttestationFailed { .. }) {
            saw_failed = true;
            break;
        }
    }
    assert!(saw_failed);
}

#[tokio::test]
async fn analysis_failure_is_non_fatal() {
    let (tx, mut rx) = mpsc::channel(64);

    let analysis = MockAnalysisProvider {
        fail_with: Some("model load failed".to_string()),
        ..Default::default()
    };

    let pipeline = Pipeline::new(
        tx,
        Arc::new(analysis),
        Arc::new(MockSearchProvider::default()),
        Arc::new(MockAttestationProvider::default()),
    );

    let result = pipeline
        .process(PipelineInput {
            raw_bytes: b"bytes".to_vec(),
            mime_type: "image/jpeg".to_string(),
            file_size: 5,
            source_path: None,
            policy: default_policy(),
        })
        .await;

    // Analysis failure is non-fatal; pipeline still completes.
    assert!(result.is_ok());

    let mut saw_failed = false;
    let mut saw_completed = false;
    while let Some(event) = rx.recv().await {
        match event {
            PipelineEvent::AnalysisFailed { .. } => saw_failed = true,
            PipelineEvent::PipelineCompleted { .. } => {
                saw_completed = true;
                break;
            }
            _ => {}
        }
    }
    assert!(saw_failed);
    assert!(saw_completed);
}

#[tokio::test]
async fn events_carry_evidence_id_and_content_hash() {
    let (tx, mut rx) = mpsc::channel(64);
    let pipeline = Pipeline::new(
        tx,
        Arc::new(MockAnalysisProvider::default()),
        Arc::new(MockSearchProvider::default()),
        Arc::new(MockAttestationProvider::default()),
    );

    pipeline
        .process(PipelineInput {
            raw_bytes: b"deterministic-bytes".to_vec(),
            mime_type: "image/jpeg".to_string(),
            file_size: 19,
            source_path: None,
            policy: default_policy(),
        })
        .await
        .unwrap();

    let mut first_id = None;
    while let Some(event) = rx.recv().await {
        if let PipelineEvent::EvidenceIngested {
            evidence_id,
            content_hash,
            ..
        } = event
        {
            first_id = Some(evidence_id);
            // SHA-256 digests are always 64 hex chars.
            assert_eq!(content_hash.0.len(), 64);
            break;
        }
    }
    assert!(first_id.is_some());
}

// ---- FACE CROP -> FULL IMAGE fallback flow ----

/// Real fixture used so `build_search_input` can produce a face crop locally.
const SOURCE_PATH: &str = "../../py/tests/fixtures/face_lena.jpg";

/// Search provider that scripts a FACE CROP result and a separate FULL IMAGE
/// result, with a call counter per input. Opts into media validation (so the
/// pipeline's verified-state fallback applies) without needing a validator or
/// network: `social_state` is whatever the test scripts.
#[derive(Clone)]
struct ScriptedSearchProvider {
    crop: SearchResult,
    full: SearchResult,
    crop_fail: Option<String>,
    full_fail: Option<String>,
    crop_calls: Arc<Mutex<usize>>,
    full_calls: Arc<Mutex<usize>>,
}

impl ScriptedSearchProvider {
    fn new(crop: SearchResult, full: SearchResult) -> Self {
        Self {
            crop,
            full,
            crop_fail: None,
            full_fail: None,
            crop_calls: Arc::new(Mutex::new(0)),
            full_calls: Arc::new(Mutex::new(0)),
        }
    }

    async fn crop_calls(&self) -> usize {
        *self.crop_calls.lock().await
    }

    async fn full_calls(&self) -> usize {
        *self.full_calls.lock().await
    }
}

#[async_trait]
impl ImageSearchProvider for ScriptedSearchProvider {
    async fn search_with_input(
        &self,
        evidence: &Evidence,
        input: &SearchInput,
    ) -> Result<SearchResult, ImmutaraError> {
        match input {
            SearchInput::FaceCrop(_) => {
                *self.crop_calls.lock().await += 1;
                if let Some(msg) = &self.crop_fail {
                    return Err(ImmutaraError::Provider {
                        provider: "scripted-search".to_string(),
                        message: msg.clone(),
                    });
                }
                Ok(self.crop.clone())
            }
            // Mirrors the real provider: a FULL IMAGE input runs `search()`.
            SearchInput::FullImage => self.search(evidence).await,
        }
    }

    async fn search(&self, _evidence: &Evidence) -> Result<SearchResult, ImmutaraError> {
        *self.full_calls.lock().await += 1;
        if let Some(msg) = &self.full_fail {
            return Err(ImmutaraError::Provider {
                provider: "scripted-search".to_string(),
                message: msg.clone(),
            });
        }
        Ok(self.full.clone())
    }

    fn provider_id(&self) -> &str {
        "scripted-search"
    }

    fn supports_media_validation(&self) -> bool {
        true
    }
}

fn scripted_result(
    input: SearchInputKind,
    state: SearchMatchState,
    matches: Vec<SearchMatch>,
) -> SearchResult {
    SearchResult {
        evidence_id: EvidenceId::new(),
        provider_id: "scripted-search".to_string(),
        search_input: input,
        matches,
        searched_at: Utc::now(),
        social_state: state,
    }
}

fn match_at(position: u32, url: &str) -> SearchMatch {
    SearchMatch {
        match_kind: SearchMatchKind::Visual,
        source_url: Some(url.to_string()),
        source_domain: Some("example.com".to_string()),
        source_title: None,
        source_description: None,
        provider_score: 0.9,
        position: Some(position),
        first_seen: None,
        thumbnail_url: None,
        media_match: None,
    }
}

fn selected_face() -> FaceAnalysis {
    FaceAnalysis {
        provider_id: "mock-analysis".to_string(),
        face_count: 1,
        selected_face: Some(SelectedFace {
            confidence: 0.92,
            bounding_box: BoundingBox {
                x: 10,
                y: 10,
                width: 80,
                height: 100,
            },
            embedding_dimension: 128,
            embedding_hash: ContentHash("e".repeat(64)),
        }),
        detector_model: "YuNet".to_string(),
        recognizer_model: "SFace".to_string(),
        model_version: "2021jan".to_string(),
    }
}

fn input_name(input: &SearchInputKind) -> &'static str {
    match input {
        SearchInputKind::FaceCrop => "FACE CROP",
        SearchInputKind::FullImage => "FULL IMAGE",
    }
}

/// Run the pipeline with `search` as the search provider against the real
/// fixture image (face present, so `build_search_input` yields a FACE CROP)
/// and collect every event until `PipelineCompleted`.
async fn run_with(search: ScriptedSearchProvider) -> Vec<PipelineEvent> {
    assert!(
        std::path::Path::new(SOURCE_PATH).exists(),
        "fixture missing: {SOURCE_PATH}"
    );
    let (tx, mut rx) = mpsc::channel(64);

    let analysis = Arc::new(MockAnalysisProvider {
        provider_id: "mock-analysis".to_string(),
        face_analysis: Some(selected_face()),
        ..Default::default()
    });
    let source_bytes = std::fs::read(SOURCE_PATH).expect("read fixture");

    let pipeline = Pipeline::new(
        tx,
        analysis,
        Arc::new(search),
        Arc::new(MockAttestationProvider::default()),
    );

    pipeline
        .process(PipelineInput {
            raw_bytes: source_bytes.clone(),
            mime_type: "image/jpeg".to_string(),
            file_size: source_bytes.len() as u64,
            source_path: Some(SOURCE_PATH.into()),
            policy: default_policy(),
        })
        .await
        .expect("pipeline should succeed");

    let mut events = Vec::new();
    while let Some(event) = rx.recv().await {
        let done = matches!(event, PipelineEvent::PipelineCompleted { .. });
        events.push(event);
        if done {
            break;
        }
    }
    events
}

/// The ordered trail of Search-stage events, as compact strings.
fn search_steps(events: &[PipelineEvent]) -> Vec<String> {
    events
        .iter()
        .map(|e| match e {
            PipelineEvent::SearchStarted { .. } => "started".to_string(),
            PipelineEvent::SearchCompleted { result, .. } => {
                format!("completed:{}", input_name(&result.search_input))
            }
            PipelineEvent::SearchFallback {
                attempted_input,
                attempted_state,
                ..
            } => format!(
                "fallback:{input}:{state:?}",
                input = input_name(attempted_input),
                state = attempted_state
            ),
            PipelineEvent::SearchFailed { .. } => "failed".to_string(),
            _ => String::new(),
        })
        .filter(|s| !s.is_empty())
        .collect()
}

fn last_search_result(events: &[PipelineEvent]) -> &SearchResult {
    events
        .iter()
        .rev()
        .find_map(|e| match e {
            PipelineEvent::SearchCompleted { result, .. } => Some(result),
            _ => None,
        })
        .expect("at least one SearchCompleted")
}

fn attestation_record(events: &[PipelineEvent]) -> &AttestationRecord {
    events
        .iter()
        .rev()
        .find_map(|e| match e {
            PipelineEvent::AttestationCompleted { record, .. } => Some(record),
            _ => None,
        })
        .expect("attestation completed")
}

fn has_search_fallback(events: &[PipelineEvent]) -> bool {
    events
        .iter()
        .any(|e| matches!(e, PipelineEvent::SearchFallback { .. }))
}

#[tokio::test]
async fn verified_crop_result_does_not_fall_back_to_full_image() {
    let crop_url = "https://www.instagram.com/p/AAAAACropVerified/";
    let full_url = "https://www.example.com/full-never-used/";
    let search = ScriptedSearchProvider::new(
        scripted_result(
            SearchInputKind::FaceCrop,
            SearchMatchState::SocialMatchVerified,
            vec![match_at(1, crop_url)],
        ),
        scripted_result(
            SearchInputKind::FullImage,
            SearchMatchState::SocialMatchVerified,
            vec![match_at(1, full_url)],
        ),
    );

    let events = run_with(search).await;

    assert_eq!(
        search_steps(&events),
        vec!["started", "completed:FACE CROP"]
    );
    assert!(
        !has_search_fallback(&events),
        "no fallback for a verified crop"
    );

    let final_result = last_search_result(&events);
    assert_eq!(final_result.search_input, SearchInputKind::FaceCrop);
    assert_eq!(
        final_result.social_state,
        SearchMatchState::SocialMatchVerified
    );
    assert_eq!(
        final_result
            .selected()
            .and_then(|m| m.source_url.as_deref()),
        Some(crop_url)
    );
    assert_eq!(
        attestation_record(&events).search_result_hash,
        search_result_hash(final_result.selected()).unwrap(),
        "attested hash must be the verified crop result"
    );
}

#[tokio::test]
async fn unverified_crop_falls_back_to_full_image_which_verifies() {
    let crop_url = "https://www.instagram.com/p/BBBBBCropUnverified/";
    let full_url = "https://www.instagram.com/p/CCCCFullVerified/";
    let crop = scripted_result(
        SearchInputKind::FaceCrop,
        SearchMatchState::SocialCandidateUnverified,
        vec![match_at(1, crop_url)],
    );
    let full = scripted_result(
        SearchInputKind::FullImage,
        SearchMatchState::SocialMatchVerified,
        vec![match_at(1, full_url)],
    );
    let search = ScriptedSearchProvider::new(crop, full);
    let counters = search.clone();

    let events = run_with(search).await;

    // Exactly one crop search, then exactly one full-image search.
    assert_eq!(counters.crop_calls().await, 1);
    assert_eq!(counters.full_calls().await, 1);

    assert_eq!(
        search_steps(&events),
        vec![
            "started",
            "completed:FACE CROP",
            "fallback:FACE CROP:SocialCandidateUnverified",
            "started",
            "completed:FULL IMAGE",
        ]
    );

    // The FULL IMAGE result is the final, attested result.
    let final_result = last_search_result(&events);
    assert_eq!(final_result.search_input, SearchInputKind::FullImage);
    assert_eq!(
        final_result.social_state,
        SearchMatchState::SocialMatchVerified
    );
    assert_eq!(
        final_result.matches.len(),
        1,
        "no duplicated/merged matches"
    );
    assert_eq!(
        final_result
            .selected()
            .and_then(|m| m.source_url.as_deref()),
        Some(full_url)
    );
    assert_eq!(
        attestation_record(&events).search_result_hash,
        search_result_hash(final_result.selected()).unwrap(),
        "attested hash must be the final verified FULL IMAGE result, not the crop"
    );
}

#[tokio::test]
async fn zero_match_crop_falls_back_to_full_image() {
    let full_url = "https://www.instagram.com/p/DDDDFullFromEmptyCrop/";
    let search = ScriptedSearchProvider::new(
        scripted_result(
            SearchInputKind::FaceCrop,
            SearchMatchState::NoResults,
            vec![],
        ),
        scripted_result(
            SearchInputKind::FullImage,
            SearchMatchState::SocialMatchVerified,
            vec![match_at(1, full_url)],
        ),
    );

    let events = run_with(search).await;

    assert_eq!(
        search_steps(&events),
        vec![
            "started",
            "completed:FACE CROP",
            "fallback:FACE CROP:NoResults",
            "started",
            "completed:FULL IMAGE",
        ]
    );
    let final_result = last_search_result(&events);
    assert_eq!(
        final_result
            .selected()
            .and_then(|m| m.source_url.as_deref()),
        Some(full_url)
    );
    assert_eq!(
        final_result.social_state,
        SearchMatchState::SocialMatchVerified
    );
}

#[tokio::test]
async fn when_full_image_also_fails_to_verify_the_honest_failure_is_preserved() {
    let crop_url = "https://www.instagram.com/p/EEEEEFailedCrop/";
    let full_url = "https://www.instagram.com/p/FFFFFullUnverified/";
    let search = ScriptedSearchProvider::new(
        scripted_result(
            SearchInputKind::FaceCrop,
            SearchMatchState::SocialCandidateUnverified,
            vec![match_at(1, crop_url)],
        ),
        scripted_result(
            SearchInputKind::FullImage,
            SearchMatchState::SocialCandidateUnverified,
            vec![match_at(1, full_url)],
        ),
    );

    let events = run_with(search).await;

    // The pipeline never manufactures a verified state where none exists.
    let final_result = last_search_result(&events);
    assert_eq!(final_result.search_input, SearchInputKind::FullImage);
    assert_eq!(
        final_result.social_state,
        SearchMatchState::SocialCandidateUnverified
    );
    assert_eq!(
        final_result
            .selected()
            .and_then(|m| m.source_url.as_deref()),
        Some(full_url)
    );
    // Still attested (search failure/weakness is non-fatal), but the bound
    // hash is the honest, unreverified full-image result.
    assert_eq!(
        attestation_record(&events).search_result_hash,
        search_result_hash(final_result.selected()).unwrap()
    );
}

#[tokio::test]
async fn crop_search_failure_is_non_fatal_and_no_fallback_is_attempted() {
    let mut search = ScriptedSearchProvider::new(
        scripted_result(
            SearchInputKind::FaceCrop,
            SearchMatchState::NoResults,
            vec![],
        ),
        scripted_result(
            SearchInputKind::FullImage,
            SearchMatchState::SocialMatchVerified,
            vec![],
        ),
    );
    search.crop_fail = Some("Lens quota exceeded".to_string());

    let events = run_with(search).await;

    assert_eq!(search_steps(&events), vec!["started", "failed"]);
    assert!(
        !has_search_fallback(&events),
        "no fallback after a failed crop"
    );
    // Pipeline still completed and attested.
    assert!(
        events
            .iter()
            .any(|e| matches!(e, PipelineEvent::AttestationCompleted { .. }))
    );
}

#[tokio::test]
async fn fallback_search_failure_keeps_the_honest_crop_result() {
    let crop_url = "https://www.instagram.com/p/GGGGCropKeptAfterFullFail/";
    let mut search = ScriptedSearchProvider::new(
        scripted_result(
            SearchInputKind::FaceCrop,
            SearchMatchState::SocialCandidateUnverified,
            vec![match_at(1, crop_url)],
        ),
        scripted_result(
            SearchInputKind::FullImage,
            SearchMatchState::SocialMatchVerified,
            vec![],
        ),
    );
    search.full_fail = Some("Lens quota exceeded".to_string());

    let events = run_with(search).await;

    assert_eq!(
        search_steps(&events),
        vec![
            "started",
            "completed:FACE CROP",
            "fallback:FACE CROP:SocialCandidateUnverified",
            "started",
            "failed",
        ]
    );
    // The crop result stands: pipeline still completes and attests its honest
    // (unverified) state rather than dropping it or inventing a score.
    let final_result = last_search_result(&events);
    assert_eq!(final_result.search_input, SearchInputKind::FaceCrop);
    assert_eq!(
        final_result
            .selected()
            .and_then(|m| m.source_url.as_deref()),
        Some(crop_url)
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, PipelineEvent::AttestationCompleted { .. }))
    );
}

#[tokio::test]
async fn non_validating_provider_never_falls_back() {
    // Even with a detected face (FACE CROP input), a provider that does not
    // opt into media validation must not trigger the verified-state fallback.
    let (tx, mut rx) = mpsc::channel(64);
    assert!(std::path::Path::new(SOURCE_PATH).exists());
    let analysis = Arc::new(MockAnalysisProvider {
        provider_id: "mock-analysis".to_string(),
        face_analysis: Some(selected_face()),
        ..Default::default()
    });
    let pipeline = Pipeline::new(
        tx,
        analysis,
        Arc::new(MockSearchProvider::default()),
        Arc::new(MockAttestationProvider::default()),
    );
    let source_bytes = std::fs::read(SOURCE_PATH).unwrap();
    let file_size = source_bytes.len() as u64;
    pipeline
        .process(PipelineInput {
            raw_bytes: source_bytes,
            mime_type: "image/jpeg".to_string(),
            file_size,
            source_path: Some(SOURCE_PATH.into()),
            policy: default_policy(),
        })
        .await
        .unwrap();

    let mut events = Vec::new();
    while let Some(event) = rx.recv().await {
        let done = matches!(event, PipelineEvent::PipelineCompleted { .. });
        events.push(event);
        if done {
            break;
        }
    }

    assert_eq!(
        search_steps(&events),
        vec!["started", "completed:FULL IMAGE"]
    );
    assert!(!has_search_fallback(&events));
}
