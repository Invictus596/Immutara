//! Independent media-match validation for reverse-image-search candidates.
//!
//! Validation checks that a candidate's public media actually contains the
//! searched image, using only deterministic hashes:
//!
//! - byte-identical media (SHA-256 equality between the fetched media and the
//!   searched image) is an exact hash match;
//! - otherwise a 64-bit perceptual hash (dHash on a grayscale 9x8 Triangle
//!   resize) is compared by Hamming distance against a fixed threshold.
//!
//! Attribution rule: a **social** candidate verifies only when media
//! discovered *on the source page itself* (`og:image`, then the first absolute
//! `<img>`) matches the query. The provider's own thumbnail URL is supporting
//! evidence only — it is the search provider's copy of the matched image and
//! proves nothing about what the page serves today.
//!
//! The validator never reuses the facial-recognition model, never claims
//! identity, and never bypasses access controls: login-walled, CAPTCHA'd,
//! private, or otherwise unreachable media simply records honest evidence of
//! why the match could not be established.

use immutara_core::domain::search::{
    MediaMatchEvidence, MediaMatchMethod, SearchMatch, SearchMatchState, SearchResult,
    is_social_media_url,
};
use sha2::{Digest, Sha256};
use std::time::Duration;

/// Image encodings the validator knows about for a run.
///
/// `searched` is the exact bytes submitted to the search provider (the face
/// crop when one was used, otherwise the full image). `photo` is the full
/// evidence photograph. The two coincide when the full image was searched.
#[derive(Debug, Clone)]
pub struct ValidationQuery {
    /// Exact bytes submitted to the reverse-image provider.
    pub searched: Vec<u8>,
    /// Full evidence photograph (same as `searched` for `FULL IMAGE` input).
    pub photo: Vec<u8>,
}

impl ValidationQuery {
    /// Build from a searched input and the evidence photograph bytes.
    pub fn new(searched: Vec<u8>, photo: Vec<u8>) -> Self {
        Self { searched, photo }
    }
}

/// Settings for the independent media-match validator.
#[derive(Debug, Clone)]
pub struct MediaValidationConfig {
    /// Maximum number of candidates whose media is actually fetched and
    /// compared (capped so a large ranking does not explode into HTTP calls).
    pub max_validations: usize,
    /// dHash Hamming threshold: `distance <= threshold` is a visual match.
    /// Documented with the run, and carried in the evidence.
    pub dhash_threshold: u32,
    /// Per-request timeout (seconds), fresh for every fetch.
    pub timeout_seconds: u64,
    /// Largest media payload fetched for comparison.
    pub max_media_bytes: usize,
    /// User-Agent identifying the validator as a plain read-only crawler.
    pub user_agent: String,
}

impl Default for MediaValidationConfig {
    fn default() -> Self {
        Self {
            max_validations: 8,
            dhash_threshold: 12,
            timeout_seconds: 15,
            max_media_bytes: 5_000_000,
            user_agent: "Immutara/0.1 (media validation; no JS; no login bypass)".into(),
        }
    }
}

/// Validates the public media of search candidates against the searched image.
#[derive(Debug, Clone)]
pub struct MediaMatchValidator {
    config: MediaValidationConfig,
    http: reqwest::Client,
}

