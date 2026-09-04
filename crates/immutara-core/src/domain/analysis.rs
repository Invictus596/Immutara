//! Analysis result domain types.

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

/// The single face selected for feature extraction.
///
/// Carries only structural information and a cryptographic fingerprint of
/// the embedding — **never** the raw embedding vector, which is sensitive
/// biometric-derived data that stays local to the analysis stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectedFace {
    /// Detection confidence in `[0, 1]`.
    pub confidence: f64,
    /// The spatial extent of the detected face.
    pub bounding_box: BoundingBox,
    /// Dimensionality of the embedding produced by the recognizer.
    pub embedding_dimension: usize,
    /// Deterministic SHA-256 of the embedding vector. This is a stable
    /// fingerprint for provenance/future search; it is NOT the embedding.
    pub embedding_hash: ContentHash,
}

/// The structured outcome of the face-analysis stage (YuNet detection +
/// SFace recognition).
///
/// The raw biometric embedding is intentionally absent: only its
/// dimensionality and cryptographic hash are recorded here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FaceAnalysis {
    /// Provider that ran the analysis (e.g. `open-cv`).
    pub provider_id: String,
    /// Total number of faces detected in the image.
    pub face_count: u32,
    /// The primary selected face, when at least one face was found.
    pub selected_face: Option<SelectedFace>,
    /// Detector model name (e.g. `YuNet`).
    pub detector_model: String,
    /// Recognizer model name (e.g. `SFace`).
    pub recognizer_model: String,
    /// Recognizer model version string.
    pub model_version: String,
}

/// The structured output of a computer-vision analysis stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalysisResult {
    pub evidence_id: EvidenceId,
    pub provider_id: String,
    pub model_version: Option<String>,
    pub objects: Vec<DetectedObject>,
    pub text_regions: Vec<TextRegion>,
    /// Face-analysis details, when the provider performed face analysis.
    pub face_analysis: Option<FaceAnalysis>,
    pub metadata_hash: ContentHash,
    pub analyzed_at: DateTime<Utc>,
}
