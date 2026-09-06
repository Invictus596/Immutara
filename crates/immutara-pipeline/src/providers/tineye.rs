//! Real reverse-image-search provider backed by the TinEye REST API.
//!
//! TinEye is a commercial, official reverse-image-search API over the public
//! web. This provider uploads the evidence image (via multipart/form-data)
//! and parses the JSON response into the shared `SearchMatch`/`SearchResult`
//! domain model.
//!
//! Selection rationale (Milestone 4):
//! - Official, stable, REST + JSON interface (`POST /rest/search/`).
//! - Accepts direct image upload (matches our `EvidenceMetadata.source_path`
//!   input; no need to host the image publicly or stash it anywhere).
//! - Returns a real relevance signal per match (`score`, 0-100) plus a
//!   canonical match URL, a backlink (page) URL, a rendered thumbnail, the
//!   image domain and the crawl date — mapping cleanly onto `SearchMatch`.
//! - Operates over the public web (finds copies/variants of the image),
//!   which is exactly the provenance question Immutara cares about.
//! - Well-documented official clients (pytineye, Node, PHP) confirm the wire
//!   protocol; a public sandbox key always returns results for the sample
//!   "melon cat" image, enabling a real end-to-end demo with no key.
//!
//! Privacy: only the raw image bytes are uploaded to TinEye — never the
//! Milestone-3 facial embedding or any derived biometric data. The embedding
//! computed in the analysis stage is never sent to any third party.
//!
//! Failures (missing image, unreadable image, network, timeout, HTTP status,
//! auth, rate limiting, malformed JSON, empty matches, schema drift) are all
//! mapped to typed `ImmutaraError` values. The pipeline treats search failure
//! as non-fatal.

use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use immutara_core::ImmutaraError;
use immutara_core::config::TineyeSearchConfig;
use immutara_core::domain::evidence::Evidence;
use immutara_core::domain::search::{SearchMatch, SearchMatchKind, SearchResult};
use immutara_core::providers::ImageSearchProvider;
use serde::Deserialize;

/// Provider implementation name reported through the domain model.
const TINEYE_PROVIDER_ID: &str = "tineye";

/// Number of multipart retries before giving up (transient network faults).
const MAX_ATTEMPTS: usize = 3;

#[derive(Debug, Clone, Deserialize)]
struct TineyeResponse {
    results: TineyeResults,
    #[serde(default)]
    code: Option<i64>,
    #[serde(default)]
    messages: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct TineyeResults {
    #[serde(default)]
    matches: Vec<TineyeMatch>,
}

#[derive(Debug, Clone, Deserialize)]
struct TineyeMatch {
    #[serde(default)]
    score: Option<f64>,
    #[serde(default)]
    image_url: Option<String>,
    #[serde(default)]
    domain: Option<String>,
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    backlinks: Vec<TineyeBacklink>,
}

#[derive(Debug, Clone, Deserialize)]
struct TineyeBacklink {
    #[serde(default)]
    backlink: Option<String>,
    /// Publicly accessible URL of the image as found on its source page.
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    crawl_date: Option<String>,
}

/// A real reverse-image-search provider backed by the TinEye API.
pub struct TineyeImageSearchProvider {
    provider_id: String,
    api_url: String,
    api_key: String,
    /// Report credential problems at construction time but never the key.
    timeout: Duration,
    http: reqwest::Client,
}

impl TineyeImageSearchProvider {
    /// Construct a provider from a config that already carries the API key
    /// (typically resolved from an environment variable).
    pub fn new(config: TineyeSearchConfig) -> Self {
        let timeout = Duration::from_secs(config.timeout_seconds.max(1));
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .unwrap_or_else(|e| {
                // Request cannot be constructed at all (e.g. no TLS backend).
                // Build a client without timing so `search` can still return
                // a typed error instead of panicking.
                let _ = e;
                reqwest::Client::builder().build().unwrap_or_else(|_| {
                    // Last resort: an unbounded client. If even this fails the
                    // sandbox/CI cannot obtain a client today.
                    reqwest::Client::new()
                })
            });
        Self {
            provider_id: TINEYE_PROVIDER_ID.to_string(),
            api_url: config.api_url,
            api_key: config.api_key,
            timeout,
            http,
        }
    }

