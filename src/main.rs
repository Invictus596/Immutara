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
use immutara_core::domain::verification::VerificationPolicy;
use immutara_pipeline::providers::EvmAttestationProvider;
use immutara_pipeline::providers::PyAnalysisProvider;
use immutara_pipeline::providers::SerpApiLensSearchProvider;
use immutara_pipeline::providers::TineyeImageSearchProvider;
use immutara_pipeline::providers::mocks::{
    MockAnalysisProvider, MockAttestationProvider, MockSearchProvider,
};
use immutara_pipeline::{Pipeline, PipelineInput, media_validation::MediaMatchValidator};
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

/// Map the `[verification.policy]` config section onto the domain policy.
fn policy_from_config(config: &Config) -> VerificationPolicy {
    let p = &config.verification.policy;
    VerificationPolicy {
        version: p.version,
        min_search_matches: p.min_search_matches,
        min_provider_score: p.min_provider_score,
        require_analysis: p.require_analysis,
        min_analysis_confidence: p.min_analysis_confidence,
        required_providers: p.required_providers.clone(),
        max_evidence_age: None,
        social_match_verified: p.social_match_verified,
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

    let search: Arc<dyn immutara_core::providers::ImageSearchProvider> = match config
        .search
        .provider
        .as_str()
    {
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
        "serpapi_lens" => {
            let mut cfg = config.search.serpapi_lens.clone();
            cfg.api_key = config
                .search
                .serpapi_lens
                .resolve_api_key()
                .map_err(|e| format!("{e}"))?;
            Arc::new(SerpApiLensSearchProvider::new(cfg))
        }
        other => {
            return Err(format!(
                "unknown search provider `{other}` (expected `mock`, `tineye` or `serpapi_lens`)"
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

    Ok(
        Pipeline::new(tx, analysis, search.clone(), attestation).with_media_validator(
            search
                .supports_media_validation()
                .then(MediaMatchValidator::default_client),
        ),
    )
}

/// Read an evidence file into `PipelineInput`.
fn read_pipeline_input(path: &str, config: &Config) -> PipelineInput {
    let bytes = std::fs::read(path).expect("failed to read evidence file");
    let file_size = bytes.len() as u64;
    PipelineInput {
        raw_bytes: bytes,
        mime_type: sniff_mime(path),
        file_size,
        source_path: Some(PathBuf::from(path)),
        policy: policy_from_config(config),
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
    let (tx, mut rx) = mpsc::channel(256);
    let pipeline = match build_pipeline(tx, &config) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("{e}");
            return;
        }
    };
    let input = read_pipeline_input(file, &config);

    match pipeline.process(input).await {
        Ok(()) => {
            tracing::info!("pipeline completed for {file}");
            print_run_summary(&mut rx).await;
        }
        Err(e) => tracing::error!("pipeline failed: {e}"),
    }
}

/// Print a concise human-readable summary of a completed run from its event
/// stream, mirroring the fields shown in the TUI detail panels.
async fn print_run_summary(rx: &mut mpsc::Receiver<PipelineEvent>) {
    use immutara_core::domain::attestation::BlockchainVerification;
    use immutara_core::domain::search::SearchInputKind;

    let mut analysis = None;
    let mut search = None;
    let mut search_fallback = None;
    let mut verification = None;
    let mut attestation = None;
    while let Some(event) = rx.recv().await {
        match event {
            PipelineEvent::AnalysisCompleted { result, .. } => {
                analysis = Some(result);
            }
            PipelineEvent::SearchCompleted { result, .. } => {
                search = Some(result);
            }
            PipelineEvent::SearchFallback {
                attempted_input,
                attempted_state,
                reason,
                ..
            } => {
                let input = match attempted_input {
                    SearchInputKind::FaceCrop => "FACE CROP",
                    SearchInputKind::FullImage => "FULL IMAGE",
                };
                search_fallback = Some(format!(
                    "{input} (state {attempted_state:?}) -> FULL IMAGE fallback: {reason}"
                ));
            }
            PipelineEvent::VerificationCompleted { result, .. } => {
                verification = Some(result);
            }
            PipelineEvent::AttestationCompleted {
                record, receipt, ..
            } => {
                attestation = Some((record, receipt));
            }
            _ => {}
        }
        if let Some((_, _)) = attestation.as_ref() {
            break;
        }
    }

    println!("\n========== IMMUTARA RUN SUMMARY ==========");
    if let Some(a) = analysis {
        if let Some(f) = a.face_analysis {
            println!(
                "Analysis : {} | model {} | faces detected: {} | selected face: conf {:.3}, bbox {}, embedding SHA-256 {}",
                f.provider_id,
                f.detector_model,
                f.face_count,
                f.selected_face
                    .as_ref()
                    .map(|s| s.confidence)
                    .unwrap_or(0.0),
                f.selected_face
                    .as_ref()
                    .map(|s| format!(
                        "({},{},{},{})",
                        s.bounding_box.x,
                        s.bounding_box.y,
                        s.bounding_box.width,
                        s.bounding_box.height
                    ))
                    .unwrap_or_else(|| "none".to_string()),
                f.selected_face
                    .as_ref()
                    .map(|s| s.embedding_hash.0.clone())
                    .unwrap_or_else(|| "n/a".to_string())
            );
        }
        println!("Analysis : provider {}", a.provider_id);
    }
    if let Some(s) = search {
        if let Some(note) = &search_fallback {
            println!("Search   : {note}");
        }
        let input = match s.search_input {
            SearchInputKind::FaceCrop => "FACE CROP",
            SearchInputKind::FullImage => "FULL IMAGE",
        };
        let exact = s
            .matches
            .iter()
            .filter(|m| m.match_kind == immutara_core::domain::search::SearchMatchKind::Exact)
            .count();
        let visual = s.matches.len() - exact;
        println!(
            "Search   : provider {} | input {input} | {} matches ({} exact, {} visual) | state {:?}",
            s.provider_id,
            s.matches.len(),
            exact,
            visual,
            s.social_state
        );
        if let Some(sel) = s.selected() {
            let kind = match sel.match_kind {
                immutara_core::domain::search::SearchMatchKind::Exact => "exact",
                immutara_core::domain::search::SearchMatchKind::Visual => "visual",
            };
            println!(
                "  selected (attested): [{kind}] #{:<2} {} | domain {}",
                sel.position.unwrap_or(0),
                sel.source_url.as_deref().unwrap_or("(no url)"),
                sel.source_domain.as_deref().unwrap_or("?")
            );
        }
        for m in s.matches.iter().take(3) {
            println!(
                "  #{:<2} {} {} | domain {} | {}",
                m.position.unwrap_or(0),
                if m.provider_score.is_finite() {
                    format!("score {:.3}", m.provider_score)
                } else {
                    "score n/a".to_string()
                },
                m.source_url.as_deref().unwrap_or("(no url)"),
                m.source_domain.as_deref().unwrap_or("?"),
                match m.match_kind {
                    immutara_core::domain::search::SearchMatchKind::Exact => "exact",
                    immutara_core::domain::search::SearchMatchKind::Visual => "visual",
                }
            );
        }
        let social = s.matches.iter().find(|m| {
            m.source_url
                .as_deref()
                .is_some_and(immutara_core::domain::search::is_social_media_url)
        });
        if let Some(m) = social {
            println!(
                "  social: {}",
                m.source_url.as_deref().unwrap_or("(no url)")
            );
        }
        if let Some(m) = s.selected()
            && let Some(ev) = m.media_match.as_ref()
        {
            let verdict = if ev.passed { "VERIFIED" } else { "UNVERIFIED" };
            let method = match ev.method {
                Some(immutara_core::domain::search::MediaMatchMethod::ExactHash) => {
                    "exact_hash".to_string()
                }
                Some(immutara_core::domain::search::MediaMatchMethod::PerceptualHash) => {
                    format!(
                        "perceptual_hash{}",
                        ev.distance
                            .map(|d| format!(" (distance {d}/{})", ev.threshold.unwrap_or(0)))
                            .unwrap_or_default()
                    )
                }
                None => "not_retrievable".to_string(),
            };
            let note = ev
                .note
                .as_deref()
                .map(|n| format!(" — {n}"))
                .unwrap_or_default();
            println!(
                "  media    : {verdict} {method}{note} | url {}",
                ev.media_url.as_deref().unwrap_or("(none)")
            );
        }
    }
    if let Some(v) = verification {
        println!(
            "Verify   : policy v{} | checks {}: {} passed | verdict {}",
            v.policy_version.0,
            v.checks.len(),
            v.checks.iter().filter(|c| c.passed).count(),
            if v.passed { "PASS" } else { "FAIL" }
        );
        for c in &v.checks {
            println!(
                "   [{:4}] {}",
                if c.passed { "PASS" } else { "FAIL" },
                c.name
            );
        }
    }
    if let Some((record, receipt)) = attestation {
        let blockchain = match receipt.blockchain_verification {
            BlockchainVerification::Verified => "VERIFIED (read-back matches)",
            BlockchainVerification::Failed => "FAILED (read-back mismatch)",
        };
        println!("Attest   : provider {} | {blockchain}", record.provider_id);
        println!("  evidence content hash {}", record.content_hash.0);
        println!("  search result hash   {}", record.search_result_hash.0);
        println!("  on-chain record hash  {}", receipt.on_chain_record_hash);
        println!(
            "  tx {} | block {} | {}",
            receipt.tx_hash, receipt.block_number, receipt.contract_address
        );
    }
    println!("===========================================");
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
    let input = read_pipeline_input(file, &config);

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
        let input = read_pipeline_input(&file, &config);

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
