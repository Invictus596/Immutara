"""Structured error codes for the CV worker.

These codes are stable identifiers surfaced to the Rust host as the
``error.code`` field of an error response, so it can map failures to typed
`ImmutaraError` variants without string matching.
"""


class CvError(Exception):
    """A typed worker failure carrying a stable machine-readable code."""

    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code
        self.message = message


# Stable error codes.
MODEL_MISSING = "model_missing"
MODEL_LOAD_FAILED = "model_load_failed"
IMAGE_UNREADABLE = "image_unreadable"
INFERENCE_FAILED = "inference_failed"
MALFORMED_REQUEST = "malformed_request"
UNSUPPORTED_REQUEST = "unsupported_request"
INTERNAL = "internal"