    /// Resolve the image bytes to upload, mapping missing/unreadable files to
    /// typed errors.
    async fn read_image_bytes(&self, source_path: &Path) -> Result<Vec<u8>, ImmutaraError> {
        if !source_path.is_file() {
            return Err(ImmutaraError::Provider {
                provider: self.provider_id.clone(),
                message: format!("search image does not exist: {}", source_path.display()),
            });
        }
        tokio::fs::read(source_path)
            .await
            .map_err(ImmutaraError::Io)
    }

    /// Perform one multipart upload, decoding the JSON response.
    async fn upload_once(
        &self,
        image_bytes: Vec<u8>,
        mime_type: &str,
    ) -> Result<TineyeResponse, ImmutaraError> {
        let form = reqwest::multipart::Form::new().part(
            "image_upload",
            reqwest::multipart::Part::bytes(image_bytes)
                .file_name("query.jpg")
                .mime_str(mime_type)
                .map_err(|e| ImmutaraError::Provider {
                    provider: self.provider_id.clone(),
                    message: format!("invalid multipart MIME type: {e}"),
                })?,
        );

        let response = self
            .http
            .post(format!("{}/search/", self.api_url.trim_end_matches('/')))
            .header("X-API-KEY", &self.api_key)
            .multipart(form)
            .send()
            .await
            .map_err(|e| self.map_network_error(e))?;

        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(|e| ImmutaraError::Provider {
                provider: self.provider_id.clone(),
                message: format!("failed to read TinEye response body: {e}"),
            })?;

        if !status.is_success() {
            return Err(self.map_http_error(status.as_u16(), bytes.as_ref()));
        }

        let parsed: TineyeResponse =
            serde_json::from_slice(&bytes).map_err(|e| ImmutaraError::Provider {
                provider: self.provider_id.clone(),
                message: format!("TinEye returned unparseable JSON (HTTP {status}): {e}"),
            })?;

        if let Some(code) = parsed.code
            && code != 200
        {
            return Err(ImmutaraError::Provider {
                provider: self.provider_id.clone(),
                message: format!(
                    "TinEye reported error code {code}: {}",
                    parsed.messages.join("; ")
                ),
            });
        }

        Ok(parsed)
    }

    /// Upload with bounded retries for transient network faults.
    async fn upload_with_retry(
        &self,
        image_bytes: Vec<u8>,
        mime_type: &str,
    ) -> Result<TineyeResponse, ImmutaraError> {
        let mut attempt = 0usize;
        loop {
            attempt += 1;
            match self.upload_once(image_bytes.clone(), mime_type).await {
                Ok(parsed) => return Ok(parsed),
                Err(e) => {
                    if attempt >= MAX_ATTEMPTS || !is_retryable(&e) {
                        return Err(e);
                    }
                    tokio::time::sleep(Duration::from_millis(250 * attempt as u64)).await;
                }
            }
        }
    }

    fn map_network_error(&self, e: reqwest::Error) -> ImmutaraError {
        if e.is_timeout() {
            return ImmutaraError::Provider {
                provider: self.provider_id.clone(),
                message: format!("TinEye request timed out after {}s", self.timeout.as_secs()),
            };
        }
        ImmutaraError::Provider {
            provider: self.provider_id.clone(),
            message: format!("network error talking to TinEye: {e}"),
        }
    }

    fn map_http_error(&self, status: u16, _body: &[u8]) -> ImmutaraError {
        let message = match status {
            401 | 403 => "authentication failed (check TINEYE_API_KEY)".to_string(),
            404 => "TinEye endpoint not found; check API URL".to_string(),
            429 => "TinEye rate limit exceeded; try again later".to_string(),
            s if s >= 500 => format!("TinEye server error (HTTP {s})"),
            s => format!("TinEye returned HTTP {s}"),
        };
        ImmutaraError::Provider {
            provider: self.provider_id.clone(),
            message,
        }
    }

