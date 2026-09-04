# Immutara Python Computer-Vision Worker

The real face-analysis stage of Immutara. It runs as a **subprocess** spawned
by the Rust `immutara-pipeline` (`python_analysis` provider) and communicates
over a **newline-delimited JSON (NDJSON)** protocol on stdin/stdout — no PyO3.

## Status

**Implemented** (Milestone 3): YuNet face detection + SFace embedding using
OpenCV's DNN module (CPU), plus a test suite.

## Setup

```bash
python3 -m pip install -r requirements.txt
python3 ../models/download_models.py   # fetch YuNet + SFace ONNX models
```

## Running

The Rust provider spawns the worker itself. To run standalone:

```bash
IMMUTARA_CV_MODELS_DIR=../models \
PYTHONPATH=. python -m immutara_cv.face_worker
```

Configuration via environment:

| Env var | Purpose | Default |
|---|---|---|
| `IMMUTARA_CV_MODELS_DIR` | Root models directory (required) | — |
| `IMMUTARA_CV_DETECT_THRESHOLD` | YuNet score threshold in `[0,1]` | `0.9` |
| `IMMUTARA_CV_MAX_IMAGE_DIM` | Max image dimension after downscale (`0` disables) | `4096` |

## Protocol (schema version 1)

Request (Rust → Python), one JSON object per line:

```json
{"schema_version":1,"image_path":"/abs/path/to/image.jpg"}
```

Response (Python → Rust), one JSON object per line:

```json
{
  "schema_version": 1,
  "status": "ok",
  "face_count": 1,
  "selected_face": {
    "confidence": 0.91,
    "bounding_box": {"x": 65, "y": 52, "width": 145, "height": 194}
  },
  "embedding": {"dimensions": 128, "values": [0.02, -1.42, "..."]},
  "model": {"detector": "YuNet", "recognizer": "SFace", "version": "2021dec"}
}
```

Error (no face, unreadable image, missing models, etc.):

```json
{"schema_version":1,"status":"error","error":{"code":"no_face","message":"no face detected in image"}}
```

The worker reads requests from stdin until EOF (one request per worker
spawn), writes responses to stdout, and writes diagnostics to stderr only.

## Privacy boundary

The raw 128-d embedding is returned over the ephemeral pipe so Rust can
immediately reduce it to a **SHA-256 fingerprint**. Immutara never stores,
logs, or displays raw embeddings, and nothing biometric is ever sent to a
ledger. The worker is a local, stateless inference process.

## Tests

Run the worker test suite (requires models + OpenCV in your Python):

```bash
IMMUTARA_CV_MODELS_DIR=../models python -m unittest discover -s tests -v
```

Fixtures under `tests/fixtures/` — provenance documented in
`tests/fixtures/README.md`.

## Licensing

Models come from the OpenCV Zoo (Apache-2.0): YuNet and SFace. See
`models/download_models.py` for exact sources and attribution.
