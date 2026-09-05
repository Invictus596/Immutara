//! Provider implementations.
//!
//! Concrete provider integrations live here (outside `immutara-core`, which
//! only defines the traits).
//!
//! - `mocks`: deterministic mock providers used for tests and the default
//!   dev configuration.
//! - `python_analysis`: the real face-analysis provider backed by a Python
//!   OpenCV worker (YuNet + SFace) over an NDJSON subprocess protocol.
//! - `tineye`: the real reverse-image-search provider backed by the TinEye
//!   REST API over the public web.
//! - `evm`: the real EVM attestation provider (Alloy) that submits record
//!   hashes to `contracts/src/AttestationRegistry.sol` and re-verifies the
//!   on-chain read-back against the locally recomputed hash.

pub mod evm;
pub mod mocks;
pub mod python_analysis;
pub mod tineye;

pub use evm::{EVM_PROVIDER_ID, EvmAttestationProvider};
pub use mocks::{MockAnalysisProvider, MockAttestationProvider, MockSearchProvider};
pub use python_analysis::PyAnalysisProvider;
pub use tineye::TineyeImageSearchProvider;
