//! Reverse-image-search result domain types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::analysis::BoundingBox;
use super::evidence::EvidenceId;

/// What image was actually submitted to the search provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchInputKind {
    /// The full evidence image.
    #[default]
    FullImage,
    /// A tight crop of the selected face (bbox + padding), no background.
    FaceCrop,
}

/// A locally-generated face crop ready to be searched.
///
/// Carries only image bytes and the spatial extent of the crop — never any
/// biometric data such as the SFace embedding (which stays local and is
/// intentionally absent from the domain model).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FaceCrop {
    /// Re-encoded JPEG bytes of the cropped region.
    pub image_bytes: Vec<u8>,
    /// MIME type of `image_bytes` (always `image/jpeg` for generated crops).
    pub mime_type: String,
    /// The selected-face bounding box the crop was derived from.
    pub bounding_box: BoundingBox,
}

/// The search input handed to a provider by the pipeline.
#[derive(Debug, Clone)]
pub enum SearchInput {
    /// Search the full evidence image.
    FullImage,
    /// Search the selected face crop first (providers may fall back).
    FaceCrop(FaceCrop),
}

/// How closely a provider considers a match to correspond to the submitted
/// image — i.e. the semantic tier of the result, not a numeric score.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchMatchKind {
    /// The provider reports the submitted image and the result are the same
    /// picture (e.g. Google Lens "exact match").
    Exact,
    /// A visually-similar but not identical match. Default for providers that
    /// do not distinguish.
    #[default]
    Visual,
}

/// Deterministic method used to compare the searched image against a public
/// candidate's media. Never a facial-identity comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaMatchMethod {
    /// Byte-identical image (SHA-256 of the fetched media equals the query).
    ExactHash,
    /// Perceptual hash (dHash Hamming distance) on resized/recompressed copies.
    PerceptualHash,
}

/// Evidence that the candidate's public media was actually fetched and
/// compared to the searched image.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaMatchEvidence {
    /// Comparison method actually applied. `None` when media could not be
    /// publicly retrieved (login wall, 404, non-image, oversized, timeout).
    pub method: Option<MediaMatchMethod>,
    /// Measured distance (`Some(0)` for byte-identical; Hamming distance for
    /// the perceptual hash; `None` when media was not retrievable).
    pub distance: Option<u32>,
    /// The comparison threshold the `passed` flag is judged against.
    pub threshold: Option<u32>,
    /// True when the candidate media visually matches the searched image.
    pub passed: bool,
    /// Public URL of the media that was actually fetched and compared.
    pub media_url: Option<String>,
    /// Outcome detail (e.g. "HTTP 403 login wall", "no og:image").
    pub note: Option<String>,
}

/// Overall outcome of the search stage for provenance purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SearchMatchState {
    /// No matches at all.
    NoResults,
    /// Only non-social matches were returned (generic web provenance).
    WebMatch,
    /// A social-domain candidate exists that has not been media-validated.
    SocialCandidate,
    /// A social candidate exists but the media could not be independently
    /// validated (login wall, inaccessible, non-image, or comparison failed).
    SocialCandidateUnverified,
    /// A social candidate's public media was retrieved and visually matches the
    /// searched image.
    #[default]
    SocialMatchVerified,
}

/// A single match returned by a reverse-image-search provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchMatch {
    /// How closely the provider matched the submitted image.
    #[serde(default)]
    pub match_kind: SearchMatchKind,
    /// Canonical URL where the matching image appears publicly.
    pub source_url: Option<String>,
    /// Top-level web domain of `source_url` (e.g. `reddit.com`).
    pub source_domain: Option<String>,
    /// Title of the source page where the match was found.
    pub source_title: Option<String>,
    /// Provider-supplied description of the source (e.g. domain or platform).
    pub source_description: Option<String>,
    /// Provider relevance score normalized to `[0, 1]` where available.
    ///
    /// This is the provider's real signal, never fabricated. When a provider
    /// omits a score, the value is `NaN` ("unavailable") rather than a made-up
    /// number; renderers should treat non-finite values as "n/a".
    pub provider_score: f64,
    /// Provider-reported ordering of the match (1 = top of the ranking).
    pub position: Option<u32>,
    pub first_seen: Option<DateTime<Utc>>,
    pub thumbnail_url: Option<String>,
    /// Media-match validation evidence for this candidate, when validation was
    /// attempted (either the public media matched the searched image, or the
    /// evidence records why it could not be established).
    #[serde(default)]
    pub media_match: Option<MediaMatchEvidence>,
}

