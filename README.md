# Immutara

Immutara is a **Rust-first visual provenance and evidence verification
pipeline**. It ingests visual evidence, analyzes it (computer vision
planned), runs genuine reverse-image search, evaluates it against a
configurable, versioned verification policy, and records an immutable
attestation.

Blockchain/ledgers are treated strictly as an **integrity/attestation layer**:
only content and metadata hashes are committed — never raw images, face
embeddings, or other biometric data.

> **Scope note:** Immutara is for verifying the provenance of visual
> evidence. It does **not** implement functionality to identify unknown real
> people from facial embeddings or to locate their social-media accounts.

---

## Architecture Overview

```
                immutara (binary: CLI / wiring)
               ╱        │        ╲
              ╱         │         ╲
             ▼          ▼          ▼
    immutara-tui   immutara-    immutara-
    (presenta-     pipeline     core
     tion)      (orchestration) (contracts)
                  │                 │
                  └──► core ◄───────┘
```

- **`immutara-core`** — domain types, provider traits, error hierarchy,
  configuration, and canonical serialization. Depends on nothing internal.
- **`immutara-pipeline`** — pipeline stages, orchestration, hashing,
  provenance construction, and concrete provider implementations (mocks).
  Depends only on `core`.
- **`immutara-tui`** — Ratatui presentation layer. Consumes `PipelineEvent`s
  over an `mpsc` channel, reduces them into a single derived view state, and
  renders one polished pipeline view (stages + detail panels + event log).
  Depends only on `core`; contains **no** business logic.
- **`immutara` (binary)** — CLI (`run`, `verify`, `tui`), configuration
  loading, logging, and wiring of providers into the pipeline.

`pipeline` and `tui` never depend on each other. They share the `PipelineEvent`
contract, which lives in `core`.

## Data Flow

```
evidence file → Ingest → Analyze → Search → Verify → Attest
                     │       │        │        │        │
                     └───────┴────────┴────────┴─── ＋───┘
                                             │
                                   ProvenanceChain
                                   (append-only entries)
                                             │
            PipelineEvent (mpsc) → TUI  ◄────┘
                    │
                    ▼
       canonical SHA-256 of AttestationRecord
                    │
                    ▼
              ledger (hashes only)
```

The pipeline publishes a single authoritative `PipelineEvent` stream through a
`tokio::sync::mpsc` channel consumed by the TUI.

## Crate Responsibilities

| Crate | Owns | Depends on |
|---|---|---|
| `immutara-core` | `Evidence`, `AnalysisResult`, `SearchResult`, `VerificationResult`, `AttestationRecord`, `ProvenanceChain`, provider traits, `ImmutaraError`, `Config`, `CanonicalSerialize`, `PipelineEvent` | — |
| `immutara-pipeline` | `Pipeline`, stage boundaries (ingest/analyze/search/verify/attest), SHA-256 hashing, provenance construction, mock providers | `core` |
| `immutara-tui` | `App` (derived view state), stage reducer, scrollable event log, terminal rendering | `core` |
| `immutara` (bin) | CLI subcommands, provider wiring, logging | all three |

### Provider abstraction

`immutara-core` defines the *contracts*; implementations live elsewhere:

- `AnalysisProvider` — computer-vision analysis (`opencv` real provider via a
  Python subprocess, or `mock`)
- `ImageSearchProvider` — reverse-image search (`tineye` real provider over the
  public web, or `mock`)
- `AttestationProvider` — ledger/blockchain attestation (single EVM testnet planned)

Mock implementations in `immutara-pipeline/src/providers/mocks.rs`
exercise the full pipeline deterministically. The real analysis provider lives
in `immutara-pipeline/src/providers/python_analysis.rs`; the real reverse-image
search provider lives in `immutara-pipeline/src/providers/tineye.rs`.

### Canonical serialization

