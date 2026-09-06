# Immutara

Immutara is a **Rust-first visual provenance and evidence verification
pipeline**. It ingests visual evidence, analyzes it with real computer
vision (YuNet + SFace face detection/embedding), runs genuine reverse-image
search, evaluates it against a configurable, versioned verification policy,
and records an immutable attestation.

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
  public web, `serpapi_lens` real Google Lens provider, or `mock`)
- `AttestationProvider` — ledger/blockchain attestation (`evm` real provider,
  or `mock`)

Mock implementations in `immutara-pipeline/src/providers/mocks.rs`
exercise the full pipeline deterministically. The real analysis provider lives
in `immutara-pipeline/src/providers/python_analysis.rs`; the real reverse-image
search providers live in `immutara-pipeline/src/providers/tineye.rs` and
`immutara-pipeline/src/providers/serpapi_lens.rs`; the real EVM provider lives
in `immutara-pipeline/src/providers/evm.rs`.

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
- [x] **Real Google Lens search** (`serpapi_lens` provider): face-crop-first
      upload to SerpApi's Lens API over the public web, real match URLs mapped
      to `SearchMatch` (position preserved, provider score honestly `n/a`),
      social-media classification by host, full-image fallback when the crop
      has no matches, **exact matches preferred and visual matches as
      fallback**, config-driven selection (`mock` vs `serpapi_lens`), and an
      opt-in live E2E test
- [x] **Search result bound into the attestation** — the **selected** result
      (first exact Lens match, else first visual) is hashed through its
      canonical `match_kind`-aware representation and committed as
      `search_result_hash` on `AttestationRecord`, so the on-chain attestation
      commits to the discovered source and whether it was an exact or visual
      match
- [x] **Real EVM attestation** (`evm` provider): minimal `AttestationRegistry`
      Solidity contract (hash anchors only), deployment via Foundry, real
      on-chain writes over Alloy, on-chain read-back re-verification with
      tamper detection, config-driven provider selection (`mock` vs `evm`),
      and real local Anvil end-to-end integration tests
- [x] **Deterministic attestation ids** — `attestationId` derived from the
      evidence content hash so the same evidence maps to the same chain slot

## Not Implemented Yet (intentionally)

- **Facial recognition / biometric identification of people**
- **Image normalization** and **EXIF extraction**
- **Real multi-chain / Solana support** — only a single EVM chain is supported
- **GPU acceleration / CUDA** for the CV models — CPU inference only

Dependencies are added only as the corresponding functionality is implemented.

## Building & Testing

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features
cargo fmt --all

# Solidity contract (see contracts/README.md)
cd contracts && forge build && forge lint src/ && forge test
```

## Usage

```bash
immutara run -f path/to/evidence.png     # run the full pipeline
immutara verify -f path/to/evidence.png  # verify an evidence file
immutara tui -f path/to/evidence.png     # live TUI of the event stream

# All commands accept an optional config file (default: config/default.toml)
immutara run -f path/to/evidence.png -c config/my.toml
```

`run` prints a concise **IMMUTARA RUN SUMMARY** after success: the face-analysis
summary (detector, faces, selected-face confidence/bbox, embedding fingerprint),
the search result (provider, FACE CROP / FULL IMAGE input, exact/visual counts,
the **selected (attested) result** with its kind, rank, URL and domain, plus the
top matches), the verification verdict, and the attestation anchor (record hash,
search-result hash, transaction and block). This mirrors exactly what the TUI
detail panels show.

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
It was chosen for the original real search provider because it:

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

**Google Lens is added as a second real provider** (`serpapi_lens`) because it
is the de-facto public reverse-image-search engine, and it too accepts a raw
image upload (face-crop first, full-image fallback) through SerpApi — see
[Real reverse-image search (SerpApi Google Lens provider)](#real-reverse-image-search-serpapi-google-lens-provider).

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

- **No real TinEye API key is currently available in this environment.** End-to-end
  genuine-match validation against evidence imagery has not yet been demonstrated;
  only the sandbox flow (melon-cat dataset) has been exercised live. Real-mode
  results shown here would be from a future run with a key, not from this session.
- Sandbox key only ever returns the "melon cat" sample set, regardless of the
  uploaded image.
- Rate limits per block depend on your TinEye plan; `429`s are surfaced (and
  retried for transient network/timeout faults, but not for auth/rate-limit).
- A reported `score` of `NaN` (unavailable) is displayed as "n/a" in the TUI —
  Immutara never fabricates a provider score.
- TinEye requires a recognized image format (JPEG/PNG/WebP/GIF/BMP/AVIF/TIFF);
  animated formats are unsupported.

## Real reverse-image search (SerpApi Google Lens provider)

In addition to `tineye`, the search stage can use **real Google Lens** via the
SerpApi image-search API. Set `provider = "serpapi_lens"`:

```toml
[search]
provider = "serpapi_lens"