/// The structured output of a reverse-image-search stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchResult {
    pub evidence_id: EvidenceId,
    pub provider_id: String,
    /// Which image was actually searched (full image or face crop).
    pub search_input: SearchInputKind,
    pub matches: Vec<SearchMatch>,
    pub searched_at: DateTime<Utc>,
    /// Overall outcome of the search stage (see [`SearchMatchState`]).
    #[serde(default)]
    pub social_state: SearchMatchState,
}

impl SearchResult {
    /// The single match the attestation binds into the record.
    ///
    /// Priority: (1) a media-verified social match, (2) the first **exact**
    /// match when any exist, (3) the first visual match. A verified social
    /// match is the strongest provenance (the public media actually contains
    /// the searched image); exact matches are stronger than plain visual ones.
    pub fn selected(&self) -> Option<&SearchMatch> {
        self.matches
            .iter()
            .find(|m| m.media_match.as_ref().map(|e| e.passed).unwrap_or(false))
            .or_else(|| {
                self.matches
                    .iter()
                    .find(|m| m.match_kind == SearchMatchKind::Exact)
            })
            .or_else(|| self.matches.first())
    }

    /// Index of the selected match within [`Self::matches`], when present.
    pub fn selected_index(&self) -> Option<usize> {
        if self.matches.is_empty() {
            return None;
        }
        self.matches
            .iter()
            .position(|m| m.media_match.as_ref().map(|e| e.passed).unwrap_or(false))
            .or_else(|| {
                self.matches
                    .iter()
                    .position(|m| m.match_kind == SearchMatchKind::Exact)
            })
            .or(Some(0))
    }
}

/// True when the URL's domain is a public social-media platform.
///
/// Social platforms are a first-class provenance target. This is a purely
/// syntactic classification of the URL's host — it does NOT assert that the
/// page is publicly reachable, is a real profile, or is owned by anyone.
pub fn is_social_media_url(url: &str) -> bool {
    match hostname_of(url) {
        Some(host) => SOCIAL_MEDIA_HOSTS
            .iter()
            .any(|known| host == *known || host.ends_with(&format!(".{known}"))),
        None => false,
    }
}