`CanonicalSerialize` + `CanonicalSerializeForHashing` produce deterministic
bytes (sorted object keys, compact JSON). These exact bytes are what get
hashed (SHA-256) before on-chain attestation, making the digest reproducible
across processes and versions.

## Current Implementation Status

- [x] Cargo **workspace** with four crates (bin + 3 libs)
- [x] Core domain types (Evidence, Analysis, Search, Verification,
      Attestation, Provenance, SchemaVersion, ContentHash, EvidenceId)
- [x] Provider traits (`AnalysisProvider`, `ImageSearchProvider`,
      `AttestationProvider`) — async at I/O boundaries only
- [x] **Real face analysis** (`opencv` provider): YuNet detection + SFace
      embedding via a Python worker over an NDJSON subprocess protocol;
      only a SHA-256 fingerprint of the embedding is recorded
- [x] `ImmutaraError` via `thiserror`
- [x] `PipelineEvent` + `mpsc` event channel (contract lives in core)
- [x] Canonical serialization (deterministic, tested)
- [x] Pipeline stages: ingest / analyze / search / verify / attest
      (interfaces + deterministic `verify` evaluation)
- [x] `VerificationPolicy` evaluation (deterministic, versioned)
- [x] SHA-256 hashing
- [x] Provenance chain construction
- [x] Mock providers + end-to-end pipeline tests
- [x] Ratatui TUI: pipeline stage visualization (pending/running/completed/
      failed), evidence/analysis/search/verification/attestation detail
      panels, scrollable event log, restart (`r`), and graceful exit
- [x] CLI: `run`, `verify`, `tui` (with config-driven providers via `--config`)
- [x] Configuration skeleton (`config/default.toml`) + analysis provider
      selection (`mock` vs `opencv`)
- [x] **Real reverse-image search** (`tineye` provider): genuine upload to the
      TinEye API over the public web, real scores/URLs mapped to `SearchMatch`,
      config-driven provider selection (`mock` vs `tineye`), search failure
      non-fatal

## Not Implemented Yet (intentionally)

- **Facial recognition / biometric identification of people**
- **Blockchain / RPC integrations** and **smart contracts**
- **Image normalization** and **EXIF extraction**
- **Real multi-chain/ Solana support** — only a single EVM testnet is planned
- **GPU acceleration / CUDA** for the CV models — CPU inference only

Dependencies are added only as the corresponding functionality is implemented.

## Building & Testing

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features
cargo fmt --all
```

## Usage

```bash
immutara run -f path/to/evidence.png     # run the full pipeline
immutara verify -f path/to/evidence.png  # verify an evidence file
immutara tui -f path/to/evidence.png     # live TUI of the event stream

