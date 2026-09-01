//! Analysis result domain types (computer-vision output).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::evidence::{ContentHash, EvidenceId};

/// A rectangular region within an image.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoundingBox {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// An object detected in evidence by a computer-vision model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DetectedObject {
    pub label: String,
    pub confidence: f64,
    pub bounding_box: BoundingBox,
}

/// A region of OCR-recognized text within evidence.
///
/// Note: this type may carry recognized text but never biometric data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextRegion {
    pub text: String,
    pub confidence: f64,
    pub bounding_box: BoundingBox,
}

/// The structured output of a computer-vision analysis stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalysisResult {
    pub evidence_id: EvidenceId,
    pub provider_id: String,
    pub model_version: Option<String>,
    pub objects: Vec<DetectedObject>,
    pub text_regions: Vec<TextRegion>,
    pub metadata_hash: ContentHash,
    pub analyzed_at: DateTime<Utc>,
}
