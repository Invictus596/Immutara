//! Immutara CLI entry point.
//!
//! Wires configuration and providers together. In this skeleton phase the
//! `run` and `verify` commands use mock providers, and `tui` runs the mock
//! pipeline live while rendering its event stream.

use std::path::PathBuf;
use std::sync::Arc;

use clap::{Args, Parser, Subcommand};
use immutara_core::PipelineEvent;
use immutara_core::config::Config;
use immutara_core::domain::evidence::SchemaVersion;
use immutara_core::domain::verification::VerificationPolicy;
use immutara_pipeline::providers::EvmAttestationProvider;
use immutara_pipeline::providers::PyAnalysisProvider;
use immutara_pipeline::providers::TineyeImageSearchProvider;
use immutara_pipeline::providers::mocks::{
    MockAnalysisProvider, MockAttestationProvider, MockSearchProvider,
};
use immutara_pipeline::{Pipeline, PipelineInput};
use immutara_tui::{App, TuiOutcome, events as tui_events};
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

    /// Path to the TOML configuration file.
    #[arg(short = 'c', long = "config", default_value = "config/default.toml")]
    config: String,
}

/// Build a default verification policy for the skeleton phase.
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

/// Assemble a pipeline whose analysis provider is chosen from config.
fn build_pipeline(tx: mpsc::Sender<PipelineEvent>, config: &Config) -> Result<Pipeline, String> {
    let analysis: Arc<dyn immutara_core::providers::AnalysisProvider> =
        match config.analysis.provider.as_str() {
            "mock" => Arc::new(MockAnalysisProvider::default()),
            "opencv" => Arc::new(PyAnalysisProvider::new(config.analysis.opencv.clone())),
            other => {
                return Err(format!(
                    "unknown analysis provider `{other}` (expected `mock` or `opencv`)"
                ));
            }
        };

    let search: Arc<dyn immutara_core::providers::ImageSearchProvider> =
        match config.search.provider.as_str() {
            "mock" => Arc::new(MockSearchProvider::default()),
            "tineye" => {
                let mut cfg = config.search.tineye.clone();
                cfg.api_key = config
                    .search
                    .tineye
                    .resolve_api_key()
                    .map_err(|e| format!("{e}"))?;
                Arc::new(TineyeImageSearchProvider::new(cfg))
            }
            other => {
                return Err(format!(
                    "unknown search provider `{other}` (expected `mock` or `tineye`)"
                ));
            }
        };

    let attestation: Arc<dyn immutara_core::providers::AttestationProvider> =
        match config.attestation.provider.as_str() {
            "mock" => Arc::new(MockAttestationProvider::default()),
            "evm" => Arc::new(
                EvmAttestationProvider::new(config.attestation.evm.clone())
                    .map_err(|e| format!("{e}"))?,
            ),
            other => {
                return Err(format!(
                    "unknown attestation provider `{other}` (expected `mock` or `evm`)"
                ));
            }
        };

    Ok(Pipeline::new(tx, analysis, search, attestation))
}

/// Read an evidence file into `PipelineInput`.
fn read_pipeline_input(path: &str) -> PipelineInput {
    let bytes = std::fs::read(path).expect("failed to read evidence file");
    let file_size = bytes.len() as u64;
    PipelineInput {
        raw_bytes: bytes,
        mime_type: sniff_mime(path),
        file_size,
        source_path: Some(PathBuf::from(path)),
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

async fn run_cli(file: &str, config: Config) {
    let (tx, _rx) = mpsc::channel(64);
    let pipeline = match build_pipeline(tx, &config) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("{e}");
            return;
        }
    };
    let input = read_pipeline_input(file);

    match pipeline.process(input).await {
        Ok(()) => tracing::info!("pipeline completed for {file}"),
        Err(e) => tracing::error!("pipeline failed: {e}"),
    }
}

async fn verify_cli(file: &str, config: Config) {
    let (tx, _rx) = mpsc::channel(64);
    let pipeline = match build_pipeline(tx, &config) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("{e}");
            return;
        }
    };
    let input = read_pipeline_input(file);

    match pipeline.process(input).await {
        Ok(()) => tracing::info!("verification passed for {file}"),
        Err(e) => tracing::error!("verification failed: {e}"),
    }
}

async fn tui_cli(file: String, config: Config) {
    // The TUI runs the pipeline in the background per "run"; pressing `r`
    // re-runs it with a fresh channel and fresh view state.
    loop {
        let (tx, mut rx) = mpsc::channel(256);
        // Re-select providers from a fresh config each run.
        let pipeline = match build_pipeline(tx, &config) {
            Ok(p) => p,
            Err(e) => {
                tracing::error!("{e}");
                break;
            }
        };
        let input = read_pipeline_input(&file);

        // Run the pipeline in the background while the TUI renders its events.
        let file_for_task = file.clone();
        tokio::spawn(async move {
            match pipeline.process(input).await {
                Ok(()) => tracing::info!("pipeline completed for {file_for_task}"),
                Err(e) => tracing::error!("pipeline failed: {e}"),
            }
        });

        let mut app = App::default();
        match tui_events::run(&mut app, &mut rx).await {
            Ok(TuiOutcome::Restart) => {
                tracing::info!("restarting pipeline ({file})");
                continue;
            }
            Ok(TuiOutcome::Quit) | Err(_) => break,
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let cli = Cli::parse();

    let config_path = match &cli.command {
        Commands::Run(args) => args.config.clone(),
        Commands::Verify(args) => args.config.clone(),
        Commands::Tui(args) => args.config.clone(),
    };
    let config = match Config::from_path(&config_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("failed to load config {config_path}: {e}");
            std::process::exit(1);
        }
    };

    match cli.command {
        Commands::Run(args) => run_cli(&args.file, config).await,
        Commands::Verify(args) => verify_cli(&args.file, config).await,
        Commands::Tui(args) => tui_cli(args.file, config).await,
    }
}