# All commands accept an optional config file (default: config/default.toml)
immutara run -f path/to/evidence.png -c config/my.toml
```

## Real face analysis (OpenCV provider)

By default the analysis stage uses the deterministic `mock` provider (no
external dependencies), selected via `[analysis] provider` in
`config/default.toml`.

To run **real** face analysis, set `provider = "opencv"` and satisfy the
prerequisites:

1. **Python** with OpenCV:

   ```bash
   python3 -m pip install -r py/requirements.txt
   ```

   (Only `opencv-python` is required; inference uses OpenCV's DNN module and
   runs on the CPU.)

2. **Download the models** (ONNX files are not committed to the repo):

   ```bash
   python3 models/download_models.py
   ```

   This fetches the OpenCV Zoo **YuNet** face detector
   (`face_detection_yunet_2023mar.onnx`) and **SFace** face recognizer
   (`face_recognition_sface_2021dec.onnx`, Apache-2.0) into `models/`.

3. **Configure the provider** (`config/default.toml`):

   ```toml
   [analysis]
   provider = "opencv"

   [analysis.opencv]
   models_dir = "./models"
   python_package_dir = "./py"
   python = "python3"
   worker_module = "immutara_cv.face_worker"
   detection_threshold = 0.9
   max_image_dimension = 4096
   timeout_seconds = 30
   ```

The plugin spawns `python -m immutara_cv.face_worker` and speaks a
newline-delimited JSON (NDJSON) protocol over stdin/stdout (schema version 1).
It detects faces with YuNet (primary face = highest confidence, largest area
tie-break) and embeds it with SFace (128-dimensional), then hands the result
back to Rust as a SHA-256 fingerprint of the embedding.

**Privacy boundary:** the raw embedding travels only over the ephemeral
pipe and is reduced to a SHA-256 hash immediately. Face analysis never leaves
the machine, raw embeddings are never stored, logged, or shown in the TUI, and
nothing biometric is ever sent to a ledger.

**Failure behavior:** analysis failures (missing models, unreadable/missing
image, no face, worker crash, malformed response, timeout) are non-fatal — the
pipeline records an `AnalysisFailed` event and continues, per the analysis
stage contract.

## Real reverse-image search (TinEye provider)

By default the search stage uses the deterministic `mock` provider (no
external dependencies), selected via `[search] provider` in
`config/default.toml`.

To run **real** reverse-image search, set `provider = "tineye"`:

```toml
[search]
provider = "tineye"
```

### Why TinEye

TinEye is a commercial, official reverse-image-search API over the public web.
It was chosen over alternatives (Google Custom Search / Bing / SerpAPI, which
do not accept raw image uploads; and niche similarity vendors that search your
*own* catalog rather than the public web) because it:

- accepts a **direct image upload** (multipart `image_upload`) — matching our
  local `EvidenceMetadata.source_path` input, with no need to host the image
  publicly;
- searches the **public web** (finding copies and resized variants), which is
  exactly the provenance question Immutara addresses;
- returns a **real relevance score** (0–100) per match, a canonical match URL,
  a backlink (page) URL, a rendered thumbnail, the image domain, and the crawl
  date — mapping cleanly onto `SearchMatch`;
- has a documented, stable REST + JSON interface and official client
  libraries (pytineye, Node, PHP).

### Credentials and modes

TinEye uses an API key, read from the `TINEYE_API_KEY` environment
variable (or `[search.tineye] api_key` in config). Real keys are never
committed.

There are two modes:

- **Sandbox mode** (default, `require_real_key = false`): when no key is
  configured, the provider falls back to TinEye's public sandbox key. This
  makes real HTTP requests to TinEye's API and returns genuine structured
  results — but the sandbox **always returns the same "melon cat" sample set
  regardless of the uploaded image**. This is useful for development and
  automated tests; it does NOT demonstrate a real match for your evidence
  image.

- **Real mode** (`require_real_key = true`): requires `TINEYE_API_KEY` to be
  set or `api_key` provided in config. If neither is present, the provider
  returns a configuration error instead of silently falling back to the sandbox.
  This mode performs a genuine reverse-image search over the public web for
  the actual evidence image.

### Request flow

1. The pipeline reaches the Search stage with the evidence image on disk.
2. `TineyeImageSearchProvider` reads the image bytes and does a
   `multipart/form-data` upload of the field `image_upload` to
   `POST {api_url}/search/` with header `X-API-KEY: <key>`.
3. Only the raw image bytes travel to TinEye — **the Milestone-3 facial
   embedding is never sent** to any third party.
4. TinEye returns JSON (`results.matches[]`); each match is mapped onto
   `SearchMatch` (`provider_score` normalized to [0,1], `source_url` = the
   canonical image URL on the source page, `thumbnail_url` = the rendered
   thumbnail, `first_seen` = the crawl date).
5. The existing `SearchResult` → `PipelineEvent::SearchCompleted` →
   `Verification` → TUI flow is unchanged.

### Example sandbox result

The sandbox key (used when `require_real_key = false` and no key is set)
performs a real HTTP request but always returns matches for the "melon cat"
sample image. A single match from a sandbox request looks like:

```text
provider_score: 100.0 (out of 100)
thumbnail:      https://img.tineye.com/result/d302fc...-71
domain:         archiveofsins.com
page:           https://www.archiveofsins.com/lgbt/thread/29041000/
image:          https://archiveofsins.com/data/lgbt/image/1672/86/1672861009492569.jpg
```

This is provider-returned data, never hardcoded. However, the sandbox
result is **not** a real match for any particular evidence image — it is
always the same melon-cat set.

### Run E2E

```bash
# Sandbox mode (no key needed — real HTTP, but always melon-cat results):
cargo run -q -- run -f py/tests/fixtures/face_lena.jpg \
    -c <(sed 's/provider = "mock"/provider = "tineye"/' config/default.toml)