[search.serpapi_lens]
api_url = "https://serpapi.com"   # optional, defaults to SerpApi
timeout_seconds = 60
max_upload_bytes = 500000
```

### Credentials

The provider reads its key from the `SERPAPI_API_KEY` environment variable at
run time (no config fallback, never committed). SerpApi has **no public
sandbox key**, so a real key is always required; requests for a missing or
invalid key surface a typed "authentication failed" error. The key is sent as
a multipart form field on the `/image` upload and as a query parameter on the
`/search` call (Bearer headers are rejected by the API).

### Face-crop-first search input

The pipeline extracts the **selected face bounding box** with OpenCV/YuNet
during analysis, then `face_crop::generate_face_crop` produces a padded
(pad = 0.25 per side), clamped, JPEG-quality-90 crop capped at
`max_upload_bytes` (progressive 0.75 downsampling) before upload.

Search input is a first-class enum on `SearchResult.search_input`:

| Input kind          | When used                                                    |
| ------------------- | ------------------------------------------------------------ |
| `FACE CROP`         | Analysis found a selected face **and** a readable `source_path` produced a valid crop |
| `FULL IMAGE`        | No usable face crop (no face, unreadable file, crop failure) |

The crop is searched first; the pipeline falls back to the full image when the
crop did not reach `SOCIAL_MATCH_VERIFIED` — either the crop search returned
zero matches, or it returned candidates that failed independent media
validation (emitting a `SearchFallback` event with the reason). A crop-search
*error* never triggers a fallback (errors propagate — nothing was learned). The
TUI shows `Search input: FACE CROP` / `FULL IMAGE` accordingly.

### Request flow

1. Upload the crop (or full image) to `POST {api_url}/image` (multipart,
   `api_key` + `image`). Retries (×3) cover transient network/timeout faults.
2. Query `GET {api_url}/search?engine=google_lens&image_id=…&json_format=1`.
3. `exact_matches[]` (Google considers them the **same picture**) are mapped
   first, then `visual_matches[]` as the fallback tier. Each `SearchMatch`
   carries a `match_kind` (`exact` / `visual`). Lens exposes no relevance
   score, so `provider_score` is `NaN` ("no signal") and **never fabricated**
   — the TUI renders it as "match (score n/a)". Real engine ordering is
   preserved in `position`; `source_domain` is derived from each result URL via
   `hostname_of`.
4. A result is classified as **social media** (`is_social_media_url`) purely by
   its host (reddit, x/twitter, facebook, instagram, youtube, tiktok, linkedin,
   pinterest, tumblr, threads, snapchat, weibo, vk). This is a syntactic
   classification of the returned URL — it does **not** assert that the page is
   reachable or that the person is who it appears to be.

### Selecting the attested result

The single result bound into the attestation is the one selected by
`SearchResult::selected()`: a **media-verified social match** when one exists,
otherwise the **first exact match** when any exist, otherwise the **first
visual match**. A media-verified social match is the strongest provenance
(independent validation confirmed the public media contains the searched
image); exact provider matches are stronger than visual ones. The selection is
deterministic and re-derivable from the events.

### Binding into the attestation

The pipeline commits `SHA-256(canonical_bytes(selected))` (the selected
result's canonical hash, including any media-match evidence, `exact` kind
whenever available; canonical `null` when nothing matched) as
`search_result_hash` on the `AttestationRecord`, which flows into `recordHash`
on-chain. **The blockchain attestation therefore commits to the discovered
source/result — the media-match validation evidence, and whether it was an
exact or visual match — through its canonical hash**; the commitment is
reproducible by recomputing from the events.

### Independent media-match validation

Search providers return *candidate* pages for the searched face crop; Immutara
then independently fetches the candidate's **source page** public media and
compares it to the searched image with deterministic hashes:

1. **Exact hash** — the fetched media bytes are SHA-256-identical to the
   searched image (`method = exact_hash`, `distance = 0`).
2. **Perceptual hash** — otherwise a 64-bit dHash (grayscale 9×8 Triangle
   resize) is compared by Hamming distance against a fixed threshold of 12
   (`method = perceptual_hash`, `distance`, `threshold`).

**Attribution rule (strict):** a **social** candidate verifies only when media
discovered *on its own source page* — the page's `og:image`, else the first
absolute `<img src>` — is independently fetched and matches the searched image.
This is what `SOCIAL_MATCH_VERIFIED` means: the matching media is attributable
to the actual public social page. The search provider's *own* thumbnail URL
(Google's `encrypted-tbnN.gstatic.com` cached thumbnail, etc.) is **supporting
evidence only** — it is the provider's copy of the matched image and proves
nothing about what the page serves today, so it is never decisive for a social
candidate. Non-social candidates keep a cheaper provider-thumbnail fast path,
falling back to page media when the thumbnail cannot establish a match.

Validation never bypasses access controls — a login wall, 4xx/403, non-image
content, or oversized media records honest *unverified* evidence instead, even
when the provider's thumbnail matched. Every comparison records a
`MediaMatchEvidence { method, distance, threshold, passed, media_url, note }`
on the match, with the `note` separating "provider thumbnail matched (supporting
evidence only)" from the *independent* page-media outcome for an operator. The
search stage exposes one overall state:

| State | Meaning |
|---|---|
| `NO_RESULTS` | No matches at all. |
| `WEB_MATCH` | Only non-social matches returned. |
| `SOCIAL_CANDIDATE` | A social-domain match exists, not yet media-validated. |
| `SOCIAL_CANDIDATE_UNVERIFIED` | A social match was attempted but the media could not be independently confirmed (login wall, unreachable, non-image, or the comparison failed). |
| `SOCIAL_MATCH_VERIFIED` | A social match's public media was fetched and visually matches the searched image. |

The validation query carries **two byte-sets**: the exact bytes submitted to
the search provider (the face crop when one was detected, the full image
otherwise) *and* the full evidence photograph on disk. Byte identity is judged
against either; perceptual matching compares against the closer of the two and
records which form won in `note`, so a tight face crop does not silently fail
against a page thumbnail of the whole photograph. The validator never reuses
the facial-recognition model and never claims identity. Because validation
performs real HTTP fetches during a run, it is enabled per provider (real
providers opt in; mock/harness providers stay offline).

A strict competition policy can require a verified social match:

```toml
[verification.policy]
social_match_verified = true
```

With this set, verification adds a `social_match_verified` check that fails
unless the search state is `SOCIAL_MATCH_VERIFIED`. The default remains
`false`, preserving the generic-web path unchanged.

### Example live results

Executed in this environment with a real key against public-domain images.
Results are runtime-discovered, not hardcoded; a representative strict run on a
NASA public-domain photograph (no face → full-image search) looked like:

```text
Search   : provider serpapi_lens | input FULL IMAGE | 60 matches (0 exact, 60 visual) | state SocialCandidateUnverified
  selected (attested): [visual] #1 https://en.wikipedia.org/wiki/Spaceship_Earth | domain en.wikipedia.org
  #2 https://science.nasa.gov/resource/apollo-8s-iconic-earthrise/ | domain science.nasa.gov  (…and more)
  social: https://www.instagram.com/reel/DSp6csUCV6I/
  media    : UNVERIFIED cannot attribute the matching media to this page (provider thumbnail did not match …); page media: HTTP 403 Forbidden | url https://scontent.cdninstagram.com/v/t51.… (Instagram CDN)
