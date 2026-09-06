//! Verification stage: deterministic, synchronous policy evaluation.
//!
//! This stage performs NO I/O. It evaluates the collected analysis and
//! search results against a `VerificationPolicy` and produces a
//! `VerificationResult`. Decisions are deterministic given the same inputs.

use chrono::Utc;
use immutara_core::ImmutaraError;
use immutara_core::domain::analysis::AnalysisResult;
use immutara_core::domain::evidence::Evidence;
use immutara_core::domain::search::SearchResult;
use immutara_core::domain::verification::{
    VerificationCheck, VerificationPolicy, VerificationResult,
};

/// Inputs available to the verification stage.
pub struct VerifyInput<'a> {
    pub evidence: &'a Evidence,
    pub analysis: Option<&'a AnalysisResult>,
    pub search: Option<&'a SearchResult>,
    pub policy: &'a VerificationPolicy,
}

/// Evaluate evidence against a verification policy.
///
/// All checks are collected (not short-circuited) so the result gives a
/// complete picture of what passed and failed.
pub fn verify(input: VerifyInput) -> Result<VerificationResult, ImmutaraError> {
    let mut checks: Vec<VerificationCheck> = Vec::new();

    if input.policy.require_analysis {
        match input.analysis {
            Some(analysis) => {
                let passed = analysis
                    .objects
                    .iter()
                    .all(|o| o.confidence >= input.policy.min_analysis_confidence)
                    && analysis
                        .text_regions
                        .iter()
                        .all(|t| t.confidence >= input.policy.min_analysis_confidence);
                checks.push(VerificationCheck {
                    name: "analysis_confidence".to_string(),
                    passed,
                    details: format!("min confidence {}", input.policy.min_analysis_confidence),
                });
            }
            None => checks.push(VerificationCheck {
                name: "analysis_required".to_string(),
                passed: false,
                details: "analysis result missing".to_string(),
            }),
        }
    }

    if let Some(search) = input.search {
        let match_count_ok = search.matches.len() >= input.policy.min_search_matches;
        checks.push(VerificationCheck {
            name: "min_search_matches".to_string(),
            passed: match_count_ok,
            details: format!(
                "{} matches (min {})",
                search.matches.len(),
                input.policy.min_search_matches
            ),
        });

        // A provider that supplies no relevance score (NaN, e.g. the Lens
        // provider) cannot be judged by a numeric threshold: the score
        // criterion applies only where a real score is present.
        let similarity_ok = search.matches.iter().all(|m| {
            m.provider_score.is_nan() || m.provider_score >= input.policy.min_provider_score
        });
        checks.push(VerificationCheck {
            name: "min_provider_score".to_string(),
            passed: similarity_ok,
            details: format!("min provider score {}", input.policy.min_provider_score),
        });

        let providers_ok = input
            .policy
            .required_providers
            .iter()
            .all(|p| p == &search.provider_id);
        if !input.policy.required_providers.is_empty() {
            checks.push(VerificationCheck {
                name: "required_providers".to_string(),
                passed: providers_ok,
                details: "required providers satisfied".to_string(),
            });
        }
    }

    if input.policy.social_match_verified {
        let social_ok = input.search.is_some()
            && input.search.unwrap().social_state
                == immutara_core::domain::search::SearchMatchState::SocialMatchVerified;
        checks.push(VerificationCheck {
            name: "social_match_verified".to_string(),
            passed: social_ok,
            details: format!(
                "search state {:?} (requires a media-verified social match)",
                input.search.map(|s| s.social_state)
            ),
        });
    }

    let passed = checks.iter().all(|c| c.passed);
    Ok(VerificationResult {
        evidence_id: input.evidence.id,
        policy_version: input.policy.version,
        passed,
        checks,
        verified_at: Utc::now(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use immutara_core::domain::analysis::{BoundingBox, DetectedObject};
    use immutara_core::domain::evidence::{
        ContentHash, EvidenceId, EvidenceMetadata, SchemaVersion,
    };
    use immutara_core::domain::provenance::ProvenanceChain;
    use immutara_core::domain::search::SearchMatch;
    use immutara_core::domain::search::SearchMatchKind;

    fn sample_evidence() -> Evidence {
        Evidence {
            id: EvidenceId::new(),
            content_hash: ContentHash("abc".into()),
            metadata: EvidenceMetadata {
                source_path: None,
                mime_type: "image/jpeg".into(),
                file_size: 100,
                dimensions: Some((10, 10)),
                captured_at: None,
                schema_version: SchemaVersion(1),
            },
            provenance: ProvenanceChain::new(),
            ingested_at: Utc::now(),
        }
    }

    fn sample_policy() -> VerificationPolicy {
        VerificationPolicy {
            version: SchemaVersion(1),
            min_search_matches: 1,
            min_provider_score: 0.5,
            require_analysis: false,
            min_analysis_confidence: 0.5,
            required_providers: vec![],
            max_evidence_age: None,
            social_match_verified: false,
        }
    }

    fn sample_search(matches: Vec<SearchMatch>) -> SearchResult {
        SearchResult {
            evidence_id: EvidenceId::new(),
            provider_id: "mock".into(),
            search_input: immutara_core::domain::search::SearchInputKind::FullImage,
            matches,
            searched_at: Utc::now(),
            social_state: immutara_core::domain::search::SearchMatchState::NoResults,
        }
    }

    #[test]
    fn empty_result_not_evaluated_when_search_missing() {
        let evidence = sample_evidence();
        let policy = sample_policy();
        let result = verify(VerifyInput {
            evidence: &evidence,
            analysis: None,
            search: None,
            policy: &policy,
        })
        .unwrap();
        // Nothing to check -> passes (all checks vacuously true).
        assert!(result.passed);
        assert!(result.checks.is_empty());
    }

    #[test]
    fn insufficient_matches_fails() {
        let evidence = sample_evidence();
        let policy = sample_policy();
        let search = sample_search(vec![]);
        let result = verify(VerifyInput {
            evidence: &evidence,
            analysis: None,
            search: Some(&search),
            policy: &policy,
        })
        .unwrap();
        assert!(!result.passed);
        assert!(result.checks.iter().any(|c| c.name == "min_search_matches"));
    }

    #[test]
    fn sufficient_matches_and_similarity_passes() {
        let evidence = sample_evidence();
        let policy = sample_policy();
        let search = sample_search(vec![SearchMatch {
            match_kind: SearchMatchKind::Visual,
            source_url: None,
            source_domain: None,
            source_title: None,
            source_description: None,
            provider_score: 0.9,
            position: None,
            first_seen: None,
            thumbnail_url: None,
            media_match: None,
        }]);
        let result = verify(VerifyInput {
            evidence: &evidence,
            analysis: None,
            search: Some(&search),
            policy: &policy,
        })
        .unwrap();
        assert!(result.passed);
    }

    #[test]
    fn low_similarity_fails() {
        let evidence = sample_evidence();
        let policy = sample_policy();
        let search = sample_search(vec![SearchMatch {
            match_kind: SearchMatchKind::Visual,
            source_url: None,
            source_domain: None,
            source_title: None,
            source_description: None,
            provider_score: 0.1,
            position: None,
            first_seen: None,
            thumbnail_url: None,
            media_match: None,
        }]);
        let result = verify(VerifyInput {
            evidence: &evidence,
            analysis: None,
            search: Some(&search),
            policy: &policy,
        })
        .unwrap();
        assert!(!result.passed);
        assert!(result.checks.iter().any(|c| c.name == "min_provider_score"));
    }

    #[test]
    fn analysis_required_but_missing_fails() {
        let evidence = sample_evidence();
        let policy = VerificationPolicy {
            require_analysis: true,
            ..sample_policy()
        };
        let result = verify(VerifyInput {
            evidence: &evidence,
            analysis: None,
            search: None,
            policy: &policy,
        })
        .unwrap();
        assert!(!result.passed);
    }

    #[test]
    fn result_records_policy_version() {
        let evidence = sample_evidence();
        let policy = sample_policy();
        let result = verify(VerifyInput {
            evidence: &evidence,
            analysis: None,
            search: None,
            policy: &policy,
        })
        .unwrap();
        assert_eq!(result.policy_version, SchemaVersion(1));
        assert_eq!(result.evidence_id, evidence.id);
    }

    #[test]
    fn low_analysis_confidence_fails_when_required() {
        let evidence = sample_evidence();
        let policy = VerificationPolicy {
            require_analysis: true,
            min_analysis_confidence: 0.9,
            ..sample_policy()
        };
        let analysis = AnalysisResult {
            evidence_id: evidence.id,
            provider_id: "mock".into(),
            model_version: None,
            objects: vec![DetectedObject {
                label: "x".into(),
                confidence: 0.5,
                bounding_box: BoundingBox {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                },
            }],
            text_regions: vec![],
            face_analysis: None,
            metadata_hash: ContentHash("x".into()),
            analyzed_at: Utc::now(),
        };
        let result = verify(VerifyInput {
            evidence: &evidence,
            analysis: Some(&analysis),
            search: None,
            policy: &policy,
        })
        .unwrap();
        assert!(!result.passed);
    }

    #[test]
    fn social_match_verified_requires_verified_social_state() {
        let evidence = sample_evidence();
        let policy = VerificationPolicy {
            social_match_verified: true,
            min_search_matches: 0,
            ..sample_policy()
        };

        // Unverified social candidate -> fails.
        let base = |url: &str| SearchMatch {
            match_kind: SearchMatchKind::Visual,
            source_url: Some(url.into()),
            source_domain: None,
            source_title: None,
            source_description: None,
            provider_score: 0.9,
            position: None,
            first_seen: None,
            thumbnail_url: None,
            media_match: None,
        };
        let mut unverified = base("https://reddit.com/r/x/1");
        unverified.media_match = Some(immutara_core::domain::search::MediaMatchEvidence {
            method: None,
            distance: None,
            threshold: Some(12),
            passed: false,
            media_url: None,
            note: Some("login wall".into()),
        });
        let mut search_failed = sample_search(vec![unverified]);
        search_failed.social_state =
            immutara_core::domain::search::SearchMatchState::SocialCandidateUnverified;
        let result = verify(VerifyInput {
            evidence: &evidence,
            analysis: None,
            search: Some(&search_failed),
            policy: &policy,
        })
        .unwrap();
        assert!(!result.passed);
        assert!(
            result
                .checks
                .iter()
                .any(|c| c.name == "social_match_verified")
        );

        // No search at all -> the check fails too (never vacuous).
        let result = verify(VerifyInput {
            evidence: &evidence,
            analysis: None,
            search: None,
            policy: &policy,
        })
        .unwrap();
        assert!(!result.passed);

        // Media-verified social match -> passes.
        let mut verified = base("https://reddit.com/r/x/2");
        verified.media_match = Some(immutara_core::domain::search::MediaMatchEvidence {
            method: Some(immutara_core::domain::search::MediaMatchMethod::PerceptualHash),
            distance: Some(4),
            threshold: Some(12),
            passed: true,
            media_url: Some("https://external.redditmedia.com/a.jpg".into()),
            note: None,
        });
        let mut search_ok = sample_search(vec![verified]);
        search_ok.social_state =
            immutara_core::domain::search::SearchMatchState::SocialMatchVerified;
        let result = verify(VerifyInput {
            evidence: &evidence,
            analysis: None,
            search: Some(&search_ok),
            policy: &policy,
        })
        .unwrap();
        assert!(result.passed);
    }
}
