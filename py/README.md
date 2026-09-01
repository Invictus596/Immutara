# Immutara Python Computer-Vision Component

This directory is reserved for the Python computer-vision worker that will
run as a **subprocess** communicating with Rust over a **JSON stdin/stdout
protocol** (NOT PyO3).

## Status

**Not implemented.** This is a placeholder. Real computer-vision analysis
(and the subprocess protocol) will be implemented in a later step.

## Intended protocol (future)

Rust spawns `python -m immutara_cv`, then exchanges newline-delimited JSON:

Request (Rust → Python):

```json
{"command":"analyze","evidence_id":"...","image_path":"/tmp/evidence/img.jpg"}
```

Response (Python → Rust):

```json
{"status":"ok","objects":[...],"text_regions":[...],"model_version":"0.1.0"}
```

Error:

```json
{"status":"error","error":"failed to load image"}
```

## Privacy boundary

Analysis may return object/OCR results but never biometric embeddings for
the purpose of identifying real people. Raw pixel data stays in-process.