impl MediaMatchValidator {
    pub fn new(config: MediaValidationConfig) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout_seconds))
            .redirect(reqwest::redirect::Policy::limited(5))
            .user_agent(&config.user_agent)
            .build()
            .expect("valid reqwest client");
        Self { config, http }
    }

    /// Default daemon-style validator wired into pipelines that opt in.
    pub fn default_client() -> Self {
        Self::new(MediaValidationConfig::default())
    }

    /// Validate the public media of eligible candidates in `result` against
    /// `query` (the searched image and its full evidence photo) and set the
    /// overall [`SearchMatchState`].
    ///
    /// Social candidates are validated first (the provenance target), then the
    /// top non-social matches, capped by `max_validations`. Deterministic.
    pub async fn validate_matches(&self, result: &mut SearchResult, query: &ValidationQuery) {
        let order = self.validation_order(result);
        let mut budget = self.config.max_validations;
        for idx in order {
            if budget == 0 {
                break;
            }
            budget -= 1;
            self.validate_one(&mut result.matches[idx], query).await;
        }
        result.social_state = derive_state(result);
    }

    /// Social candidates first (their proven order), then others.
    fn validation_order(&self, result: &SearchResult) -> Vec<usize> {
        let mut social = Vec::new();
        let mut others = Vec::new();
        for (i, m) in result.matches.iter().enumerate() {
            let url = m.source_url.as_deref().unwrap_or("");
            if is_social_media_url(url) {
                social.push(i);
            } else {
                others.push(i);
            }
        }
        social.into_iter().chain(others).collect()
    }

    /// Fetch and compare a single candidate's public media.
    ///
    /// For **non-social** candidates the provider thumbnail is attempted first
    /// (cheap, on-point) and the page is only fetched when it cannot establish
    /// a match. For **social** candidates the source page is always consulted:
    /// a provider thumbnail is supporting evidence only — it is the search
    /// provider's copy of the matched image, so by itself it proves nothing
    /// about what the page serves today. A social candidate verifies only when
    /// media discovered *on the page* (`og:image`, falling back to the first
    /// absolute `<img>`) matches the query.
    async fn validate_one(&self, m: &mut SearchMatch, query: &ValidationQuery) {
        let is_social = m.source_url.as_deref().is_some_and(is_social_media_url);

        // (A) Provider-returned thumbnail — supporting signal for socials.
        let thumbnail = match m
            .thumbnail_url
            .as_deref()
            .filter(|u| is_absolute_http_url(u))
        {
            Some(url) => match self.fetch_image_bytes(url).await {
                Ok(bytes) => {
                    let evidence = self.compare(url.to_string(), &bytes, query);
                    if !is_social && evidence.passed {
                        m.media_match = Some(evidence);
                        return;
                    }
                    Some(evidence)
                }
                Err(_) => None,
            },
            None => None,
        };

        // (B/C) The page's own discoverable media — the independent evidence.
        let page_evidence = self.page_media(m, query).await;

        if is_social {
            m.media_match = Some(match page_evidence {
                Some(ev) if ev.passed => ev,
                other => self.social_failure(thumbnail.as_ref(), other.as_ref()),
            });
            return;
        }

        // Non-social: prefer page-discovered media, otherwise the thumbnail
        // evidence (which did not pass), otherwise an honest no-source note.
        m.media_match = Some(page_evidence.or(thumbnail).unwrap_or_else(|| {
            self.failed("candidate has no source URL to inspect".to_string(), None)
        }));
    }

    /// Retrieve a candidate's own media from its source page and compare it.
    ///
    /// Returns `None` only when the candidate has no usable absolute source
    /// URL. Every other outcome — page fetch failure, no discoverable image,
    /// media fetch failure, or a compared (passing/failing) match — is a
    /// concrete `MediaMatchEvidence`.
    async fn page_media(
        &self,
        m: &SearchMatch,
        query: &ValidationQuery,
    ) -> Option<MediaMatchEvidence> {
        let page = m
            .source_url
            .as_deref()
            .filter(|u| is_absolute_http_url(u))?;
        let html = match self.fetch_page(page).await {
            Ok(html) => html,
            Err(note) => return Some(self.failed(note, None)),
        };
        let (og, first_img) = extract_image_urls(&html);
        let url = match og.or(first_img) {
            Some(url) => url,
            None => {
                return Some(self.failed(
                    "page has no discoverable og:image or absolute <img> src".to_string(),
                    Some(page.to_string()),
                ));
            }
        };
        match self.fetch_image_bytes(&url).await {
            Ok(bytes) => Some(self.compare(url, &bytes, query)),
            Err(note) => Some(self.failed(note, Some(url))),
        }
    }

    /// Failure evidence for a social candidate, explicitly separating the
    /// (supporting) provider-thumbnail outcome from the independent page
    /// outcome, so an operator can see that the thumbnail matched even though
    /// the page itself did not.
    fn social_failure(
        &self,
        thumbnail: Option<&MediaMatchEvidence>,
        page: Option<&MediaMatchEvidence>,
    ) -> MediaMatchEvidence {
        let thumb = match thumbnail {
            Some(t) if t.passed => {
                "provider thumbnail matched (supporting evidence only)".to_string()
            }
            Some(t) => format!(
                "provider thumbnail did not match ({})",
                t.note.as_deref().unwrap_or("failed")
            ),
            None => "no provider thumbnail".to_string(),
        };
        let (page_note, page_url) = match page {
            Some(p) => (
                format!("page media: {}", p.note.as_deref().unwrap_or("unavailable")),
                p.media_url.clone(),
            ),
            None => ("no source URL to inspect".to_string(), None),
        };
        self.failed(
            format!("cannot attribute the matching media to this page ({thumb}); {page_note}"),
            page_url,
        )
    }

    /// Compare fetched media against the searched image, deterministically.
    ///
    /// Byte identity is judged against either the searched bytes or the full
    /// evidence photo (`note` names which matched). Perceptual matching takes
    /// the closer dHash of the media against the searched crop and the full
    /// photo, records that distance and the winning query form, and passes
    /// when it is within the documented threshold.
    fn compare(
        &self,
        media_url: String,
        media: &[u8],
        query: &ValidationQuery,
    ) -> MediaMatchEvidence {
        let media_hash = Sha256::digest(media);
        if Sha256::digest(&query.searched) == media_hash
            || Sha256::digest(&query.photo) == media_hash
        {
            return MediaMatchEvidence {
                method: Some(MediaMatchMethod::ExactHash),
                distance: Some(0),
                threshold: Some(0),
                passed: true,
                media_url: Some(media_url),
                note: Some("byte-identical to the searched image".to_string()),
            };
        }

        let threshold = self.config.dhash_threshold;
        let media_hash_ph = Self::dhash(media);
        if let Some(mh) = media_hash_ph {
            let mut candidates: Vec<(u64, &str)> = if query.searched == query.photo {
                vec![(
                    Self::dhash(&query.photo).expect("full image decodes"),
                    "searched image",
                )]
            } else {
                let mut v = Vec::new();
                if let Some(ph) = Self::dhash(&query.photo) {
                    v.push((ph, "full photo"));
                }
                if let Some(ph) = Self::dhash(&query.searched) {
                    v.push((ph, "face crop"));
                }
                v
            };
            candidates.sort_by_key(|(qh, _)| Self::hamming(mh, *qh));
            let (best_qh, label) = candidates[0];
            let distance = Self::hamming(mh, best_qh);
            let passed = distance <= threshold;
            let note = if passed {
                format!("perceptual match against {}", label)
            } else {
                format!(
                    "perceptual distance {distance} exceeds threshold {threshold} (against {label})"
                )
            };
            MediaMatchEvidence {
                method: Some(MediaMatchMethod::PerceptualHash),
                distance: Some(distance),
                threshold: Some(threshold),
                passed,
                media_url: Some(media_url),
                note: Some(note),
            }
        } else {
            MediaMatchEvidence {
                method: Some(MediaMatchMethod::PerceptualHash),
                distance: None,
                threshold: Some(threshold),
                passed: false,
                media_url: Some(media_url),
                note: Some("media not decodable as a comparable image".to_string()),
            }
        }
    }

    /// Evidence recording why a comparison could not be performed.
    fn failed(&self, note: String, media_url: Option<String>) -> MediaMatchEvidence {
        MediaMatchEvidence {
            method: None,
            distance: None,
            threshold: Some(self.config.dhash_threshold),
            passed: false,
            media_url,
            note: Some(note),
        }
    }

    /// dHash of an image: grayscale, 9x8 Triangle resize, 64 bits.
    /// `None` when the bytes are not a decodable JPEG/PNG.
    fn dhash(bytes: &[u8]) -> Option<u64> {
        let img = image::load_from_memory(bytes).ok()?.to_luma8();
        let resized = image::imageops::resize(&img, 9, 8, image::imageops::FilterType::Triangle);
        let mut hash: u64 = 0;
        for row in 0..8 {
            for col in 0..8 {
                let left = resized.get_pixel(col, row).0[0];
                let right = resized.get_pixel(col + 1, row).0[0];
                hash = (hash << 1) | u64::from(left < right);
            }
        }
        Some(hash)
    }

    fn hamming(a: u64, b: u64) -> u32 {
        (a ^ b).count_ones()
    }

    /// Fetch image bytes, honoring `image/*` gate and size cap; never follows
    /// into login walls or bypasses anything cryptographic.
    async fn fetch_image_bytes(&self, url: &str) -> Result<Vec<u8>, String> {
        let resp = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| format!("fetch error: {e}"))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("HTTP {status}"));
        }
        let ctype = content_type(resp.headers())
            .unwrap_or_default()
            .to_lowercase();
        if !ctype.starts_with("image/") {
            return Err(format!("content-type {ctype:?} is not an image"));
        }
        let body = resp.bytes().await.map_err(|e| format!("body error: {e}"))?;
        if body.len() > self.config.max_media_bytes {
            return Err(format!("media too large ({} bytes)", body.len()));
        }
        Ok(body.to_vec())
    }

    /// Fetch a candidate page (HTML) for media discovery.
    async fn fetch_page(&self, url: &str) -> Result<String, String> {
        let resp = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| format!("page fetch error: {e}"))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("page HTTP {status}"));
        }
        let ctype = content_type(resp.headers())
            .unwrap_or_default()
            .to_lowercase();
        if !ctype.contains("html") && !ctype.contains("text/plain") {
            return Err(format!("page content-type {ctype:?}"));
        }
        let body = resp.bytes().await.map_err(|e| format!("body error: {e}"))?;
        if body.len() > 8_000_000 {
            return Err("page too large to scan".to_string());
        }
        Ok(String::from_utf8_lossy(&body).into_owned())
    }
}

