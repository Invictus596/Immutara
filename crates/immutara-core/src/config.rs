//! Application configuration.
//!
//! Currently a minimal structural skeleton. Provider-specific settings will
//! be added as those provider implementations are introduced.

use serde::{Deserialize, Serialize};

use crate::domain::evidence::SchemaVersion;
use crate::errors::ImmutaraError;

/// Top-level configuration loaded from TOML (with future env overrides).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub pipeline: PipelineConfig,
    pub verification: VerificationConfig,
    pub search: ProviderConfig,
    pub analysis: ProviderConfig,
    pub attestation: ProviderConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PipelineConfig {
    pub evidence_dir: String,
    pub log_level: String,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            evidence_dir: "./evidence".to_string(),
            log_level: "info".to_string(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct VerificationConfig {
    pub policy: VerificationPolicyConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VerificationPolicyConfig {
    pub version: SchemaVersion,
    pub min_search_matches: usize,
    pub min_search_similarity: f64,
    pub require_analysis: bool,
    pub min_analysis_confidence: f64,
    pub required_providers: Vec<String>,
}

impl Default for VerificationPolicyConfig {
    fn default() -> Self {
        Self {
            version: SchemaVersion(1),
            min_search_matches: 0,
            min_search_similarity: 0.0,
            require_analysis: false,
            min_analysis_confidence: 0.0,
            required_providers: Vec::new(),
        }
    }
}

/// Identifies which provider implementation a stage should use.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderConfig {
    pub provider: String,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            provider: "mock".to_string(),
        }
    }
}

impl Config {
    /// Load configuration from a TOML file path.
    pub fn from_path(path: &str) -> Result<Self, ImmutaraError> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| ImmutaraError::Config(format!("failed to read {path}: {e}")))?;
        let config: Config = toml::from_str(&raw)
            .map_err(|e| ImmutaraError::Config(format!("failed to parse {path}: {e}")))?;
        Ok(config)
    }
}
