//! Immutara CLI entry point.
//!
//! Wires configuration and providers together. In this skeleton phase the
//! `run` and `verify` commands use mock providers, and `tui` runs the mock
//! pipeline live while rendering its event stream.

use std::path::PathBuf;
use std::sync::Arc;

use clap::{Args, Parser, Subcommand};
use immutara_core::PipelineEvent;
use immutara_core::domain::evidence::SchemaVersion;
use immutara_core::domain::verification::VerificationPolicy;
use immutara_pipeline::providers::mocks::{
    MockAnalysisProvider, MockAttestationProvider, MockSearchProvider,
};
use immutara_pipeline::{Pipeline, PipelineInput};
use immutara_tui::{App, events as tui_events};
use tokio::sync::mpsc;

#[derive(Parser)]
#[command(
    name = "immutara",
    about = "Visual provenance and evidence verification pipeline"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the full pipeline on a single evidence file.
    Run(CliArgs),
    /// Verify evidence against the active policy.
    Verify(CliArgs),
    /// Launch the terminal UI.
    Tui(CliArgs),
}

#[derive(Args)]
struct CliArgs {
    /// Path to the evidence file.
    #[arg(short = 'f', long = "file")]
    file: String,
}

/// Build a default verification policy for the skeleton phase.
fn default_policy() -> VerificationPolicy {
    VerificationPolicy {
        version: SchemaVersion(1),
        min_search_matches: 0,
        min_search_similarity: 0.0,
        require_analysis: false,
        min_analysis_confidence: 0.0,
        required_providers: vec![],
        max_evidence_age: None,
    }
}

/// Assemble a mock-wired pipeline.
fn build_mock_pipeline(tx: mpsc::Sender<PipelineEvent>) -> Pipeline {
    Pipeline::new(
        tx,
        Arc::new(MockAnalysisProvider::default()),
        Arc::new(MockSearchProvider::default()),
        Arc::new(MockAttestationProvider::default()),
    )
}

/// Read an evidence file into `PipelineInput`.
fn read_pipeline_input(path: &str) -> PipelineInput {
    let bytes = std::fs::read(path).expect("failed to read evidence file");
    PipelineInput {
        raw_bytes: bytes,
        mime_type: sniff_mime(path),
        policy: default_policy(),
    }
}

/// Guess a MIME type from the file extension (skeleton phase).
fn sniff_mime(path: &str) -> String {
    match PathBuf::from(path).extension().and_then(|e| e.to_str()) {
        Some("png") => "image/png".to_string(),
        Some("jpg") | Some("jpeg") => "image/jpeg".to_string(),
        Some("gif") => "image/gif".to_string(),
        Some("webp") => "image/webp".to_string(),
        Some("bmp") => "image/bmp".to_string(),
        _ => "application/octet-stream".to_string(),
    }
}

async fn run_cli(file: &str) {
    let (tx, _rx) = mpsc::channel(64);
    let pipeline = build_mock_pipeline(tx);
    let input = read_pipeline_input(file);

    match pipeline.process(input).await {
        Ok(()) => tracing::info!("pipeline completed for {file}"),
        Err(e) => tracing::error!("pipeline failed: {e}"),
    }
}

async fn verify_cli(file: &str) {
    let (tx, _rx) = mpsc::channel(64);
    let pipeline = build_mock_pipeline(tx);
    let input = read_pipeline_input(file);

    match pipeline.process(input).await {
        Ok(()) => tracing::info!("verification passed for {file}"),
        Err(e) => tracing::error!("verification failed: {e}"),
    }
}

async fn tui_cli(file: String) {
    let (tx, mut rx) = mpsc::channel(256);
    let pipeline = build_mock_pipeline(tx);
    let input = read_pipeline_input(&file);

    // Run the pipeline in the background while the TUI renders its events.
    tokio::spawn(async move {
        match pipeline.process(input).await {
            Ok(()) => tracing::info!("pipeline completed for {file}"),
            Err(e) => tracing::error!("pipeline failed: {e}"),
        }
    });

    let mut app = App::default();
    match tui_events::run(&mut app, &mut rx).await {
        Ok(()) => {}
        Err(e) => tracing::error!("tui event loop failed: {e}"),
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Run(args) => run_cli(&args.file).await,
        Commands::Verify(args) => verify_cli(&args.file).await,
        Commands::Tui(args) => tui_cli(args.file).await,
    }
}
