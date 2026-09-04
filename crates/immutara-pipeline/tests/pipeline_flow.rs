//! Integration tests exercising the full pipeline flow with mock providers.

use std::sync::Arc;

use immutara_core::domain::evidence::SchemaVersion;
use immutara_core::domain::verification::VerificationPolicy;
use immutara_pipeline::event::PipelineEvent;
use immutara_pipeline::providers::mocks::{
    MockAnalysisProvider, MockAttestationProvider, MockSearchProvider,
};
use immutara_pipeline::{Pipeline, PipelineInput};
use tokio::sync::mpsc;

fn default_policy() -> VerificationPolicy {
    VerificationPolicy {
        version: SchemaVersion(1),
        min_search_matches: 0,
        min_provider_score: 0.0,
        require_analysis: false,
        min_analysis_confidence: 0.0,
        required_providers: vec![],
        max_evidence_age: None,
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