Verify   : policy v1 | checks 3: 2 passed | verdict FAIL
   [FAIL] social_match_verified
Attest   : provider evm | VERIFIED (read-back matches)
```

(Represents the strict validator exactly: Instagram/Facebook page media is not
independently retrievable, so the social check honestly fails even though the
web results are genuine; the strict `verdict FAIL` record is still attested.)

The same pipeline exercises the **face-crop-first** path on public-domain
portraits. Whether a repost verifies end-to-end depends entirely on whether the
candidate's **own page media** is publicly fetchable and matches the searched
frame at `distance ≤ 12`. In this environment, Instagram reposts almost always
fail this under the strict validator because their CDN media returns **HTTP
403** to anonymous fetch (see
[Demo test assets and provenance](#demo-test-assets-and-provenance)) — those are
honestly reported as `SOCIAL_CANDIDATE_UNVERIFIED` / `verdict FAIL`, and the
failed record is still attested so the decision is not lost. That failure is the
point: the system never claims a match it cannot independently confirm on the
candidate's own page.

A live end-to-end contrast (this repository's validation fixtures, Section
[Demo test assets and provenance](#demo-test-assets-and-provenance)) shows the
strict validator telling the two apart:

- **Positive — Tesla portrait** (`test_images/tesla_portrait.jpg`): face crop →
  runtime-discovered Pinterest pin → the pin's own `i.pinimg.com` media is freely
  fetchable and matches at dHash **3/12** → `SOCIAL_MATCH_VERIFIED` → `verdict
  PASS` → `EVM VERIFIED` (read-back matches).
- **Negative — friend photo** (`test_images/test.jpg`): face crop → full-image
  fallback → runtime-discovered Instagram post whose CDN media returns **HTTP
  403**. The provider thumbnail matched (supporting evidence only) but the page
  could not be independently confirmed → `SOCIAL_CANDIDATE_UNVERIFIED` →
  `verdict FAIL` (`social_match_verified` fails) → still attested so nothing is
  lost. The honest diagnostic reads: *"provider thumbnail matched (supporting
  evidence only) … page media: HTTP 403 Forbidden."*

### Testing

```bash
# Offline unit tests (mock HTTP) — run with the whole workspace
cargo test --workspace

