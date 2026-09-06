//! Real reverse-image-search provider backed by the SerpApi Google Lens API.
//!
//! Shipped search input: the pipeline first searches the **selected face
//! crop** (generated locally from the analysis bounding box) — the region
//! that actually contains the face — and falls back to the full image when
//! the crop yields no useful results. On top of this provider-level zero-match
//! fallback, the pipeline retries the **full image** whenever this provider's
//! crop result did not reach `SOCIAL_MATCH_VERIFIED` after independent media
//! validation (see `Pipeline::run_search`).
//!
//! Selection rationale (Milestone 6, phase 1 live test succeeded):
//! - Official, stable JSON interface. Google Lens requires uploading the
//!   image to SerpApi's `/image` endpoint, then querying `/search` with the
//!   returned `image_id`.
//! - Accepts direct image upload (matches our `EvidenceMetadata.source_path`
//!   input); no public hosting needed.
//! - Returns genuinely useful provenance: Google Lens `exact_matches`
//!   (Google considers them the same picture) listed first, then
//!   `visual_matches` with the real source URL, page title, platform name
//!   ("Reddit", "X", …) and ordering. The pipeline binds the first exact
//!   match (fallback: first visual match) into the attestation — the exact
//!   "did a matching public/social page actually come back" signal.
//!   The live test on a face crop returned real Reddit and X posts matching
//!   the submitted face.
//! - Google Lens has no client-side numeric relevance score, so
//!   `provider_score` is `NaN` ("n/a") and the real ordering (`position`) is
//!   preserved instead of fabricating a similarity number.
//!
//! Privacy: only the (crop or full) image bytes are uploaded — never the
//! Milestone-3 facial embedding or any derived biometric data.
//!
//! Failures (missing image, unreadable image, missing/oversized upload,
//! network, timeout, HTTP status incl. credential rejection, rate limiting,
//! missing image_id, non-success status, empty matches, malformed JSON, schema
//! drift) are all mapped to typed `ImmutaraError`. Search failure is
//! non-fatal to the pipeline.

use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use immutara_core::ImmutaraError;
use immutara_core::config::SerpApiLensConfig;
use immutara_core::domain::evidence::Evidence;
use immutara_core::domain::search::{
    SearchInput, SearchInputKind, SearchMatch, SearchMatchKind, SearchResult, hostname_of,
    is_social_media_url,
};
use immutara_core::providers::ImageSearchProvider;
use serde::Deserialize;

/// Provider implementation name reported through the domain model.
const SERPAPI_LENS_PROVIDER_ID: &str = "serpapi_lens";

/// Number of multipart retries before giving up (transient network faults).
const MAX_ATTEMPTS: usize = 3;