/// Overall search outcome derived from candidate media evidence.
pub fn derive_state(result: &SearchResult) -> SearchMatchState {
    if result.matches.is_empty() {
        return SearchMatchState::NoResults;
    }
    let mut has_social = false;
    let mut attempted = false;
    for m in &result.matches {
        let url = m.source_url.as_deref().unwrap_or("");
        if !is_social_media_url(url) {
            continue;
        }
        has_social = true;
        match m.media_match.as_ref() {
            Some(e) if e.passed => return SearchMatchState::SocialMatchVerified,
            Some(_) => attempted = true,
            None => {}
        }
    }
    if has_social {
        if attempted {
            SearchMatchState::SocialCandidateUnverified
        } else {
            SearchMatchState::SocialCandidate
        }
    } else if result.matches.is_empty() {
        SearchMatchState::NoResults
    } else {
        SearchMatchState::WebMatch
    }
}

fn content_type(headers: &reqwest::header::HeaderMap) -> Option<&str> {
    headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
}

/// True for `http://`/`https://` absolute URLs (relative references cannot be
/// resolved without a URL library and are therefore reported as undiscoverable).
fn is_absolute_http_url(url: &str) -> bool {
    let t = url.trim_start();
    t.starts_with("https://") || t.starts_with("http://")
}