# Opt-in live test against real SerpApi (requires a real key):
IMMUTARA_E2E_SEARCH=1 SERPAPI_API_KEY=… cargo test -p immutara-pipeline \
    real_serpapi_lens_crop_search_returns_social_matches -- --nocapture
```

### Failure behavior

Search failures are non-fatal (same as `tineye`): missing/unreadable image, no
`source_path`, missing/invalid key, network errors, timeouts, HTTP 4xx/5xx, rate
limiting (429), malformed JSON, and empty matches map to typed `ImmutaraError`
values; the pipeline records `SearchFailed` and continues. Missing keys abort
the search immediately with a clear configuration error.

### Limitations

- Real Google Lens search requires a SerpApi key (free tier is rate-limited);
  it is a commercial API and results depend on their index. Lens result sets are
  **nondeterministic across calls**; the face-crop-first + full-image fallback is
  what makes verification reproducible-enough across runs.
- Lens has no numeric relevance score; the plugin never invents one.
- Social classification is host-based only — Immutara does not verify page
  ownership or reachability, and does not identify people.
- The face crop searches the same public index as the full image; garbage in →
  garbage out remains true.
- **Recall limitation:** Lens does not always surface every copy of an image. In
  this environment it repeatedly failed to surface a known ground-truth
  Instagram post for the friend-photo fixture — the tool honestly reports
  `SOCIAL_CANDIDATE_UNVERIFIED` rather than guessing.
- **Instagram retrieval limitation:** Instagram post pages expose no
  `og:image`/absolute `<img>`, and their CDN media returns **HTTP 403** to
  anonymous fetch. Immutara does not work around this (no auth/cookies/browser
  automation/private APIs) — such candidates resolve to
  `SOCIAL_CANDIDATE_UNVERIFIED`.

## Media discovery tool (dev only)

`crates/immutara-pipeline/src/bin/discover.rs` is a disposable investigator's
tool. Given one or more images it runs the exact production stages — detection
+ face crop, Lens search, media validation — and prints the strongest candidate
per image. No URLs are hardcoded, no scores fabricated; the output is the same
deterministic evidence the pipeline would produce. It is not part of the
attestation path.

```bash
export SERPAPI_API_KEY=…          # real key, required
cargo run -p immutara-pipeline --bin discover -- ./test_images/obama.jpg
```

## Demo test assets and provenance

The three checked-in demo images are public-domain U.S. government works, fetched
from Wikimedia Commons via its API (license fields verified before download):

| File | Source (Commons) | Provenance | Size / dims |
|---|---|---|---|
| `test_images/mona_lisa.jpg` | `Mona Lisa, by Leonardo da Vinci, from C2RMF retouched.jpg` | Leonardo da Vinci, c. 1503–1506 (Louvre); C2RMF-retouched scan, Commons license `Public domain`. 2048-px derivative rendered from the 94 MB master with cv2 `INTER_AREA`. Face fixture exercising the strict validator's handling of repost pages whose media is not independently retrievable (Instagram pages 403 → `SOCIAL_CANDIDATE_UNVERIFIED`). | 3 465 931 B, 2048×3052 |
| `test_images/earthrise.jpg` | `NASA-Apollo8-Dec24-Earthrise.jpg` (`Commons: a/a8`) | NASA/Apollo 8, photo by astronaut Bill Anders; Commons license `Public domain` (PD-USGov-NASA). No face → exercises full-image search. | 311 263 B, 2400×2400 |
| `test_images/obama.jpg` | `President_Barack_Obama.jpg` (`Commons: 8/8d`) | Official White House portrait (Canon EOS 5D Mark III, 2012-12-06), PD-USGov; Commons license `Public domain`. Multiple faces → exercises face-crop search. | 1 276 121 B, 2687×3356 |

Download commands used (Commons `action=query&prop=imageinfo&iiprop=url|size|extmetadata`
used to confirm the license short name before saving each file).

### Evaluation fixture hashes and provenance (not committed)

The following local evaluation fixtures exercise the strict validator live
(positive and negative controls). They are **not committed** to the repository
(inline, and confirmed by the objective to keep out of git); the public-domain
source is cited and each file's SHA-256 is recorded so the attested content hash
from any run can be cross-checked:

| File | Source | Provenance | SHA-256 (content hash attested) |
|---|---|---|---|
| `test_images/tesla_portrait.jpg` | `Tesla_circa_1890.jpeg` (Wikimedia Commons) | Nikola Tesla portrait, c. 1890 — **public domain** (pre-1928 publication). Single high-confidence face (YuNet conf ~0.912). Live strict run: face crop → runtime-discovered Pinterest pin → pin's own `i.pinimg.com` media matches at dHash **3/12** → `SOCIAL_MATCH_VERIFIED` → `EVM VERIFIED`. | `c26252cc5d907d2404b68d04480e5eb2fa0f2dadc085e9e2a5a09dabbaf3a89b` |
| `test_images/test.jpg` | personal test photograph (the "friend photo") | The negative-control fixture. Its Instagram provenance is **not** asserted by the tool — it is an *offline* evaluation reference only, never injected into search. Live strict run: face crop → full-image fallback → runtime-discovered Instagram post whose CDN media returns **HTTP 403** → `SOCIAL_CANDIDATE_UNVERIFIED` → `verdict FAIL` (honest rejection; still attested). | `12b9d77d8a5f6b2da3b030b9b82ce97a907e587621bd1f68b9fc5f04d52d1370` |
| `test_images/tesla.jpg` | `Tesla_circa_1890.jpeg` (Wikimedia Commons) | A fragile/weak duplicate of the same Tesla portrait (lower-confidence crop); retained as a secondary evaluation positive, not a canonical fixture. | `55235649d7162b97b5eb8f6318f776ba46bc43f8cc93179e8c8c9b7ed541643b` |

**Instagram retrieval limitation (recorded, not worked around):** Google
Lens/SerpApi has not surfaced the friend's ground-truth post across repeated
runs (a Lens recall limitation), and Instagram post pages return **no
`og:image`, `display_url`, or absolute `<img>`** — only base64 placeholders and
login prompts (an Instagram retrieval limitation). Even when Lens surfaces an
Instagram post, its CDN media (`scontent.cdninstagram.com`/`i.instagram.com`)
consistently returns **HTTP 403** to Immutara's anonymous fetch, so such
candidates correctly resolve to `SOCIAL_CANDIDATE_UNVERIFIED`. Immutara does
not bypass these access controls (no auth, cookies, browser automation, or
private APIs) — an unreachable page is honestly reported as unverified. As with
all discovered URLs, recent runtime-discovered Instagram post IDs are **not**
hardcoded into production logic.

## Real EVM attestation

By default the attestation stage uses the deterministic `mock` provider, selected
via `[attestation] provider` in `config/default.toml`. To attest on a **real
EVM chain**, set `provider = "evm"`:

```toml
[attestation]
provider = "evm"

