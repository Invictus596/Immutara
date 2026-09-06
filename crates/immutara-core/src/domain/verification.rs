//! Verification policy and result domain types.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::evidence::{EvidenceId, SchemaVersion};

/// Deterministic, configurable, versioned rules governing whether evidence
/// passes verification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerificationPolicy {
    pub version: SchemaVersion,
    pub min_search_matches: usize,
    pub min_provider_score: f64,
    pub require_analysis: bool,
    pub min_analysis_confidence: f64,
    pub required_providers: Vec<String>,
    pub max_evidence_age: Option<Duration>,
    /// When true, verification only passes when the search stage produced a
    /// `SOCIAL_MATCH_VERIFIED` result (independent media-match validation).
    ///
    /// Defaults to `false` so existing policies and generic-web provenance
    /// keep working unchanged. The final demonstration config sets it `true`.
    #[serde(default)]
    pub social_match_verified: bool,
}

/// The outcome of a single check within a verification run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerificationCheck {
    pub name: String,
    pub passed: bool,
    pub details: String,
}

/// The result of evaluating evidence against a verification policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerificationResult {
    pub evidence_id: EvidenceId,
    pub policy_version: SchemaVersion,
    pub passed: bool,
    pub checks: Vec<VerificationCheck>,
    pub verified_at: DateTime<Utc>,
}