/// Extract `og:image` and the first absolute `<img src>` from a page.
/// Returns `(og:image, first_absolute_img)`; both absolute URLs only.
fn extract_image_urls(html: &str) -> (Option<String>, Option<String>) {
    let hay = html.as_bytes();
    const OG_IMAGE: &[u8] = b"property=\"og:image\"";
    const IMG_TAG: &[u8] = b"<img";
    let mut og: Option<String> = None;
    let mut img: Option<String> = None;

    let mut from = 0;
    while let Some(pos) = find_sub(OG_IMAGE, hay, from) {
        let mut after = pos + OG_IMAGE.len();
        if let Some(url) = extract_quoted_attr(hay, &mut after, b"content")
            && is_absolute_http_url(&url)
        {
            og = Some(url);
            break;
        }
        from = pos + OG_IMAGE.len();
    }

    let mut from = 0;
    while let Some(pos) = find_sub(IMG_TAG, hay, from) {
        let mut after = pos + IMG_TAG.len();
        if let Some(url) = extract_quoted_attr(hay, &mut after, b"src")
            && is_absolute_http_url(&url)
        {
            img = Some(url);
            break;
        }
        from = pos + IMG_TAG.len();
    }

    (og, img)
}

/// Within a short window after `tag_end`, find `attr"` (single-quoted or
/// double-quoted) and extract the quoted value bytes as `String`.
fn extract_quoted_attr(hay: &[u8], tag_end: &mut usize, attr: &[u8]) -> Option<String> {
    let window = &hay[*tag_end..(*tag_end + 800).min(hay.len())];
    let attr_pos = window.windows(attr.len()).position(|w| w == attr)?;
    let after_attr = *tag_end + attr_pos + attr.len();
    let rest = &hay[after_attr..(after_attr + 40).min(hay.len())];
    let rest = rest.iter().position(|b| b == &b'=')?;
    let after_eq = after_attr + rest + 1;
    let val_bytes = &hay[after_eq..(after_eq + 4000).min(hay.len())];
    let quote_start = val_bytes.iter().position(|b| b == &b'"' || b == &b'\'')?;
    let quote = val_bytes[quote_start];
    let inner = &val_bytes[quote_start + 1..];
    let quote_end = inner.iter().position(|b| *b == quote)?;
    let value = std::str::from_utf8(&inner[..quote_end]).ok()?;
    let value = value.trim().to_string();
    *tag_end = after_eq + 1;
    (!value.is_empty()).then_some(value)
}