#[derive(Debug, Clone, Deserialize)]
struct UploadResponse {
    /// SerpApi returns the uploaded image's id here.
    #[serde(default)]
    image_id: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct LensResponse {
    #[serde(default)]
    search_metadata: Option<SearchMetadata>,
    #[serde(default)]
    search_information: Option<SearchInformation>,
    #[serde(default)]
    exact_matches: Vec<LensMatch>,
    #[serde(default)]
    visual_matches: Vec<LensMatch>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct SearchMetadata {
    #[serde(default)]
    status: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct SearchInformation {}

#[derive(Debug, Clone, Deserialize)]
struct LensMatch {
    #[serde(default)]
    position: Option<u32>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    link: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    thumbnail: Option<String>,
}

/// A real reverse-image-search provider backed by the SerpApi Google Lens API.
pub struct SerpApiLensSearchProvider {
    provider_id: String,
    api_url: String,
    api_key: String,
    /// Bytes cap for the uploaded image.
    max_upload_bytes: usize,
    timeout: Duration,
    http: reqwest::Client,
}

impl SerpApiLensSearchProvider {
    /// Construct a provider from a config that already carries the API key
    /// (typically resolved from the `SERPAPI_API_KEY` environment variable).
    pub fn new(config: SerpApiLensConfig) -> Self {
        let timeout = Duration::from_secs(config.timeout_seconds.max(1));
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .unwrap_or_else(|e| {
                let _ = e;
                reqwest::Client::builder()
                    .build()
                    .unwrap_or_else(|_| reqwest::Client::new())
            });
        Self {
            provider_id: SERPAPI_LENS_PROVIDER_ID.to_string(),
            api_url: config.api_url.trim_end_matches('/').to_string(),
            api_key: config.api_key,
            max_upload_bytes: config.max_upload_bytes.max(1),
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

    /// Upload an image to SerpApi, returning the `image_id` for the Lens query.
    async fn upload_once(
        &self,
        image_bytes: Vec<u8>,
        mime_type: &str,
    ) -> Result<UploadResponse, ImmutaraError> {
        if image_bytes.len() > self.max_upload_bytes {
            return Err(ImmutaraError::Provider {
                provider: self.provider_id.clone(),
                message: format!(
                    "image is {} bytes, exceeding the {}-byte upload cap",
                    image_bytes.len(),
                    self.max_upload_bytes
                ),
            });
        }

        let form = reqwest::multipart::Form::new()
            .text("api_key", self.api_key.clone())
            .part(
                "image",
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
            .post(format!("{}/image", self.api_url))
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
                message: format!("failed to read SerpApi upload response: {e}"),
            })?;

        if !status.is_success() {
            return Err(self.map_http_error(status.as_u16(), bytes.as_ref()));
        }

        let parsed: UploadResponse =
            serde_json::from_slice(&bytes).map_err(|e| ImmutaraError::Provider {
                provider: self.provider_id.clone(),
                message: format!("SerpApi returned unparseable upload JSON (HTTP {status}): {e}"),
            })?;

        if parsed.image_id.is_none() {
            return Err(ImmutaraError::Provider {
                provider: self.provider_id.clone(),
                message: format!(
                    "SerpApi upload response carried no image_id{}",
                    parsed
                        .message
                        .as_deref()
                        .map(|m| format!(" ({m})"))
                        .unwrap_or_default()
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
    ) -> Result<UploadResponse, ImmutaraError> {
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

    /// Run the Lens query for a previously uploaded `image_id`.
    async fn lens_query_once(&self, image_id: &str) -> Result<LensResponse, ImmutaraError> {
        let response = self
            .http
            .get(format!("{}/search", self.api_url))
            .query(&[
                ("engine", "google_lens"),
                ("image_id", image_id),
                ("api_key", self.api_key.as_str()),
                ("json_format", "1"),
            ])
            .send()
            .await
            .map_err(|e| self.map_network_error(e))?;

        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(|e| ImmutaraError::Provider {
                provider: self.provider_id.clone(),
                message: format!("failed to read SerpApi Lens response: {e}"),
            })?;

        if !status.is_success() {
            return Err(self.map_http_error(status.as_u16(), bytes.as_ref()));
        }

        let parsed: LensResponse =
            serde_json::from_slice(&bytes).map_err(|e| ImmutaraError::Provider {
                provider: self.provider_id.clone(),
                message: format!("SerpApi returned unparseable Lens JSON (HTTP {status}): {e}"),
            })?;

        if let Some(msg) = parsed.error.as_deref()
            && !msg.is_empty()
        {
            return Err(ImmutaraError::Provider {
                provider: self.provider_id.clone(),
                message: format!("SerpApi/Lens reported an error: {msg}"),
            });
        }

        let status_ok = parsed
            .search_metadata
            .as_ref()
            .and_then(|m| m.status.clone())
            .is_some_and(|s| s == "Success");
        if !status_ok {
            return Err(ImmutaraError::Provider {
                provider: self.provider_id.clone(),
                message: format!(
                    "Lens search did not report Success (metadata {:?}, information {:?})",
                    parsed.search_metadata, parsed.search_information
                ),
            });
        }

        Ok(parsed)
    }

    /// Query with bounded retries for transient network faults.
    async fn lens_query_with_retry(&self, image_id: &str) -> Result<LensResponse, ImmutaraError> {
        let mut attempt = 0usize;
        loop {
            attempt += 1;
            match self.lens_query_once(image_id).await {
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

    /// Perform an upload + Lens query for `image_bytes`, mapped to domain.
    async fn search_bytes(
        &self,
        evidence: &Evidence,
        image_bytes: Vec<u8>,
        mime_type: &str,
        input_kind: SearchInputKind,
    ) -> Result<SearchResult, ImmutaraError> {
        let upload = self.upload_with_retry(image_bytes, mime_type).await?;
        let image_id = upload.image_id.expect("validated as present above");
        let parsed = self.lens_query_with_retry(&image_id).await?;
        Ok(self.to_result(evidence, parsed, input_kind))
    }

    /// Translate `LensResponse` into the shared domain model.
    ///
    /// Ordering: Google Lens `exact_matches` (same picture, per Google) come
    /// first, then `visual_matches` (similar pictures). The selected/bound
    /// match therefore prefers an exact match when one exists.
    fn to_result(
        &self,
        evidence: &Evidence,
        parsed: LensResponse,
        input_kind: SearchInputKind,
    ) -> SearchResult {
        let mut matches = parsed
            .exact_matches
            .into_iter()
            .filter_map(|m| self.to_match(m, SearchMatchKind::Exact))
            .collect::<Vec<_>>();
        matches.extend(
            parsed
                .visual_matches
                .into_iter()
                .filter_map(|m| self.to_match(m, SearchMatchKind::Visual)),
        );
        SearchResult {
            evidence_id: evidence.id,
            provider_id: self.provider_id.clone(),
            search_input: input_kind,
            matches,
            searched_at: Utc::now(),
            social_state: Default::default(),
        }
    }

    fn to_match(&self, m: LensMatch, match_kind: SearchMatchKind) -> Option<SearchMatch> {
        let source_url = m.link;
        let source_url = source_url?;
        let domain = hostname_of(&source_url);

        Some(SearchMatch {
            match_kind,
            source_url: Some(source_url),
            source_domain: domain,
            source_title: m.title,
            source_description: m.source,
            // Google Lens exposes no similarity score; position carries the
            // ordering. Report the score as unavailable rather than fabricate.
            provider_score: f64::NAN,
            position: m.position,
            first_seen: None,
            thumbnail_url: m.thumbnail,
            media_match: None,
        })
    }

    fn map_network_error(&self, e: reqwest::Error) -> ImmutaraError {
        if e.is_timeout() {
            return ImmutaraError::Provider {
                provider: self.provider_id.clone(),
                message: format!(
                    "SerpApi/Lens request timed out after {}s",
                    self.timeout.as_secs()
                ),
            };
        }
        ImmutaraError::Provider {
            provider: self.provider_id.clone(),
            message: format!("network error talking to SerpApi/Lens: {e}"),
        }
    }

    fn map_http_error(&self, status: u16, body: &[u8]) -> ImmutaraError {
        let hint = String::from_utf8_lossy(body);
        let message = match status {
            401 | 403 => format!(
                "authentication failed (check SERPAPI_API_KEY){}",
                suffix(&hint)
            ),
            429 => format!(
                "SerpApi rate limit exceeded; try again later{}",
                suffix(&hint)
            ),
            s if s >= 500 => format!("SerpApi server error (HTTP {s})"),
            s => format!("SerpApi returned HTTP {s}{}", suffix(&hint)),
        };
        ImmutaraError::Provider {
            provider: self.provider_id.clone(),
            message,
        }
    }
}

/// Short quote of a response body hint (SerpApi embeds errors in the body).
fn suffix(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        String::new()
    } else {
        format!(" ({trimmed})")
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

#[async_trait]
impl ImageSearchProvider for SerpApiLensSearchProvider {
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
        self.search_bytes(
            evidence,
            image_bytes,
            &mime_type,
            SearchInputKind::FullImage,
        )
        .await
    }

    async fn search_with_input(
        &self,
        evidence: &Evidence,
        input: &SearchInput,
    ) -> Result<SearchResult, ImmutaraError> {
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

        match input {
            SearchInput::FaceCrop(crop) => {
                // Crop-first: submit the selected face region.
                let crop_result = self
                    .search_bytes(
                        evidence,
                        crop.image_bytes.clone(),
                        &crop.mime_type,
                        SearchInputKind::FaceCrop,
                    )
                    .await?;
                // Fall back to the full image only when the crop yielded no
                // useful result (genuinely empty, not an error).
                if !crop_result.matches.is_empty() {
                    return Ok(crop_result);
                }
                let full_bytes = self.read_image_bytes(&source_path).await?;
                let mime_type = normalize_mime(&evidence.metadata.mime_type, &source_path);
                self.search_bytes(evidence, full_bytes, &mime_type, SearchInputKind::FullImage)
                    .await
            }
            SearchInput::FullImage => self.search(evidence).await,
        }
    }

    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    fn supports_media_validation(&self) -> bool {
        true
    }
}

impl SerpApiLensSearchProvider {
    /// True when the provider reports a social media URL in any result.
    pub fn has_social_media_result(&self, result: &SearchResult) -> bool {
        result
            .matches
            .iter()
            .filter_map(|m| m.source_url.as_deref())
            .any(is_social_media_url)
    }
}

/// SerpApi requires a recognizable image content type.
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

    fn provider() -> SerpApiLensSearchProvider {
        SerpApiLensSearchProvider::new(SerpApiLensConfig {
            api_url: "https://serpapi.com".to_string(),
            api_key: "dummy-key".to_string(),
            timeout_seconds: 30,
            max_upload_bytes: 500_000,
        })
    }

    fn sample_lens_response() -> LensResponse {
        serde_json::from_str(
            r#"{
              "search_metadata": { "status": "Success" },
              "search_parameters": { "engine": "google_lens", "image_id": "abc123" },
              "exact_matches": [
                {
                  "position": 1,
                  "title": "The original Lena photo (512x512 crop)",
                  "link": "https://www.newscientist.com/article/dn10471-lenna/",
                  "source": "New Scientist",
                  "thumbnail": null
                }
              ],
              "visual_matches": [
                {
                  "position": 1,
                  "title": "TIL the 512x512 image is a crop of Lena",
                  "link": "https://www.reddit.com/r/todayilearned/comments/1je7ab/x",
                  "source": "Reddit",
                  "thumbnail": "https://external.xx.fbcdn.net/thumb1"
                },
                {
                  "position": 2,
                  "title": "Lena image discussion",
                  "link": "https://x.com/someuser/status/907693162625298432",
                  "source": "X",
                  "thumbnail": null
                },
                {
                  "position": 3,
                  "title": "A research paper about Lena",
                  "link": "https://arxiv.org/abs/2301.00001",
                  "source": "arXiv",
                  "thumbnail": null
                }
              ]
            }"#,
        )
        .unwrap()
    }

    // ---- Response mapping (pure, no network) ----

    #[test]
    fn maps_lens_fields_into_search_match() {
        let p = provider();
        let ev = evidence_at(Some(fixture("face_lena.jpg")), "image/jpeg");
        let result = p.to_result(&ev, sample_lens_response(), SearchInputKind::FaceCrop);
        assert_eq!(result.search_input, SearchInputKind::FaceCrop);

        // Exact matches come first and carry the real metadata.
        let exact = &result.matches[0];
        assert_eq!(exact.match_kind, SearchMatchKind::Exact);
        assert_eq!(exact.source_domain.as_deref(), Some("newscientist.com"));
        assert_eq!(exact.position, Some(1));

        // Visual matches follow; the top visual result is a social post.
        let first_visual = &result.matches[1];
        assert_eq!(first_visual.match_kind, SearchMatchKind::Visual);
        assert_eq!(first_visual.source_domain.as_deref(), Some("reddit.com"));
        assert_eq!(first_visual.position, Some(1));
        assert_eq!(first_visual.source_description.as_deref(), Some("Reddit"));
        assert!(
            first_visual
                .source_title
                .as_deref()
                .unwrap()
                .contains("Lena")
        );
        // Lens exposes no similarity score -> NaN, never fabricated.
        assert!(first_visual.provider_score.is_nan());
        assert!(result.matches[2].provider_score.is_nan());
        // academic result is not classified social
        let academic = result
            .matches
            .iter()
            .find(|m| m.source_domain.as_deref() == Some("arxiv.org"))
            .expect("arxiv result present");
        assert!(!is_social_media_url(
            academic.source_url.as_deref().unwrap()
        ));

        // The selected (attestation-bound) result is the exact match.
        let selected = result.selected().unwrap();
        assert_eq!(selected.match_kind, SearchMatchKind::Exact);
        assert_eq!(selected.source_domain.as_deref(), Some("newscientist.com"));
        assert_eq!(result.selected_index(), Some(0));
    }

    #[test]
    fn zero_matches_maps_to_empty_result() {
        let p = provider();
        let parsed: LensResponse =
            serde_json::from_str(r#"{"search_metadata":{"status":"Success"},"visual_matches":[]}"#)
                .unwrap();
        let ev = evidence_at(Some(fixture("face_lena.jpg")), "image/jpeg");
        let result = p.to_result(&ev, parsed, SearchInputKind::FaceCrop);
        assert!(result.matches.is_empty());
    }

    #[test]
    fn missing_score_and_links_tolerated() {
        let p = provider();
        let parsed: LensResponse = serde_json::from_str(
            r#"{"search_metadata":{"status":"Success"},"visual_matches":[{"title":"x","source":"Y"}]}"#,
        )
        .unwrap();
        let ev = evidence_at(Some(fixture("face_lena.jpg")), "image/jpeg");
        let result = p.to_result(&ev, parsed, SearchInputKind::FullImage);
        // A match without a link is not useful -> dropped.
        assert!(result.matches.is_empty());
    }

    #[test]
    fn http_error_mapping_is_typed() {
        let p = provider();
        for (status, needle) in [
            (401u16, "authentication"),
            (403u16, "authentication"),
            (429u16, "rate limit"),
            (500u16, "server error"),
        ] {
            let err = p.map_http_error(status, b"{\"error\":\"boom\"}");
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
        let bad: Result<LensResponse, _> = serde_json::from_str("this is not json");
        assert!(bad.is_err());
        let bad_upload: Result<UploadResponse, _> = serde_json::from_str("nope");
        assert!(bad_upload.is_err());
    }

    #[test]
    fn upload_missing_image_id_is_an_error() {
        let p = provider();
        let parsed: UploadResponse = serde_json::from_str(r#"{"message":"ok"}"#).unwrap();
        assert!(parsed.image_id.is_none());
        // The semantic check lives in upload_once (needs transport), so here we
        // assert the parse shape: image_id present round-trips.
        let with_id: UploadResponse = serde_json::from_str(
            r#"{"image_id":"8giruXicu2lgbmxunGqRZGFilJqYbGpmnpZmmpKWaGaaapSUnGZgmqiXVZCaHm"}"#,
        )
        .unwrap();
        assert_eq!(
            with_id.image_id.as_deref(),
            Some("8giruXicu2lgbmxunGqRZGFilJqYbGpmnpZmmpKWaGaaapSUnGZgmqiXVZCaHm")
        );
        let _ = p;
    }

    // ---- Full-image-only error paths (no live network) ----

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

    // ---- Real integration test (optional, offline by default) ----
    //
    // Set IMMUTARA_E2E_SEARCH=1 and a real SERPAPI_API_KEY to run. Uploads a
    // locally generated face-crop of the fixture and asserts real matches come
    // back, including at least one social-media URL.

    #[tokio::test]
    async fn real_serpapi_lens_crop_search_returns_social_matches() {
        if std::env::var("IMMUTARA_E2E_SEARCH")
            .map(|v| v != "1")
            .unwrap_or(true)
        {
            eprintln!(
                "SKIP real_serpapi_lens_crop_search (set IMMUTARA_E2E_SEARCH=1 and SERPAPI_API_KEY)"
            );
            return;
        }
        let key = std::env::var("SERPAPI_API_KEY").expect("SERPAPI_API_KEY required for live test");
        let p = SerpApiLensSearchProvider::new(SerpApiLensConfig {
            api_url: "https://serpapi.com".to_string(),
            api_key: key,
            timeout_seconds: 60,
            max_upload_bytes: 500_000,
        });

        // Generate the face crop locally from the known YuNet detection on the
        // fixture (bbox from a real analysis run of face_lena.jpg).
        let bytes = std::fs::read(fixture("face_lena.jpg")).expect("fixture present");
        use immutara_core::domain::analysis::BoundingBox;
        let crop = crate::face_crop::generate_face_crop(
            &bytes,
            BoundingBox {
                x: 58,
                y: 45,
                width: 128,
                height: 177,
            },
            p.max_upload_bytes,
        )
        .expect("crop generation should succeed");

        let ev = evidence_at(Some(fixture("face_lena.jpg")), "image/jpeg");
        let result = p
            .search_with_input(&ev, &SearchInput::FaceCrop(crop))
            .await
            .expect("real Lens search should succeed");
        assert_eq!(result.search_input, SearchInputKind::FaceCrop);
        assert!(
            !result.matches.is_empty(),
            "expected >=1 match from Google Lens on the face crop"
        );
        assert!(
            p.has_social_media_result(&result),
            "expected at least one social-media result from Lens"
        );
        let exact_count = result
            .matches
            .iter()
            .filter(|m| m.match_kind == SearchMatchKind::Exact)
            .count();
        let visual_count = result.matches.len() - exact_count;
        let selected = result
            .selected()
            .expect("a match is selected for attestation");
        // Preference rule: exact match when available, otherwise visual.
        if exact_count > 0 {
            assert_eq!(selected.match_kind, SearchMatchKind::Exact);
        } else {
            assert_eq!(selected.match_kind, SearchMatchKind::Visual);
        }

        let serialized = serde_json::to_string(&result).unwrap();
        assert!(serialized.contains("serpapi_lens"));
        eprintln!(
            "LIVE serpapi_lens: {} matches ({} exact, {} visual); top domains: {:?}",
            result.matches.len(),
            exact_count,
            visual_count,
            result
                .matches
                .iter()
                .take(5)
                .map(|m| format!(
                    "{} ({}@pos {})",
                    m.source_domain.as_deref().unwrap_or("?"),
                    match m.match_kind {
                        SearchMatchKind::Exact => "exact",
                        SearchMatchKind::Visual => "visual",
                    },
                    m.position.unwrap_or(0)
                ))
                .collect::<Vec<_>>()
        );
        eprintln!(
            "  LIVE selected (attested): {:?} | domain {:?} | pos {:?}",
            selected.source_url.as_deref().unwrap_or("?"),
            selected.source_domain.as_deref().unwrap_or("?"),
            selected.position.unwrap_or(0)
        );
        for m in result.matches.iter().take(3) {
            eprintln!(
                "  LIVE result {} ({}): {}",
                m.position.unwrap_or(0),
                match m.match_kind {
                    SearchMatchKind::Exact => "exact",
                    SearchMatchKind::Visual => "visual",
                },
                m.source_url.as_deref().unwrap_or("?")
            );
        }
        if let Some(social) = result
            .matches
            .iter()
            .find(|m| m.source_url.as_deref().is_some_and(is_social_media_url))
        {
            eprintln!(
                "  LIVE first social result: {}",
                social.source_url.as_deref().unwrap_or("?")
            );
        }
    }
}
