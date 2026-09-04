"""Immutara computer-vision worker.

This package provides the real face-analysis worker used by the Rust pipeline
via a newline-delimited JSON (`lines` JSON) protocol over stdin/stdout.

The worker uses OpenCV's DNN module:
  - YuNet (`face_detection_yunet_2023mar.onnx`) for face detection, and
  - SFace (`face_recognition_sface_2021dec.onnx`) for face embedding.

Privacy note: this worker is a *local*, stateless inference process. It emits
only a SHA-256 hash of the selected face's embedding to the caller; raw
embeddings never leave the process boundary in persistent form.
"""

__version__ = "0.1.0"

__all__ = [
    "errors",
    "analysis",
    "face_worker",
]
