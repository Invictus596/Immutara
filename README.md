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
  over an `mpsc` channel and renders them. Depends only on `core`; contains
  **no** business logic.
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
| `immutara-tui` | `App`, event loop, screens, terminal rendering | `core` |
| `immutara` (bin) | CLI subcommands, provider wiring, logging | all three |

### Provider abstraction

`immutara-core` defines the *contracts*; implementations live elsewhere:

- `AnalysisProvider` — computer-vision analysis (Python subprocess planned)
- `ImageSearchProvider` — reverse-image search (genuine APIs planned)
- `AttestationProvider` — ledger/blockchain attestation (single EVM testnet planned)

Mock implementations in `immutara-pipeline/src/providers/mocks.rs`
exercise the full pipeline deterministically.

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
- [x] `ImmutaraError` via `thiserror`
- [x] `PipelineEvent` + `mpsc` event channel (contract lives in core)
- [x] Canonical serialization (deterministic, tested)
- [x] Pipeline stages: ingest / analyze / search / verify / attest
      (interfaces + deterministic `verify` evaluation)
- [x] `VerificationPolicy` evaluation (deterministic, versioned)
- [x] SHA-256 hashing
- [x] Provenance chain construction
- [x] Mock providers + end-to-end pipeline tests
- [x] Ratatui TUI shell (renders `PipelineEvent` stream)
- [x] CLI: `run`, `verify`, `tui`
- [x] Configuration skeleton (`config/default.toml`)

## Not Implemented Yet (intentionally)

- **Real computer vision** — no object detection / OCR / face analysis
- **Facial recognition / biometric identification of people**
- **Genuine reverse-image search integrations** (external APIs)
- **Blockchain / RPC integrations** and **smart contracts**
- **Image normalization** and **EXIF extraction**
- **Python subprocess worker** (planned protocol in `py/README.md`)
- **Provider config wiring** — all providers are mocks in this phase
- **Real multi-chain/ Solana support** — only a single EVM testnet is planned

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
```

## Energy & Privacy Notes

- Raw pixel data, face embeddings, and other biometric content are **never**
  stored on-chain or transmitted. Only hashes are attested.
- The `py/` component and provider traits are designed so that privacy and
  provenance boundaries are enforced at the contract level.
