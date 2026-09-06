//! Development-only media-discovery tool.
//!
//! Given one or more images, this tool runs the exact production stages used
//! by the `run` pipeline — face detection + crop, Google Lens reverse-image
//! search, and independent public-media validation — and prints the best
//! candidate. It is a disposable investigator's tool, NOT part of the
//! attestation path: URLs are never hardcoded, scores are never fabricated,
//! and every claim it prints comes from the same deterministic modules the
//! pipeline uses.
//!
//! Usage: `cargo run -p immutara-pipeline --bin discover -- ./img1.jpg ./img2.png`
//!
//! Requires `SERPAPI_API_KEY` in the environment and the OpenCV models on
//! disk (same requirements as the real `run` command).

use chrono::Utc;
use immutara_core::config::{Config, OpenCvAnalysisConfig, SerpApiLensConfig};
use immutara_core::domain::evidence::{ContentHash, Evidence, EvidenceMetadata};
use immutara_core::domain::provenance::ProvenanceChain;
use immutara_core::domain::search::{SearchInput, SearchInputKind};
use immutara_core::providers::{AnalysisProvider, ImageSearchProvider};
use immutara_pipeline::face_crop;
use immutara_pipeline::media_validation::{MediaMatchValidator, MediaValidationConfig};
use immutara_pipeline::providers::PyAnalysisProvider;
use immutara_pipeline::providers::SerpApiLensSearchProvider;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut config_path = "config/default.toml".to_string();
    let mut images = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--config" {
            config_path = args.next().expect("--config <path>");
        } else {
            images.push(arg);
        }
    }
    if images.is_empty() {
        eprintln!("usage: discover [--config <toml>] <image> [image...]");
        std::process::exit(2);
    }

    let config = Config::from_path(&config_path).expect("config file");
    let opencv = OpenCvAnalysisConfig {
        models_dir: config.analysis.opencv.models_dir.clone(),
        python_package_dir: config.analysis.opencv.python_package_dir.clone(),
        python: config.analysis.opencv.python.clone(),
        worker_module: config.analysis.opencv.worker_module.clone(),
        detection_threshold: config.analysis.opencv.detection_threshold,
        max_image_dimension: config.analysis.opencv.max_image_dimension,
        timeout_seconds: config.analysis.opencv.timeout_seconds,
    };
    let mut lens_cfg = SerpApiLensConfig {
        api_url: config.search.serpapi_lens.api_url.clone(),
        ..SerpApiLensConfig::default()
    };
    lens_cfg.api_key = config
        .search
        .serpapi_lens
        .resolve_api_key()
        .expect("SERPAPI_API_KEY set");
    let lens_cfg = lens_cfg;

    let analysis = PyAnalysisProvider::new(opencv);
    let search = SerpApiLensSearchProvider::new(lens_cfg);
    let validator = MediaMatchValidator::new(MediaValidationConfig::default());

    let mut best: Option<(String, f32)> = None;
    for path in &images {
        let summary = discover_one(&analysis, &search, &validator, path).await;
        match summary {
            Some((label, score)) => {
                println!("\nBEST for {path}: {label} (priority {score:.2})");
                if best.as_ref().map(|(_, s)| *s > score).unwrap_or(true) {
                    best = Some((label, score));
                }
            }
            None => eprintln!("\n{path}: no usable face detected; skipped"),
        }
    }
    if let Some((label, _)) = best {
        println!("\nBEST CANDIDATE OVERALL: {label}");
    }
}

async fn discover_one(
    analysis: &PyAnalysisProvider,
    search: &SerpApiLensSearchProvider,
    validator: &MediaMatchValidator,
    path: &str,
) -> Option<(String, f32)> {
    let bytes = std::fs::read(path).ok()?;
    let evidence = Evidence {
        id: immutara_core::domain::evidence::EvidenceId::new(),
        content_hash: ContentHash(immutara_pipeline::hashing::sha256_hex(&bytes)),
        metadata: EvidenceMetadata {
            source_path: Some(path.into()),
            mime_type: "image/jpeg".into(),
            file_size: bytes.len() as u64,
            dimensions: None,
            captured_at: None,
            schema_version: immutara_core::domain::evidence::SchemaVersion(1),
        },
        provenance: ProvenanceChain::new(),
        ingested_at: Utc::now(),
    };

    let analyzed = analysis.analyze(&evidence).await.ok();
    let face = analyzed.as_ref().and_then(|a| a.face_analysis.as_ref());
    let input = match face.and_then(|f| f.selected_face.as_ref()) {
        Some(selected_face) => {
            println!(
                "\n[{path}] faces={} selected=(conf {:.3}, bbox ({},{},{},{}))",
                face.map(|f| f.face_count).unwrap_or(0),
                selected_face.confidence,
                selected_face.bounding_box.x,
                selected_face.bounding_box.y,
                selected_face.bounding_box.width,
                selected_face.bounding_box.height,
            );
            match face_crop::generate_face_crop(&bytes, selected_face.bounding_box, 500_000) {
                Ok(crop) => SearchInput::FaceCrop(crop),
                Err(_) => SearchInput::FullImage,
            }
        }
        None => {
            println!("\n[{path}] faces=0 -> FULL IMAGE search");
            SearchInput::FullImage
        }
    };
    let mut result = search.search_with_input(&evidence, &input).await.ok()?;

    let query = {
        let searched = match &input {
            SearchInput::FaceCrop(crop) => crop.image_bytes.clone(),
            SearchInput::FullImage => bytes.clone(),
        };
        immutara_pipeline::media_validation::ValidationQuery::new(searched, bytes.clone())
    };
    validator.validate_matches(&mut result, &query).await;

    let exact = result
        .matches
        .iter()
        .filter(|m| m.match_kind == immutara_core::domain::search::SearchMatchKind::Exact)
        .count();
    println!(
        "  lens: input {} | {} matches ({} exact, {} visual) | state {:?}",
        match result.search_input {
            SearchInputKind::FaceCrop => "FACE CROP",
            SearchInputKind::FullImage => "FULL IMAGE",
        },
        result.matches.len(),
        exact,
        result.matches.len() - exact,
        result.social_state,
    );

    for m in result.matches.iter() {
        let url = m.source_url.as_deref().unwrap_or("(no url)");
        let domain = m.source_domain.as_deref().unwrap_or("?");
        let kind = if m.match_kind == immutara_core::domain::search::SearchMatchKind::Exact {
            "exact"
        } else {
            "visual"
        };
        println!(
            "  #{:<2} [{kind}] {url}  ({domain})",
            m.position.unwrap_or(0)
        );
        if let Some(ev) = &m.media_match {
            println!(
                "       media: {} (distance {:?} / threshold {:?}){} | url {}",
                if ev.passed { "VERIFIED" } else { "UNVERIFIED" },
                ev.distance,
                ev.threshold,
                ev.note
                    .as_deref()
                    .map(|n| format!(" {n}"))
                    .unwrap_or_default(),
                ev.media_url.as_deref().unwrap_or("(none)"),
            );
        }
    }

    result.selected().map(|sel| {
        let priority = if sel.media_match.as_ref().map(|e| e.passed).unwrap_or(false) {
            3.0
        } else if sel.match_kind == immutara_core::domain::search::SearchMatchKind::Exact {
            2.0
        } else {
            1.0
        };
        let url = sel.source_url.as_deref().unwrap_or("(no url)");
        (format!("{url} (state {:?})", result.social_state), priority)
    })
}
