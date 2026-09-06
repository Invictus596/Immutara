//! Real-EVM integration tests against a local Anvil node.
//!
//! These tests perform genuine on-chain transactions. They skip (with a
//! message) when either the Anvil node is unreachable or the Foundry build
//! artifact for `AttestationRegistry` has not been generated, so
//! `cargo test --workspace` stays usable without any node. To actually run
//! them:
//!
//! ```text
//! forge build                       # from the repo root (contracts/)
//! anvil                            # terminal 1: local EVM on :8545
//! cargo test -p immutara-pipeline --test evm_integration   # terminal 2
//! ```
//!
//! The RPC endpoint can be overridden with `IMMUTARA_TEST_EVM_RPC` (defaults
//! to `http://127.0.0.1:8545`).

use std::path::PathBuf;

use alloy::network::TransactionBuilder;
use alloy::primitives::{Address, B256, keccak256};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::rpc::types::TransactionRequest;
use alloy::signers::local::PrivateKeySigner;
use chrono::Utc;
use immutara_core::AttestationProvider;
use immutara_core::config::EvmChainConfig;
use immutara_core::domain::attestation::{AttestationRecord, BlockchainVerification};
use immutara_core::domain::evidence::{ContentHash, EvidenceId, SchemaVersion};
use immutara_pipeline::hashing::{derive_attestation_id, record_hash, sha256_hex};
use immutara_pipeline::providers::evm::{EvmAttestationProvider, verify_read_back};

/// Standard Anvil dev account 0. The key is public and intended for local
/// development; production keys must only ever come from an env variable.
const ANVIL_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const KEY_ENV: &str = "IMMUTARA_INTEGRATION_TEST_EVM_KEY";

fn rpc_url() -> String {
    std::env::var("IMMUTARA_TEST_EVM_RPC").unwrap_or_else(|_| "http://127.0.0.1:8545".to_string())
}

fn artifact_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/out/AttestationRegistry.sol/AttestationRegistry.json")
}

fn record(content: &str) -> AttestationRecord {
    AttestationRecord {
        schema_version: SchemaVersion(1),
        pipeline_version: "0.1.0".to_string(),
        evidence_id: EvidenceId::new(),
        content_hash: ContentHash(sha256_hex(content.as_bytes())),
        metadata_hash: ContentHash(sha256_hex(format!("meta:{content}").as_bytes())),
        verification_result_hash: ContentHash(sha256_hex(format!("verify:{content}").as_bytes())),
        search_result_hash: ContentHash(sha256_hex(format!("search:{content}").as_bytes())),
        verification_policy_version: SchemaVersion(1),
        provider_id: "evm".to_string(),
        chain_id: "31337".to_string(),
        attested_at: Utc::now(),
    }
}

/// Load the deployed contract bytecode from the Foundry artifact.
fn read_artifact_bytecode(path: &PathBuf) -> Option<Vec<u8>> {
    let raw = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let object = value.get("bytecode")?.get("object")?.as_str()?;
    alloy::hex::decode(object).ok()
}

/// Build a provider under test, configured for the running node.
fn provider_under_test(contract_address: Address) -> EvmAttestationProvider {
    unsafe {
        std::env::set_var(KEY_ENV, ANVIL_KEY);
    }
    let config = EvmChainConfig {
        rpc_url: rpc_url(),
        chain_id: 31337,
        contract_address: format!("{contract_address:#x}"),
        private_key_env: KEY_ENV.to_string(),
        timeout_seconds: 60,
        confirmations: 1,
    };
    EvmAttestationProvider::new(config).expect("provider config valid")
}

