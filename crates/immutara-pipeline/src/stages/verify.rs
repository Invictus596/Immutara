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

        let similarity_ok = search
            .matches
            .iter()
            .all(|m| m.provider_score >= input.policy.min_provider_score);
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
        }
    }

    fn sample_search(matches: Vec<SearchMatch>) -> SearchResult {
        SearchResult {
            evidence_id: EvidenceId::new(),
            provider_id: "mock".into(),
            matches,
            searched_at: Utc::now(),
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
            source_url: None,
            source_description: None,
            provider_score: 0.9,
            first_seen: None,
            thumbnail_url: None,
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
            source_url: None,
            source_description: None,
            provider_score: 0.1,
            first_seen: None,
            thumbnail_url: None,
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
}
