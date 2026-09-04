"""Face-detection and face-embedding logic backed by OpenCV's DNN module.

This module is intentionally free of any I/O protocol concerns; `face_worker`
wraps it with the line-delimited JSON transport.

Model versions pinned to match the files fetched by
`models/download_models.py`.
"""

from pathlib import Path

import cv2
import numpy as np

from .errors import (
    CvError,
    IMAGE_UNREADABLE,
    INFERENCE_FAILED,
    MODEL_LOAD_FAILED,
    MODEL_MISSING,
    INTERNAL,
)

DETECTOR_MODEL = "YuNet"
DETECTOR_MODEL_FILE = "face_detection_yunet_2023mar.onnx"
DETECTOR_MODEL_VERSION = "2023mar"

RECOGNIZER_MODEL = "SFace"
RECOGNIZER_MODEL_FILE = "face_recognition_sface_2021dec.onnx"
RECOGNIZER_MODEL_VERSION = "2021dec"

_DETECTOR_SUBDIR = "face_detection_yunet"
_RECOGNIZER_SUBDIR = "face_recognition_sface"


class FaceAnalysis:
    """Result of analyzing a single image."""

    __slots__ = (
        "face_count",
        "confidence",
        "bounding_box",
        "embedding_dimensions",
        "embedding",
        "detector_model",
        "recognizer_model",
        "model_version",
    )

    def __init__(
        self,
        face_count: int,
        confidence: float,
        bounding_box: tuple,
        embedding_dimensions: int,
        embedding: np.ndarray,
        detector_model: str,
        recognizer_model: str,
        model_version: str,
    ):
        self.face_count = face_count
        self.confidence = confidence
        # bounding_box as (x, y, width, height)
        self.bounding_box = bounding_box
        self.embedding_dimensions = embedding_dimensions
        self.embedding = embedding
        self.detector_model = detector_model
        self.recognizer_model = recognizer_model
        self.model_version = model_version


class FaceAnalyzer:
    """Loads YuNet + SFace once and provides `analyze(image_path)`."""

    def __init__(self, models_dir: str | Path, score_threshold: float = 0.9):
        self._score_threshold = score_threshold
        root = Path(models_dir)
        detector_path = root / _DETECTOR_SUBDIR / DETECTOR_MODEL_FILE
        recognizer_path = root / _RECOGNIZER_SUBDIR / RECOGNIZER_MODEL_FILE

        missing = [p for p in (detector_path, recognizer_path) if not p.exists()]
        if missing:
            raise CvError(
                MODEL_MISSING,
                "missing model file(s): "
                + ", ".join(str(m) for m in missing)
                + " — run `python3 models/download_models.py`",
            )

        try:
            # Both models use OpenCV's DNN module with the default CPU backend.
            self._detector = cv2.FaceDetectorYN_create(
                str(detector_path),
                "",
                (320, 320),
                score_threshold=self._score_threshold,
            )
            self._recognizer = cv2.FaceRecognizerSF_create(str(recognizer_path), "")
        except cv2.error as exc:  # noqa: BLE001
            raise CvError(MODEL_LOAD_FAILED, f"failed to load models: {exc}") from exc

        self._detector_model = DETECTOR_MODEL
        self._recognizer_model = RECOGNIZER_MODEL
        self._model_version = RECOGNIZER_MODEL_VERSION

    @staticmethod
    def _primary_face(faces: np.ndarray):
        """Deterministic primary face: highest confidence, largest area tie-break.

        `faces` is an Nx15 array: [x, y, w, h, score, ...landmarks].
        The score column placement differs between OpenCV 4.x (index 4) and
        OpenCV 5.x's new graph engine (index 14), so the layout is detected
        from the first row and applied consistently.
        """
        if len(faces) == 0:
            return None

        # Score is the value in [0, 1]; the box is always indices 0..3.
        row = faces[0]
        score_at = 14 if 0.0 <= float(row[14]) <= 1.0 and not (0.0 <= float(row[4]) <= 1.0) else 4

        def keyed(det):
            x, y, w, h = (float(v) for v in det[:4])
            score = float(det[score_at])
            return (round(score, 6), w * h, x, y)

        order = sorted(range(len(faces)), key=lambda i: keyed(faces[i]), reverse=True)
        best = faces[order[0]]
        x, y, w, h = (int(round(float(v))) for v in best[:4])
        return score_at, best, (x, y, w, h), float(best[score_at])

    def analyze(self, image_path: str | Path, max_dimension: int | None = None) -> FaceAnalysis:
        """Detect faces and embed the primary face in `image_path`."""
        image_path = str(image_path)
        img = cv2.imread(image_path, cv2.IMREAD_COLOR)
        if img is None:
            raise CvError(IMAGE_UNREADABLE, f"cannot read image: {image_path}")

        try:
            if max_dimension and max_dimension > 0:
                img = self._fit_to_max(img, max_dimension)

            height, width = img.shape[:2]
            self._detector.setInputSize((width, height))

            faces = None
            try:
                _, faces = self._detector.detect(img)
            except cv2.error as exc:  # noqa: BLE001
                raise CvError(INFERENCE_FAILED, f"detection failed: {exc}") from exc

            if faces is None or len(faces) == 0:
                return FaceAnalysis(
                    face_count=0,
                    confidence=0.0,
                    bounding_box=(0, 0, 0, 0),
                    embedding_dimensions=0,
                    embedding=np.zeros((0,), dtype=np.float32),
                    detector_model=self._detector_model,
                    recognizer_model=self._recognizer_model,
                    model_version=self._model_version,
                )

            primary = self._primary_face(faces)
            if primary is None:
                return FaceAnalysis(
                    face_count=0,
                    confidence=0.0,
                    bounding_box=(0, 0, 0, 0),
                    embedding_dimensions=0,
                    embedding=np.zeros((0,), dtype=np.float32),
                    detector_model=self._detector_model,
                    recognizer_model=self._recognizer_model,
                    model_version=self._model_version,
                )

            _score_at, primary_row, box, confidence = primary
            x, y, w, h = box

            try:
                aligned = self._recognizer.alignCrop(img, primary_row)
                feat = self._recognizer.feature(aligned)
            except cv2.error as exc:  # noqa: BLE001
                raise CvError(INFERENCE_FAILED, f"embedding failed: {exc}") from exc

            dims = int(feat.shape[1]) if feat.ndim >= 2 else int(feat.shape[0])
            flat = feat.reshape(-1).astype(np.float32)
            return FaceAnalysis(
                face_count=int(len(faces)),
                confidence=confidence,
                bounding_box=(x, y, w, h),
                embedding_dimensions=dims,
                embedding=flat,
                detector_model=self._detector_model,
                recognizer_model=self._recognizer_model,
                model_version=self._model_version,
            )
        except CvError:
            raise
        except Exception as exc:  # noqa: BLE001
            raise CvError(INTERNAL, f"unexpected analysis error: {exc}") from exc

    @staticmethod
    def _fit_to_max(img: np.ndarray, max_dimension: int) -> np.ndarray:
        height, width = img.shape[:2]
        longest = max(height, width)
        if longest <= max_dimension:
            return img
        scale = max_dimension / float(longest)
        new_size = (int(round(width * scale)), int(round(height * scale)))
        return cv2.resize(img, new_size, interpolation=cv2.INTER_AREA)