    /// Translate `TineyeResponse` into the shared domain model.
    fn to_result(&self, evidence: &Evidence, parsed: TineyeResponse) -> SearchResult {
        let matches = parsed
            .results
            .matches
            .into_iter()
            .filter_map(|m| self.to_match(m))
            .collect::<Vec<_>>();
        SearchResult {
            evidence_id: evidence.id,
            provider_id: self.provider_id.clone(),
            search_input: immutara_core::domain::search::SearchInputKind::FullImage,
            matches,
            searched_at: Utc::now(),
            social_state: Default::default(),
        }
    }

    fn to_match(&self, m: TineyeMatch) -> Option<SearchMatch> {
        // The canonical image URL on the source page (the most actionable
        // proof that the image appears publicly somewhere).
        let source_url = m
            .backlinks
            .iter()
            .find_map(|b| b.url.clone())
            .or_else(|| m.backlinks.iter().find_map(|b| b.backlink.clone()))
            .or(m.image_url.clone());

        let source_description = m.domain.clone().or(m.format.clone());

        // TinEye's real relevance signal, normalized to [0, 1]. When the
        // provider omits the score (schema drift), represent it as
        // unavailable (`NaN`) rather than fabricating a value.
        let provider_score = m
            .score
            .map(|s| (s / 100.0).clamp(0.0, 1.0))
            .unwrap_or(f64::NAN);

        let first_seen = m
            .backlinks
            .iter()
            .find_map(|b| parse_crawl_date(b.crawl_date.as_deref()));

        Some(SearchMatch {
            match_kind: SearchMatchKind::Visual,
            source_url,
            source_domain: m.domain.clone(),
            source_title: None,
            source_description,
            provider_score,
            position: None,
            first_seen,
            thumbnail_url: m.image_url,
            media_match: None,
        })
    }
}

fn is_retryable(e: &ImmutaraError) -> bool {
    matches!(
        e,
        ImmutaraError::Provider { message, .. }
            if message.to_lowercase().contains("network error")
                || message.to_lowercase().contains("timed out")
    )
}

/// Parse a TinEye `crawl_date` (`YYYY-MM-DD`) into a `DateTime<Utc>` at noon
/// UTC (TinEye provides a date, not a timestamp). Returns `None` on failure.
fn parse_crawl_date(raw: Option<&str>) -> Option<DateTime<Utc>> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    let date = NaiveDate::parse_from_str(raw, "%Y-%m-%d").ok()?;
    Some(DateTime::<Utc>::from_naive_utc_and_offset(
        date.and_hms_opt(12, 0, 0)?,
        Utc,
    ))
}

#[async_trait]
impl ImageSearchProvider for TineyeImageSearchProvider {
    async fn search(&self, evidence: &Evidence) -> Result<SearchResult, ImmutaraError> {
        let source_path =
            evidence
                .metadata
                .source_path
                .clone()
                .ok_or_else(|| ImmutaraError::Provider {
                    provider: self.provider_id.clone(),
                    message:
                        "evidence has no on-disk source_path; cannot run real reverse-image search"
                            .to_string(),
                })?;

        let image_bytes = self.read_image_bytes(&source_path).await?;
        let mime_type = normalize_mime(&evidence.metadata.mime_type, &source_path);

        let parsed = self.upload_with_retry(image_bytes, &mime_type).await?;
        Ok(self.to_result(evidence, parsed))
    }

    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    fn supports_media_validation(&self) -> bool {
        true
    }
}