# Real mode (genuine search for YOUR image — requires a TinEye API key):
TINEYE_API_KEY=your_key cargo run -q -- run -f path/to/evidence.jpg \
    -c <(sed 's/provider = "mock"/provider = "tineye"/' \
           -e 's/require_real_key = false/require_real_key = true/' \
           config/default.toml)
```

### Failure behavior

Search failures are non-fatal. Missing/unreadable image, no `source_path`,
network errors, timeouts, HTTP 4xx/5xx, auth errors (401/403), rate limiting
(429), malformed JSON, and empty matches all map to typed `ImmutaraError`
values; the pipeline records `SearchFailed` and continues. Multiple persistent
errors are reported; only search stage contributes nothing to verification.

### Limitations

- Sandbox key only ever returns the "melon cat" sample set, regardless of the
  uploaded image.
- Rate limits per block depend on your TinEye plan; `429`s are surfaced (and
  retried for transient network/timeout faults, but not for auth/rate-limit).
- A reported `score` of `NaN` (unavailable) is displayed as "n/a" in the TUI —
  Immutara never fabricates a provider score.
- TinEye requires a recognized image format (JPEG/PNG/WebP/GIF/BMP/AVIF/TIFF);
  animated formats are unsupported.

## Terminal UI (`immutara tui`)

`immutara tui` launches the Ratatui interface and runs the pipeline (mock or
`opencv`, per config) against the given evidence file, visualizing the result
as the `PipelineEvent` stream arrives over the `mpsc` channel.

The single-view layout shows:

- an **IMMUTARA header** with an overall run status (idle / running / complete /
  failed);
- a **PIPELINE STAGES** panel (Evidence/Ingest → Analyze → Search → Verify →
  Attest) with per-stage state — `pending`, `running`, `completed`, `failed` —
  and elapsed duration;
- a **DETAILS** panel with evidence info (id, SHA-256 hash, MIME type, size,
  dimensions when available), analysis info (provider, model, object count,
  confidence, plus the face-analysis summary: detector/recognizer models,
  faces detected, selected-face confidence, and embedding dimensionality —
  never a raw embedding), search info (provider, results, source URLs,
   provider score), verification info (policy version, individual PASS/FAIL checks,
  timestamp), and attestation info (provider, chain ID, tx hash, block number,
  status);
- a scrollable **EVENT LOG** where failures are clearly flagged.

The TUI derives all of this state directly from `PipelineEvent`s; it contains
no pipeline logic. Failures are shown inline and do not clear the state that
preceded them.

### Keyboard controls

| Key | Action |
|---|---|
| `q` / `Esc` | Quit and restore the terminal |
| `r` | Restart the pipeline (fresh run, fresh view) |
| `↑` / `↓` | Scroll the event log |
| `PageUp` / `PageDown` | Scroll the event log by a page |
| `Home` / `End` | Jump to the oldest / newest log entry |

Terminal resize is handled gracefully, and the terminal is always restored on
exit, including on error.

## Energy & Privacy Notes

- Raw pixel data, face embeddings, and other biometric content are **never**
  stored on-chain or transmitted. Only hashes are attested.
- The `py/` component and provider traits are designed so that privacy and
  provenance boundaries are enforced at the contract level.
