"""Line-delimited JSON worker providing Immutara's real face analysis.

Protocol (v1)
-------------
The worker reads request objects from stdin, one JSON object per line
(NDJSON), and writes one JSON object per line to stdout for each request.
Diagnostics go to stderr only. A non-zero exit status signals an
unrecoverable startup/loop error.

Request
    {"schema_version": 1, "image_path": "<absolute path>"}

Response (ok)
    {
      "schema_version": 1,
      "status": "ok",
      "face_count": <int>,
      "selected_face": {
        "confidence": <float>,
        "bounding_box": {"x": int, "y": int, "width": int, "height": int}
      },
      "embedding": {"dimensions": <int>, "values": [ <float>, ... ]},
      "model": {"detector": "YuNet", "recognizer": "SFace", "version": "..."}
    }

Response (error — no faces or failed)
    {
      "schema_version": 1,
      "status": "error",
      "error": {"code": "no_face", "message": "..."}
    }

Configuration via environment:
    IMMUTARA_CV_MODELS_DIR       required, root models directory
    IMMUTARA_CV_DETECT_THRESHOLD optional, default 0.9
    IMMUTARA_CV_MAX_IMAGE_DIM    optional, default 4096
"""

import json
import os
import sys

# The models directory is mandated so the worker can fail loudly at startup if
# it is not set, rather than guessing a filesystem location.
MODELS_DIR_ENV = "IMMUTARA_CV_MODELS_DIR"
DETECT_THRESHOLD_ENV = "IMMUTARA_CV_DETECT_THRESHOLD"
MAX_IMAGE_DIM_ENV = "IMMUTARA_CV_MAX_IMAGE_DIM"

SCHEMA_VERSION = 1

from .analysis import FaceAnalyzer  # noqa: E402
from .errors import (  # noqa: E402
    CvError,
    IMAGE_UNREADABLE,
    INFERENCE_FAILED,
    INTERNAL,
    MALFORMED_REQUEST,
    MODEL_LOAD_FAILED,
    MODEL_MISSING,
    UNSUPPORTED_REQUEST,
)


def _write_line(obj) -> None:
    sys.stdout.write(json.dumps(obj, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def _err(msg: str) -> None:
    sys.stderr.write(msg + "\n")
    sys.stderr.flush()


def _box(resp_info) -> dict:
    x, y, w, h = resp_info.bounding_box
    return {"x": x, "y": y, "width": w, "height": h}


def _handle_request(analyzer: FaceAnalyzer, max_image_dim: int, req: str) -> None:
    try:
        obj = json.loads(req)
    except json.JSONDecodeError as exc:
        _write_line(
            {
                "schema_version": SCHEMA_VERSION,
                "status": "error",
                "error": {"code": MALFORMED_REQUEST, "message": f"invalid JSON: {exc}"},
            }
        )
        return

    if not isinstance(obj, dict):
        _write_line(
            {
                "schema_version": SCHEMA_VERSION,
                "status": "error",
                "error": {"code": MALFORMED_REQUEST, "message": "request must be a JSON object"},
            }
        )
        return

    if obj.get("schema_version") != SCHEMA_VERSION:
        _write_line(
            {
                "schema_version": SCHEMA_VERSION,
                "status": "error",
                "error": {
                    "code": UNSUPPORTED_REQUEST,
                    "message": f"unsupported schema_version: {obj.get('schema_version')!r}",
                },
            }
        )
        return

    image_path = obj.get("image_path")
    if not isinstance(image_path, str) or not image_path:
        _write_line(
            {
                "schema_version": SCHEMA_VERSION,
                "status": "error",
                "error": {"code": MALFORMED_REQUEST, "message": "missing image_path"},
            }
        )
        return

    result = analyzer.analyze(image_path, max_image_dim)

    if result.face_count == 0:
        _write_line(
            {
                "schema_version": SCHEMA_VERSION,
                "status": "error",
                "error": {"code": "no_face", "message": "no face detected in image"},
            }
        )
        return

    _write_line(
        {
            "schema_version": SCHEMA_VERSION,
            "status": "ok",
            "face_count": result.face_count,
            "selected_face": {
                "confidence": round(result.confidence, 6),
                "bounding_box": _box(result),
            },
            "embedding": {
                "dimensions": result.embedding_dimensions,
                "values": [float(v) for v in result.embedding.tolist()],
            },
            "model": {
                "detector": result.detector_model,
                "recognizer": result.recognizer_model,
                "version": result.model_version,
            },
        }
    )


def main(argv=None) -> int:
    models_dir = os.environ.get(MODELS_DIR_ENV)
    if not models_dir:
        _err(f"{MODELS_DIR_ENV} must be set to the models directory; aborting")
        return 1

    try:
        threshold = float(os.environ.get(DETECT_THRESHOLD_ENV, "0.9"))
        max_image_dim = int(os.environ.get(MAX_IMAGE_DIM_ENV, "4096"))
    except ValueError as exc:
        _err(f"invalid numeric configuration: {exc}")
        return 1

    try:
        analyzer = FaceAnalyzer(models_dir, score_threshold=threshold)
    except CvError as exc:
        _err(f"worker startup failed [{exc.code}]: {exc.message}")
        return 1

    for raw in sys.stdin:
        line = raw.rstrip("\n")
        if not line.strip():
            continue
        try:
            _handle_request(analyzer, max_image_dim, line)
        except CvError as exc:
            _write_line(
                {
                    "schema_version": SCHEMA_VERSION,
                    "status": "error",
                    "error": {"code": exc.code, "message": exc.message},
                }
            )
        except Exception as exc:  # noqa: BLE001
            _err(f"unexpected worker error: {exc}")
            _write_line(
                {
                    "schema_version": SCHEMA_VERSION,
                    "status": "error",
                    "error": {"code": INTERNAL, "message": "internal worker error"},
                }
            )

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