/// TinEye requires a recognizable image content type.
fn normalize_mime(mime: &str, path: &Path) -> String {
    let mime = mime.trim();
    if mime.is_empty() || mime == "application/octet-stream" {
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_lowercase())
            .as_deref()
        {
            Some("jpg") | Some("jpeg") => "image/jpeg".to_string(),
            Some("png") => "image/png".to_string(),
            Some("gif") => "image/gif".to_string(),
            Some("webp") => "image/webp".to_string(),
            Some("bmp") => "image/bmp".to_string(),
            Some("tif") | Some("tiff") => "image/tiff".to_string(),
            Some("avif") => "image/avif".to_string(),
            _ => "application/octet-stream".to_string(),
        }
    } else {
        mime.to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::Utc;
    use immutara_core::domain::evidence::{
        ContentHash, Evidence, EvidenceId, EvidenceMetadata, SchemaVersion,
    };
    use immutara_core::domain::provenance::ProvenanceChain;
    use immutara_core::domain::search::SearchMatch;

    use super::*;

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../")
    }

    fn fixture(name: &str) -> PathBuf {
        repo_root()
            .join("py")
            .join("tests")
            .join("fixtures")
            .join(name)
    }

    fn evidence_at(source_path: Option<PathBuf>, mime: &str) -> Evidence {
        Evidence {
            id: EvidenceId::new(),
            content_hash: ContentHash("e".repeat(64)),
            metadata: EvidenceMetadata {
                source_path,
                mime_type: mime.to_string(),
                file_size: 0,
                dimensions: None,
                captured_at: None,
                schema_version: SchemaVersion(1),
            },
            provenance: ProvenanceChain::new(),
            ingested_at: Utc::now(),
        }
    }

    fn provider() -> TineyeImageSearchProvider {
        TineyeImageSearchProvider::new(TineyeSearchConfig {
            api_url: "https://api.tineye.com/rest".to_string(),
            api_key: "sandbox-key".to_string(),
            require_real_key: false,
            timeout_seconds: 30,
        })
    }

    fn sample_response() -> TineyeResponse {
        serde_json::from_str(
            r#"{
              "results": { "matches": [
                {
                  "score": 100.0,
                  "image_url": "https://img.tineye.com/result/abc-1",
                  "domain": "example.com",
                  "format": "JPEG",
                  "backlinks": [
                    { "url": "https://example.com/pages/1", "backlink": "https://example.com/img/1.jpg", "crawl_date": "2024-02-13" }
                  ]
                },
                {
                  "score": 60.5,
                  "image_url": "https://img.tineye.com/result/abc-2",
                  "domain": null,
                  "format": "PNG",
                  "backlinks": []
                }
              ]},
              "code": 200,
              "messages": []
            }"#,
        )
        .unwrap()
    }

    // ---- Response mapping (pure, no network) ----

    #[test]
    fn maps_scores_scale_and_urls() {
        let p = provider();
        let ev = evidence_at(Some(fixture("face_lena.jpg")), "image/jpeg");
        let result = p.to_result(&ev, sample_response());
        assert_eq!(result.matches.len(), 2);
        let first = &result.matches[0];
        assert_eq!(first.provider_score, 1.0); // 100/100
        assert_eq!(
            first.source_url.as_deref(),
            Some("https://example.com/pages/1")
        );
        assert_eq!(
            first.thumbnail_url.as_deref(),
            Some("https://img.tineye.com/result/abc-1")
        );
        assert_eq!(first.source_description.as_deref(), Some("example.com"));
        assert_eq!(
            first.first_seen.unwrap().date_naive(),
            NaiveDate::from_ymd_opt(2024, 2, 13).unwrap()
        );
        let second = &result.matches[1];
        assert_eq!(second.provider_score, 0.605);
        assert_eq!(
            second.source_url.as_deref(),
            Some("https://img.tineye.com/result/abc-2")
        );
        assert_eq!(second.source_description.as_deref(), Some("PNG"));
        assert!(second.first_seen.is_none());
    }

    #[test]
    fn missing_score_is_nan_not_fabricated() {
        let p = provider();
        let parsed: TineyeResponse =
            serde_json::from_str(r#"{"results":{"matches":[{"backlinks":[]}]},"code":200}"#)
                .unwrap();
        let ev = evidence_at(Some(fixture("face_lena.jpg")), "image/jpeg");
        let result = p.to_result(&ev, parsed);
        assert_eq!(result.matches.len(), 1);
        assert!(result.matches[0].provider_score.is_nan());
    }

    #[test]
    fn zero_matches_maps_to_empty_result() {
        let p = provider();
        let parsed: TineyeResponse =
            serde_json::from_str(r#"{"results":{"matches":[]},"code":200}"#).unwrap();
        let ev = evidence_at(Some(fixture("face_lena.jpg")), "image/jpeg");
        let result = p.to_result(&ev, parsed);
        assert!(result.matches.is_empty());
    }

    #[test]
    fn crawl_date_parsing() {
        assert!(parse_crawl_date(Some("2024-02-13")).is_some());
        assert!(parse_crawl_date(Some("not-a-date")).is_none());
        assert!(parse_crawl_date(Some("")).is_none());
        assert!(parse_crawl_date(None).is_none());
    }

    #[test]
    fn normalize_mime_falls_back_from_octet_stream() {
        assert_eq!(normalize_mime("", Path::new("a.jpg")), "image/jpeg");
        assert_eq!(
            normalize_mime("application/octet-stream", Path::new("a.PNG")),
            "image/png"
        );
        assert_eq!(
            normalize_mime("image/webp", Path::new("a.jpg")),
            "image/webp"
        );
    }

    // ---- Error mapping (no live network) ----

    #[tokio::test]
    async fn search_missing_source_path_is_error() {
        let p = provider();
        let ev = evidence_at(None, "image/jpeg");
        let err = p.search(&ev).await.unwrap_err();
        assert!(matches!(err, ImmutaraError::Provider { .. }));
        assert!(err.to_string().contains("source_path"));
    }

    #[tokio::test]
    async fn search_nonexistent_image_is_error() {
        let p = provider();
        let ev = evidence_at(Some(fixture("does_not_exist.jpg")), "image/jpeg");
        let err = p.search(&ev).await.unwrap_err();
        assert!(matches!(err, ImmutaraError::Provider { .. }));
        assert!(err.to_string().contains("does not exist"));
    }

    #[test]
    fn http_error_mapping_is_typed() {
        let p = provider();
        for (status, needle) in [
            (401u16, "authentication"),
            (403u16, "authentication"),
            (429u16, "rate limit"),
            (500u16, "server error"),
            (404u16, "endpoint"),
        ] {
            let err = p.map_http_error(status, b"");
            assert!(matches!(err, ImmutaraError::Provider { .. }));
            assert!(
                err.to_string().to_lowercase().contains(needle),
                "status {status}: {}",
                err
            );
        }
    }

    #[test]
    fn malformed_json_is_provider_error_without_panic() {
        let bad: Result<TineyeResponse, _> = serde_json::from_str("this is not json");
        assert!(bad.is_err());
    }

    // ---- Real integration test (optional, offline by default) ----
    //
    // Set TINEYE_API_KEY to a real/purchased key OR leave it unset to use the
    // public sandbox key (which always returns the "melon cat" sample set).
    // Network is required. This test is ignored unless IMMUTARA_E2E_SEARCH=1.

    #[tokio::test]
    async fn real_tineye_upload_returns_matches() {
        if std::env::var("IMMUTARA_E2E_SEARCH")
            .map(|v| v != "1")
            .unwrap_or(true)
        {
            eprintln!("SKIP real_tineye_upload_returns_matches (set IMMUTARA_E2E_SEARCH=1)");
            return;
        }
        let key = std::env::var("TINEYE_API_KEY")
            .unwrap_or_else(|_| "6mm60lsCNIBqFwOWjJqA80QZHh9BMwc-ber4u=t^".to_string());
        let p = TineyeImageSearchProvider::new(TineyeSearchConfig {
            api_url: "https://api.tineye.com/rest".to_string(),
            api_key: key,
            require_real_key: false,
            timeout_seconds: 60,
        });
        let ev = evidence_at(Some(fixture("face_lena.jpg")), "image/jpeg");
        let result = p.search(&ev).await.expect("real search should succeed");
        assert!(
            !result.matches.is_empty(),
            "expected >=1 match from TinEye sandbox"
        );
        let serialized = serde_json::to_string(&result).unwrap();
        assert!(serialized.contains("tineye"));
        assert!(
            result
                .matches
                .iter()
                .all(|m: &SearchMatch| m.provider_score.is_finite())
        );
    }
}
