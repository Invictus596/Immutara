//! Mock providers for deterministic testing of the full pipeline flow.
//!
//! These are deterministic and synchronous in nature (they return fixed or
//! configurable results) but still honor the async trait boundaries so the
//! pipeline exercises the same code paths as real providers.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use immutara_core::ImmutaraError;
use immutara_core::domain::analysis::{
    AnalysisResult, BoundingBox, DetectedObject, FaceAnalysis, TextRegion,
};
use immutara_core::domain::attestation::{AttestationReceipt, AttestationRecord};
use immutara_core::domain::evidence::{ContentHash, Evidence};
use immutara_core::domain::search::{SearchMatch, SearchResult};
use immutara_core::providers::{AnalysisProvider, AttestationProvider, ImageSearchProvider};
use tokio::sync::Mutex;

/// A mock analysis provider returning configurable object detections.
pub struct MockAnalysisProvider {
    pub provider_id: String,
    pub objects: Vec<DetectedObject>,
    pub text_regions: Vec<TextRegion>,
    /// When set, the mock returns this face-analysis payload in results.
    pub face_analysis: Option<FaceAnalysis>,
    /// When set, analyzing returns this error.
    pub fail_with: Option<String>,
    pub calls: Arc<Mutex<usize>>,
}

impl Default for MockAnalysisProvider {
    fn default() -> Self {
        Self {
            provider_id: "mock-analysis".to_string(),
            objects: vec![DetectedObject {
                label: "test-object".to_string(),
                confidence: 0.95,
                bounding_box: BoundingBox {
                    x: 0,
                    y: 0,
                    width: 100,
                    height: 100,
                },
            }],
            text_regions: vec![],
            face_analysis: None,
            fail_with: None,
            calls: Arc::new(Mutex::new(0)),
        }
    }
}

#[async_trait]
impl AnalysisProvider for MockAnalysisProvider {
    async fn analyze(&self, evidence: &Evidence) -> Result<AnalysisResult, ImmutaraError> {
        *self.calls.lock().await += 1;
        if let Some(msg) = &self.fail_with {
            return Err(ImmutaraError::Provider {
                provider: self.provider_id.clone(),
                message: msg.clone(),
            });
        }
        Ok(AnalysisResult {
            evidence_id: evidence.id,
            provider_id: self.provider_id.clone(),
            model_version: Some("0.1.0".to_string()),
            objects: self.objects.clone(),
            text_regions: self.text_regions.clone(),
            face_analysis: self.face_analysis.clone(),
            metadata_hash: ContentHash("mock".repeat(64)),
            analyzed_at: Utc::now(),
        })
    }

    fn provider_id(&self) -> &str {
        &self.provider_id
    }
}

/// A mock reverse-image-search provider returning configurable matches.
pub struct MockSearchProvider {
    pub provider_id: String,
    pub matches: Vec<SearchMatch>,
    /// When set, searching returns this error.
    pub fail_with: Option<String>,
    pub calls: Arc<Mutex<usize>>,
}

impl Default for MockSearchProvider {
    fn default() -> Self {
        Self {
            provider_id: "mock-search".to_string(),
            matches: vec![SearchMatch {
                source_url: Some("https://example.com/mock".to_string()),
                source_description: None,
                provider_score: 0.9,
                first_seen: None,
                thumbnail_url: None,
            }],
            fail_with: None,
            calls: Arc::new(Mutex::new(0)),
        }
    }
}

#[async_trait]
impl ImageSearchProvider for MockSearchProvider {
    async fn search(&self, evidence: &Evidence) -> Result<SearchResult, ImmutaraError> {
        *self.calls.lock().await += 1;
        if let Some(msg) = &self.fail_with {
            return Err(ImmutaraError::Provider {
                provider: self.provider_id.clone(),
                message: msg.clone(),
            });
        }
        Ok(SearchResult {
            evidence_id: evidence.id,
            provider_id: self.provider_id.clone(),
            matches: self.matches.clone(),
            searched_at: Utc::now(),
        })
    }

    fn provider_id(&self) -> &str {
        &self.provider_id
    }
}

/// A mock attestation provider simulating an on-chain submission.
pub struct MockAttestationProvider {
    pub provider_id: String,
    pub chain_id: String,
    /// When set, attesting returns this error.
    pub fail_with: Option<String>,
    pub calls: Arc<Mutex<usize>>,
}

impl Default for MockAttestationProvider {
    fn default() -> Self {
        Self {
            provider_id: "mock-attestation".to_string(),
            chain_id: "0x1".to_string(),
            fail_with: None,
            calls: Arc::new(Mutex::new(0)),
        }
    }
}

#[async_trait]
impl AttestationProvider for MockAttestationProvider {
    async fn attest(
        &self,
        _record: &AttestationRecord,
    ) -> Result<AttestationReceipt, ImmutaraError> {
        *self.calls.lock().await += 1;
        if let Some(msg) = &self.fail_with {
            return Err(ImmutaraError::Provider {
                provider: self.provider_id.clone(),
                message: msg.clone(),
            });
        }
        Ok(AttestationReceipt {
            tx_hash: format!("0x{:064}", 0),
            block_number: 1,
            chain_id: self.chain_id.clone(),
        })
    }

    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    fn chain_id(&self) -> &str {
        &self.chain_id
    }
}