#[tokio::test]
async fn real_anvil_attest_reverify_and_tamper_detection() {
    let Some(bytecode) = read_artifact_bytecode(&artifact_path()) else {
        eprintln!("SKIPPED: forge artifact not found (run `forge build` in contracts/)");
        return;
    };
    let wallet: PrivateKeySigner = ANVIL_KEY.parse().expect("valid dev key");
    let rpc = rpc_url();
    let provider = match ProviderBuilder::new()
        .wallet(wallet.clone())
        .connect(&rpc)
        .await
    {
        Ok(p) => p,
        Err(e) => {
            eprintln!("SKIPPED: no EVM node at {} ({e})", rpc_url());
            return;
        }
    };
    let chain_id = provider.get_chain_id().await.expect("chain id");
    assert_eq!(chain_id, 31337, "test assumes the default Anvil chain id");

    // ---- Deploy the AttestationRegistry ----
    let deployed = provider
        .send_transaction(
            TransactionRequest::default()
                .with_from(wallet.address())
                .with_deploy_code(bytecode)
                .with_gas_limit(1_500_000),
        )
        .await
        .expect("deploy tx accepted");
    let deploy_receipt = deployed.get_receipt().await.expect("deploy mined");
    let contract = deploy_receipt
        .contract_address
        .expect("deploy produced contract address");
    println!("[integration] deployed AttestationRegistry at {contract}");
    println!(
        "[integration] deployment tx {} in block {}",
        deploy_receipt.transaction_hash,
        deploy_receipt.block_number.expect("mined")
    );

    let evm = provider_under_test(contract);

    // ---- Happy path: attest + on-chain re-verification ----
    let rec = record("pristine-evidence");
    let local_hash = record_hash(&rec).expect("local record hash");
    let anchor_id = derive_attestation_id(&rec.content_hash);

    let receipt = evm.attest(&rec).await.expect("attest succeeds");
    assert_eq!(
        receipt.blockchain_verification,
        BlockchainVerification::Verified,
        "fresh anchor must verify"
    );
    assert!(receipt.tx_hash.starts_with("0x"), "tx hash present");
    assert!(receipt.block_number >= 1, "block number present");
    assert_eq!(
        receipt.contract_address.to_lowercase(),
        format!("{contract:#x}").to_lowercase()
    );
    assert_eq!(receipt.attestation_id, format!("0x{anchor_id:x}"));
    assert_eq!(receipt.on_chain_record_hash, format!("0x{local_hash:x}"));

    // Independent re-verification (no submission).
    assert_eq!(
        evm.verify_anchor(&rec).await.expect("read back"),
        BlockchainVerification::Verified
    );

    // ---- Tamper test: one field changed -> local hash floats, chain stays ----
    let mut tampered = rec.clone();
    tampered.pipeline_version = "TAMPERED".to_string();
    let tampered_hash = record_hash(&tampered).expect("tampered record hash");
    assert_ne!(tampered_hash, local_hash, "tamper must change the hash");
    assert_eq!(
        verify_read_back(tampered_hash, local_hash),
        BlockchainVerification::Failed,
        "tampered local copy must NOT be considered verified"
    );
    // Same anchor id (same content hash), read-back against genuinely stored
    // data also fails because the local record no longer matches.
    assert_eq!(
        evm.verify_anchor(&tampered)
            .await
            .expect("read back tampered"),
        BlockchainVerification::Failed
    );

    // ---- Read-back failure: an unset slot returns zeroes -> mismatch ----
    let ghost = ContentHash(sha256_hex(b"never-attested-content"));
    let ghost_id = derive_attestation_id(&ghost);
    assert_ne!(ghost_id, anchor_id);
    let ghost_record = record("never-attested-content");
    assert_eq!(
        evm.verify_anchor(&ghost_record)
            .await
            .expect("read back ghost slot"),
        BlockchainVerification::Failed,
        "an anchor that was never set must read back as a mismatch"
    );

    // ---- Read-back failure: corrupted storage -> mismatch ----
    // Directly overwrite the anchor slot with a bogus hash, bypassing the
    // provider, then re-verify: the corrupted chain data must FAIL. The nonce
    // is fetched fresh because the deploy provider's cached nonce is stale by
    // now (the attest transaction used another provider).
    let nonce = provider
        .get_transaction_count(wallet.address())
        .await
        .expect("current nonce");
    let bogus = B256::from([0xeeu8; 32]);
    let mut overwrite = Vec::with_capacity(4 + 64);
    overwrite.extend_from_slice(&keccak256(b"attest(bytes32,bytes32)")[..4]);
    overwrite.extend_from_slice(anchor_id.as_slice());
    overwrite.extend_from_slice(bogus.as_slice());
    let tx = provider
        .send_transaction(
            TransactionRequest::default()
                .with_from(wallet.address())
                .with_to(contract)
                .with_nonce(nonce)
                .with_input(overwrite),
        )
        .await
        .expect("overwrite tx accepted");
    let over_receipt = tx.get_receipt().await.expect("overwrite mined");
    assert!(over_receipt.status(), "overwrite tx mined successfully");
    assert_eq!(
        evm.verify_anchor(&rec)
            .await
            .expect("read back corrupted slot"),
        BlockchainVerification::Failed,
        "corrupted chain storage must FAIL re-verification"
    );

    println!("✓ attest, read-back, tamper, corrupt-storage scenarios all pass");
}

#[tokio::test]
async fn rpc_unavailable_returns_provider_error_without_panicking() {
    let wallet: PrivateKeySigner = ANVIL_KEY.parse().expect("valid dev key");
    // A port that nothing listens on: connection refused. from_signer bypasses
    // the environment-variable read so this test is self-contained.
    let evm = EvmAttestationProvider::from_signer(
        EvmChainConfig {
            rpc_url: "http://127.0.0.1:1".to_string(),
            chain_id: 31337,
            contract_address: format!("{:#x}", Address::ZERO),
            private_key_env: String::new(),
            timeout_seconds: 10,
            confirmations: 1,
        },
        wallet,
    );
    let res = evm.attest(&record("rpc-unavailable")).await;
    assert!(
        res.is_err(),
        "unreachable RPC must surface as a provider error"
    );
    let err = res.err().unwrap();
    let msg = err.to_string();
    assert!(msg.contains("evm"), "error names the provider: {msg}");
}