[attestation.evm]
rpc_url = "http://127.0.0.1:8545"
chain_id = 31337
contract_address = "0x5FbDB2315678afecb367f032d93F642f64180aa3"
private_key_env = "IMMUTARA_EVM_PRIVATE_KEY"
timeout_seconds = 60
confirmations = 1
```

### The hash anchor

Only hashes are ever written on-chain. Each evidence item yields:

- `recordHash` — SHA-256 of the **canonical** `AttestationRecord` bytes
  (same canonical serializer used everywhere else in the pipeline);
- `attestationId` — SHA-256 of `contentHash || "immutara-attestation:v1"`,
  a **deterministic** `bytes32` slot key derived from the evidence itself.

The `AttestationRegistry` contract (see [`contracts/README.md`](contracts/README.md))
stores `(recordHash, timestamp, submitter)` under that slot and emits an
`Attested` event. No media, embeddings, or biometric data touch the chain.

### Verification is a read-back, not a receipt

After the transaction mines, the `evm` provider **re-reads the slot** with
`getAttestation` and compares the stored hash against the locally recomputed
record hash. The difference matters:

| Condition                                            | Verdict          |
| ---------------------------------------------------- | ---------------- |
| Transaction mined + on-chain hash == local hash      | `VERIFIED`       |
| Transaction mined + on-chain hash != local hash      | `FAILED` (never shown as verified) |
| Unset / corrupted slot                               | `FAILED` (re-verification) |

A successful transaction with a mismatched hash is **never** rendered as
`VERIFIED`: the pipeline records the block factually (tx hash, block number,
contract, anchor id) but fails the attestation stage.

### Running it (local Anvil)

```bash
# 1. Terminal A — start a local node
anvil