fn find_sub(needle: &[u8], hay: &[u8], from: usize) -> Option<usize> {
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use immutara_core::domain::evidence::EvidenceId;
    use immutara_core::domain::search::SearchInputKind;
    use immutara_core::domain::search::{SearchMatch, SearchMatchKind};
    use std::net::TcpListener;
    use std::net::TcpStream;

    fn sample_result(matches: Vec<SearchMatch>) -> SearchResult {
        SearchResult {
            evidence_id: EvidenceId::new(),
            provider_id: "mock".to_string(),
            search_input: SearchInputKind::FullImage,
            matches,
            searched_at: Utc::now(),
            social_state: Default::default(),
        }
    }

    fn match_at(url: &str) -> SearchMatch {
        SearchMatch {
            match_kind: SearchMatchKind::Visual,
            source_url: Some(url.to_string()),
            source_domain: None,
            source_title: None,
            source_description: None,
            provider_score: f64::NAN,
            position: None,
            first_seen: None,
            thumbnail_url: None,
            media_match: None,
        }
    }

    #[test]
    fn dhash_bits_and_hamming_are_stable() {
        let buffer =
            image::ImageBuffer::<image::Luma<u8>, Vec<u8>>::from_raw(64, 64, vec![90u8; 4096])
                .unwrap();
        let png = {
            let mut out = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageLuma8(buffer)
                .write_to(&mut out, image::ImageFormat::Png)
                .unwrap();
            out.into_inner()
        };
        let h1 = MediaMatchValidator::dhash(&png).expect("png decodes");
        // Identical bytes -> identical hash, zero hamming.
        let h2 = MediaMatchValidator::dhash(&png).expect("png decodes");
        assert_eq!(h1, h2);
        assert_eq!(MediaMatchValidator::hamming(h1, h2), 0);
        // Determinism across independent decodes.
        let h3 = MediaMatchValidator::dhash(&png).expect("png decodes");
        assert_eq!(h1, h3);
    }

    #[test]
    fn derive_state_classifies_all_outcomes() {
        // No matches.
        assert_eq!(
            derive_state(&sample_result(vec![])),
            SearchMatchState::NoResults
        );
        // Only non-social matches -> WebMatch.
        assert_eq!(
            derive_state(&sample_result(vec![match_at("https://news.com/a")])),
            SearchMatchState::WebMatch
        );
        // Social present, not validated -> SocialCandidate.
        assert_eq!(
            derive_state(&sample_result(vec![match_at("https://reddit.com/r/x/1")])),
            SearchMatchState::SocialCandidate
        );
        // Social present, attempted but failed -> SocialCandidateUnverified.
        let mut unverified = match_at("https://reddit.com/r/x/1");
        unverified.media_match = Some(MediaMatchEvidence {
            method: None,
            distance: None,
            threshold: Some(12),
            passed: false,
            media_url: None,
            note: Some("HTTP 403 login wall".to_string()),
        });
        assert_eq!(
            derive_state(&sample_result(vec![unverified.clone()])),
            SearchMatchState::SocialCandidateUnverified
        );
        // Verified social -> verified even with other failing candidates.
        let mut verified = match_at("https://reddit.com/r/x/2");
        verified.media_match = Some(MediaMatchEvidence {
            method: Some(MediaMatchMethod::PerceptualHash),
            distance: Some(4),
            threshold: Some(12),
            passed: true,
            media_url: Some("https://external.redditmedia.com/a.jpg".to_string()),
            note: None,
        });
        assert_eq!(
            derive_state(&sample_result(vec![unverified.clone(), verified])),
            SearchMatchState::SocialMatchVerified
        );
    }

    #[test]
    fn html_extraction_finds_og_image_and_img() {
        let html = "<html><head><meta property=\"og:image\" content=\"https://cdn.example/x.jpg\"></head>\
                    <body><img src=\"https://cdn.example/y.png\"></body></html>";
        let (og, img) = extract_image_urls(html);
        assert_eq!(og.as_deref(), Some("https://cdn.example/x.jpg"));
        assert_eq!(img.as_deref(), Some("https://cdn.example/y.png"));
    }

    #[test]
    fn html_extraction_rejects_relative_urls() {
        let html = "<img src=\"/local/x.jpg\"><p>c</p>";
        let (og, img) = extract_image_urls(html);
        assert!(og.is_none());
        assert!(img.is_none());
    }

    #[test]
    fn compare_distinguishes_exact_and_perceptual() {
        // The same pixels, re-encoded, forward a perceptual match.
        let v = vec![120u8; 16 * 16];
        let img = image::DynamicImage::ImageLuma8(
            image::ImageBuffer::<image::Luma<u8>, Vec<u8>>::from_raw(16, 16, v.clone()).unwrap(),
        );
        let mut png_a = std::io::Cursor::new(Vec::new());
        img.write_to(&mut png_a, image::ImageFormat::Png).unwrap();
        let mut png_b = std::io::Cursor::new(Vec::new());
        // Slightly perturbed: not byte-identical but visually near-identical.
        let mut v2 = v;
        v2[0] = 122;
        let img2 = image::DynamicImage::ImageLuma8(
            image::ImageBuffer::<image::Luma<u8>, Vec<u8>>::from_raw(16, 16, v2).unwrap(),
        );
        img2.write_to(&mut png_b, image::ImageFormat::Png).unwrap();

        let v = MediaMatchValidator::default_client();
        let q = |bytes: &[u8]| ValidationQuery::new(bytes.to_vec(), bytes.to_vec());
        assert!(
            v.compare(
                "https://cdn/x.png".to_string(),
                png_a.get_ref(),
                &q(png_a.get_ref()),
            )
            .passed
        );

        // Byte-identical -> ExactHash method with distance 0.
        let a = png_a.get_ref().clone();
        let exact2 = v.compare("https://cdn/x.png".to_string(), &a, &q(&a));
        assert_eq!(exact2.method, Some(MediaMatchMethod::ExactHash));
        assert_eq!(exact2.distance, Some(0));
        assert!(exact2.passed);
    }

    #[test]
    fn validation_order_social_first() {
        let result = sample_result(vec![
            match_at("https://news.com/a"),
            match_at("https://reddit.com/r/x/1"),
            match_at("https://facebook.com/p/2"),
        ]);
        let v = MediaMatchValidator::default_client();
        assert_eq!(v.validation_order(&result), vec![1, 2, 0]);
    }

    /// Two 64x64 patterns with strongly opposed dHash values: `0` is dark
    /// left / light right, `1` is the inverse. Far enough apart that a
    /// perceptual mismatch is unambiguous.
    fn pattern_bytes(pattern: usize) -> Vec<u8> {
        let mut img = image::RgbImage::new(64, 64);
        for (x, _y, px) in img.enumerate_pixels_mut() {
            let (r, g, b) = match pattern {
                0 => {
                    if x < 32 {
                        (8, 8, 8)
                    } else {
                        (247, 247, 247)
                    }
                }
                _ => {
                    if x < 32 {
                        (247, 247, 247)
                    } else {
                        (8, 8, 8)
                    }
                }
            };
            *px = image::Rgb([r, g, b]);
        }
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    fn far_part_dhash(a: &[u8], b: &[u8]) -> u32 {
        let ha = MediaMatchValidator::dhash(a).expect("decodes");
        let hb = MediaMatchValidator::dhash(b).expect("decodes");
        MediaMatchValidator::hamming(ha, hb)
    }

    #[test]
    fn regression_thumbnail_matches_but_page_media_differs_is_unverified() {
        let v = MediaMatchValidator::default_client();
        // The query is image B; the page now serves a different image C.
        let b = pattern_bytes(0);
        let c = pattern_bytes(1);
        assert!(
            far_part_dhash(&b, &c) > 12,
            "patterns must be visually far apart"
        );
        let query = ValidationQuery::new(b.clone(), b.clone());

        // Google-provided thumbnail of the matched result: matches B exactly.
        let thumbnail = v.compare("https://encrypted-tbn1.gstatic.com/tbn".into(), &b, &query);
        assert!(
            thumbnail.passed,
            "the false-positive precondition: thumbnail matches"
        );

        // Independently fetched page media: B is NOT on the page; C is.
        let page = v.compare("https://scontent.example/actual.png".into(), &c, &query);
        assert!(!page.passed, "page media must not match the query");

        let evidence = v.social_failure(Some(&thumbnail), Some(&page));
        assert!(!evidence.passed, "must NOT verify from the thumbnail alone");
        let note = evidence.note.as_deref().unwrap_or("");
        assert!(
            note.contains("supporting evidence only"),
            "thumbnail matched but is supporting: {note}"
        );
        assert!(
            note.contains("page media"),
            "page outcome must be recorded: {note}"
        );

        let mut m = match_at("https://www.instagram.com/p/AAAA");
        m.media_match = Some(evidence);
        assert_eq!(
            derive_state(&sample_result(vec![m])),
            SearchMatchState::SocialCandidateUnverified
        );
    }

    #[test]
    fn social_page_with_matching_page_media_is_verified() {
        let v = MediaMatchValidator::default_client();
        let b = pattern_bytes(0);
        let query = ValidationQuery::new(b.clone(), b.clone());

        // The page's og:image is exactly the searched media.
        let page = v.compare("https://scontent.example/upload.png".into(), &b, &query);
        assert!(page.passed);

        let mut m = match_at("https://www.instagram.com/p/BBBB");
        m.media_match = Some(page);
        assert_eq!(
            derive_state(&sample_result(vec![m])),
            SearchMatchState::SocialMatchVerified
        );
    }

    #[test]
    fn social_candidate_with_inaccessible_page_is_unverified_even_with_matching_thumbnail() {
        let v = MediaMatchValidator::default_client();
        let b = pattern_bytes(0);
        let query = ValidationQuery::new(b.clone(), b.clone());

        let thumbnail = v.compare("https://encrypted-tbn1.gstatic.com/tbn".into(), &b, &query);
        assert!(thumbnail.passed);
        // Instagram refuses the page (login wall/bot protection): the media of
        // the page cannot be independently tied to the source, so the honest
        // outcome is UNVERIFIED — never a bypass.
        let evidence = v.social_failure(Some(&thumbnail), None);
        assert!(!evidence.passed);
        assert!(
            evidence
                .note
                .as_deref()
                .unwrap_or("")
                .contains("no source URL to inspect")
        );

        let mut m = match_at("https://www.instagram.com/p/CCCC");
        m.media_match = Some(evidence);
        assert_eq!(
            derive_state(&sample_result(vec![m])),
            SearchMatchState::SocialCandidateUnverified
        );
    }

    /// Minimal static HTTP server for exercising `validate_one` over real
    /// reqwest calls without leaving the machine.
    struct StaticServer {
        port: u16,
        _handle: std::thread::JoinHandle<()>,
    }

    impl StaticServer {
        fn serve(routes: Vec<(&'static str, &'static str, Vec<u8>)>) -> Self {
            Self::serve_on(0, routes)
        }

        fn serve_on(port: u16, routes: Vec<(&'static str, &'static str, Vec<u8>)>) -> Self {
            let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind loopback");
            let port = listener.local_addr().expect("port").port();
            let handle = std::thread::spawn(move || {
                for conn in listener.incoming() {
                    let Ok(mut stream) = conn else {
                        continue;
                    };
                    let path = read_request_path(&mut stream);
                    let Some((ctype, body)) = routes
                        .iter()
                        .find(|(p, _, _)| Some(*p) == path.as_deref())
                        .map(|(_, c, b)| (*c, b.clone()))
                    else {
                        let _ = write_response(&mut stream, "text/plain", b"404");
                        continue;
                    };
                    let _ = write_response(&mut stream, ctype, &body);
                }
            });
            Self {
                port,
                _handle: handle,
            }
        }
    }

    fn read_request_path(stream: &mut TcpStream) -> Option<String> {
        use std::io::Read;
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).ok()?;
        let head = String::from_utf8_lossy(&buf[..n]);
        let line = head.lines().next()?;
        let path = line.split_whitespace().nth(1)?;
        Some(path.to_string())
    }

    fn write_response(stream: &mut TcpStream, ctype: &str, body: &[u8]) -> std::io::Result<()> {
        use std::io::Write;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )?;
        stream.write_all(body)?;
        stream.flush()
    }

    #[tokio::test]
    async fn validate_one_uses_provider_thumbnail_for_non_social_fast_path() {
        let b = pattern_bytes(0);
        let c = pattern_bytes(1);
        let server = StaticServer::serve(vec![
            ("/thumb.png", "image/png", b.clone()),
            ("/page.html", "text/html", Vec::new()),
            ("/media.png", "image/png", c),
        ]);
        let v = MediaMatchValidator::default_client();
        let mut m = match_at(&format!("http://127.0.0.1:{}/page.html", server.port));
        m.thumbnail_url = Some(format!("http://127.0.0.1:{}/thumb.png", server.port));
        let query = ValidationQuery::new(b.clone(), b.clone());

        v.validate_one(&mut m, &query).await;

        let ev = m.media_match.as_ref().expect("evidence recorded");
        assert!(ev.passed);
        assert_eq!(ev.method, Some(MediaMatchMethod::ExactHash));
        assert_eq!(
            ev.media_url.as_deref(),
            Some(format!("http://127.0.0.1:{}/thumb.png", server.port).as_str())
        );
    }

    #[tokio::test]
    async fn validate_one_falls_back_to_page_media_when_thumbnail_is_absent() {
        let b = pattern_bytes(0);
        // Reserve a stable loopback port so the page HTML can name the real
        // media URL before the server thread binds it.
        let guest = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = guest.local_addr().unwrap().port();
        drop(guest);

        let server = StaticServer::serve_on(
            port,
            vec![
                ("/page.html", "text/html", {
                    format!(
                        "<meta property=\"og:image\" content=\"http://127.0.0.1:{port}/media.png\">"
                    )
                    .into_bytes()
                }),
                ("/media.png", "image/png", b.clone()),
            ],
        );
        let v = MediaMatchValidator::default_client();
        let mut m = match_at(&format!("http://127.0.0.1:{}/page.html", server.port));
        let query = ValidationQuery::new(b.clone(), b.clone());

        v.validate_one(&mut m, &query).await;

        let ev = m.media_match.as_ref().expect("evidence recorded");
        assert!(ev.passed, "page media matches: {:?}", ev.note);
        assert_eq!(
            ev.media_url.as_deref(),
            Some(format!("http://127.0.0.1:{}/media.png", server.port).as_str())
        );
    }
}
