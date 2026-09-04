//! Workspace-level integration test.
//!
//! Exercises the public API surface that the CLI uses: building a
//! mock-wired pipeline and validating that events flow through the
//! mpsc channel and the pipeline completes.

use std::collections::HashMap;
use std::sync::Arc;

use immutara_core::PipelineEvent;
use immutara_core::domain::evidence::SchemaVersion;
use immutara_core::domain::verification::VerificationPolicy;
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
async fn mock_pipeline_runs_end_to_end_and_publishes_events() {
    let (tx, mut rx) = mpsc::channel(64);
    let pipeline = Pipeline::new(
        tx,
        Arc::new(MockAnalysisProvider::default()),
        Arc::new(MockSearchProvider::default()),
        Arc::new(MockAttestationProvider::default()),
    );

    pipeline
        .process(PipelineInput {
            raw_bytes: b"evidence".to_vec(),
            mime_type: "image/png".to_string(),
            file_size: 8,
            source_path: None,
            policy: default_policy(),
        })
        .await
        .expect("mock pipeline should succeed");

    let mut counts = HashMap::new();
    while let Some(event) = rx.recv().await {
        let tag = match &event {
            PipelineEvent::PipelineStarted { .. } => "started",
            PipelineEvent::EvidenceIngested { .. } => "ingested",
            PipelineEvent::AnalysisCompleted { .. } => "analysis",
            PipelineEvent::SearchCompleted { .. } => "search",
            PipelineEvent::VerificationCompleted { .. } => "verification",
            PipelineEvent::AttestationCompleted { .. } => "attestation",
            PipelineEvent::PipelineCompleted { .. } => "completed",
            _ => "other",
        };
        *counts.entry(tag.to_string()).or_insert(0) += 1;
        if matches!(event, PipelineEvent::PipelineCompleted { .. }) {
            break;
        }
    }

    // Each stage must have completed exactly once, in sequence.
    assert_eq!(counts["started"], 1);
    assert_eq!(counts["ingested"], 1);
    assert_eq!(counts["analysis"], 1);
    assert_eq!(counts["search"], 1);
    assert_eq!(counts["verification"], 1);
    assert_eq!(counts["attestation"], 1);
    assert_eq!(counts["completed"], 1);
}