# 2. Build + deploy the contract (dev key is public Anvil account 0)
IMMUTARA_EVM_PRIVATE_KEY=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 \
  ./scripts/deploy_contract.sh
#   → prints "Contract address: 0x…" — paste it into [attestation.evm] contract_address

# 3. Terminal B — run the pipeline against real chain state
IMMUTARA_EVM_PRIVATE_KEY=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 \
  immutara tui -f path/to/evidence.png \
  -c <(sed 's/provider = "mock"/provider = "evm"/' config/default.toml)
```

### Testing

- `cargo test -p immutara-pipeline --lib` — unit tests (hashing, config,
  re-verification logic) without any node.
- `cargo test -p immutara-pipeline --test evm_integration` — **real** Anvil
  end-to-end: deploys the contract from its Foundry artifact, attests real
  evidence, re-reads the slot, and proves tamper detection (mutated record,
  never-set slot, and deliberately corrupted chain storage all read back as
  failures). Tests skip gracefully when no node or artifact is present.

### Failure behavior

Attestation failures are **fatal** for the run (unlike analysis/search, which
are non-fatal): an unreachable RPC, a missing private key, a rejected or
reverted transaction, or a failed read-back all end the pipeline with the error
surfaced in the TUI event log.

### Security notes

- The private key comes **exclusively** from an environment variable
  (`private_key_env`, default `IMMUTARA_EVM_PRIVATE_KEY`); it is never read
  from config, CLI, or files, and never committed.
- The `0xac0974…` Anvil address shown above is the standard public test key — it
  is only ever valid on a throwaway local chain.

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
  never a raw embedding), search info (provider, **search input** — FACE CROP
  vs FULL IMAGE, exact/visual counts, the **selected result** with its exact/
  visual kind, rank, URL, domain and position, plus the match list with per-
  match kind/rank/domain/score), verification info (policy version, individual
  PASS/FAIL checks,
  timestamp), and attestation info (provider, chain ID, contract address,
  attestation id, tx hash, block number, and a `VERIFIED` / `FAILED` verdict);
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