/// Extract the lower-cased hostname (without scheme/port) from a URL.
pub fn hostname_of(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_start_matches("www.");
    let after_scheme = match trimmed.find("://") {
        Some(idx) => &trimmed[idx + 3..],
        None => trimmed,
    };
    let host = after_scheme.split('/').next().unwrap_or("").to_lowercase();
    // Strip an explicit numeric port when present (e.g. `host:8443`).
    let host = match host.rsplit_once(':') {
        Some((h, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => {
            h.to_string()
        }
        _ => host,
    };
    let host = host.trim_start_matches("www.").to_string();
    // A hostname cannot contain whitespace; reject junk like "not a url".
    if host.is_empty() || host.chars().any(|c| c.is_whitespace()) {
        None
    } else {
        Some(host)
    }
}

/// Public social-media platforms recognized as provenance targets.
const SOCIAL_MEDIA_HOSTS: &[&str] = &[
    "reddit.com",
    "facebook.com",
    "x.com",
    "twitter.com",
    "instagram.com",
    "youtube.com",
    "tiktok.com",
    "linkedin.com",
    "pinterest.com",
    "tumblr.com",
    "threads.net",
    "snapchat.com",
    "weibo.com",
    "vk.com",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostname_extraction_handles_schemes_ports_and_www() {
        assert_eq!(
            hostname_of("https://www.reddit.com/r/x/post"),
            Some("reddit.com".into())
        );
        assert_eq!(hostname_of("http://x.com/status/123"), Some("x.com".into()));
        assert_eq!(
            hostname_of("https://example.com:8443/path?q=1"),
            Some("example.com".into())
        );
        assert_eq!(hostname_of("not a url"), None);
        assert_eq!(hostname_of(""), None);
    }

    #[test]
    fn social_media_classification_covers_major_platforms() {
        for url in [
            "https://www.reddit.com/r/todayilearned/comments/1je7ab",
            "https://x.com/SomeUser/status/907693162625298432",
            "https://twitter.com/SomeOther/status/1",
            "https://www.facebook.com/groups/123/posts/456",
            "https://www.instagram.com/p/abc/",
            "https://www.youtube.com/watch?v=abc",
            "https://www.tiktok.com/@user/video/1",
            "https://www.linkedin.com/posts/x_y",
            "https://www.pinterest.com/pin/1/",
        ] {
            assert!(is_social_media_url(url), "{url} should be social media");
        }
    }

    #[test]
    fn social_media_classification_excludes_news_and_academic() {
        for url in [
            "https://www.theguardian.com/technology/2024/01/01/lena",
            "https://news.com.au/story",
            "https://arxiv.org/abs/2301.00001",
            "https://www.springer.com/chapter/10.1007",
            "https://example.com/photo",
        ] {
            assert!(
                !is_social_media_url(url),
                "{url} should NOT be social media"
            );
        }
    }

    #[test]
    fn search_input_kind_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&SearchInputKind::FaceCrop).unwrap(),
            "\"face_crop\""
        );
        assert_eq!(
            serde_json::to_string(&SearchInputKind::FullImage).unwrap(),
            "\"full_image\""
        );
    }

    fn match_at(kind: SearchMatchKind, url: &str, position: u32) -> SearchMatch {
        SearchMatch {
            match_kind: kind,
            source_url: Some(url.to_string()),
            source_domain: hostname_of(url),
            source_title: None,
            source_description: None,
            provider_score: f64::NAN,
            position: Some(position),
            first_seen: None,
            thumbnail_url: None,
            media_match: None,
        }
    }

    fn result_with(matches: Vec<SearchMatch>) -> SearchResult {
        SearchResult {
            evidence_id: EvidenceId::new(),
            provider_id: "mock".to_string(),
            search_input: SearchInputKind::FullImage,
            matches,
            searched_at: Utc::now(),
            social_state: SearchMatchState::NoResults,
        }
    }

    #[test]
    fn selected_prefers_exact_match_and_falls_back_to_visual() {
        // Exact matches present -> the first exact one is selected.
        let result = result_with(vec![
            match_at(SearchMatchKind::Visual, "https://x.com/u/status/2", 1),
            match_at(
                SearchMatchKind::Exact,
                "https://www.reddit.com/r/x/comments/1",
                1,
            ),
            match_at(SearchMatchKind::Exact, "https://x.com/u/status/3", 2),
        ]);
        let selected = result.selected().expect("selected match present");
        assert_eq!(selected.match_kind, SearchMatchKind::Exact);
        assert_eq!(selected.source_domain.as_deref(), Some("reddit.com"));
        assert_eq!(result.selected_index(), Some(1));

        // No exact matches -> first visual match wins.
        let result = result_with(vec![
            match_at(SearchMatchKind::Visual, "https://x.com/u/status/2", 1),
            match_at(SearchMatchKind::Visual, "https://news.com/story", 2),
        ]);
        let selected = result.selected().expect("selected match present");
        assert_eq!(selected.match_kind, SearchMatchKind::Visual);
        assert_eq!(result.selected_index(), Some(0));

        // Empty result -> no selection.
        assert!(result_with(Vec::new()).selected().is_none());
    }

    #[test]
    fn selected_prefers_verified_social_media_match() {
        // Unverified exact first, verified visual social later: the verified
        // one must win even though it is only a "visual" provider match.
        let exact = match_at(SearchMatchKind::Exact, "https://x.com/u/status/1", 2);
        let mut visual = match_at(
            SearchMatchKind::Visual,
            "https://www.reddit.com/r/x/comments/1",
            1,
        );
        visual.media_match = Some(MediaMatchEvidence {
            method: Some(MediaMatchMethod::PerceptualHash),
            distance: Some(4),
            threshold: Some(12),
            passed: true,
            media_url: Some("https://external.redditmedia.com/x.jpg".to_string()),
            note: None,
        });
        let result = result_with(vec![exact.clone(), visual.clone()]);
        let selected = result.selected().expect("selected match present");
        assert_eq!(
            selected.source_url.as_deref(),
            Some("https://www.reddit.com/r/x/comments/1")
        );
        assert_eq!(result.selected_index(), Some(1));

        // A failed validation never outranks an exact match.
        let mut unverified = visual.clone();
        unverified.media_match = Some(MediaMatchEvidence {
            method: None,
            distance: None,
            threshold: None,
            passed: false,
            media_url: None,
            note: Some("login wall".to_string()),
        });
        let result = result_with(vec![unverified, exact]);
        assert_eq!(result.selected_index(), Some(1));
    }

    #[test]
    fn evidence_serializes_to_stable_canonical_shapes() {
        let evidence = MediaMatchEvidence {
            method: Some(MediaMatchMethod::PerceptualHash),
            distance: Some(4),
            threshold: Some(12),
            passed: true,
            media_url: Some("https://external.redditmedia.com/x.jpg".to_string()),
            note: None,
        };
        let json = serde_json::to_value(&evidence).unwrap();
        assert_eq!(json["method"], "perceptual_hash");
        assert_eq!(
            serde_json::to_string(&SearchMatchState::NoResults).unwrap(),
            "\"NO_RESULTS\""
        );
        assert_eq!(
            serde_json::to_string(&SearchMatchState::SocialMatchVerified).unwrap(),
            "\"SOCIAL_MATCH_VERIFIED\""
        );
    }
}
