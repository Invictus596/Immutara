//! Real EVM attestation provider built on Alloy.
//!
//! Workflow for a single attestation:
//!
//! 1. Compute `record_hash = SHA-256(canonical_bytes(AttestationRecord))` and
//!    the deterministic `attestation_id` from the content hash.
//! 2. Submit `attest(attestationId, recordHash)` to the `AttestationRegistry`
//!    contract, signing locally with the configured private key.
//! 3. **Re-verify**: read the anchor back from the chain and compare the
//!    stored hash with the locally recomputed hash. Equal → `Verified`;
//!    different → `Failed` (integrity mismatch, the anchor must not be
//!    trusted).
//!
//! All EVM types live in this crate; `immutara-core` stays Alloy-free.

use std::time::Duration;

use alloy::network::TransactionBuilder;
use alloy::primitives::{Address, B256};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::rpc::types::TransactionRequest;
use alloy::signers::local::PrivateKeySigner;
use alloy::sol_types::SolCall;
use async_trait::async_trait;
use immutara_core::ImmutaraError;
use immutara_core::config::EvmChainConfig;
use immutara_core::domain::attestation::{
    AttestationReceipt, AttestationRecord, BlockchainVerification,
};
use immutara_core::providers::AttestationProvider;

use crate::hashing::{derive_attestation_id, record_hash};

// `sol!` binding mirroring `contracts/src/AttestationRegistry.sol`. Only the
// functions the provider uses are declared; the deployed contract is the
// Solidity source, this is just the ABI surface.
alloy::sol! {
    /// Attestation registry interface (mirrors contracts/src/AttestationRegistry.sol).
    #[sol(rpc)]
    contract AttestationRegistry {
        function attest(bytes32 attestationId, bytes32 recordHash) external;
        function getAttestation(bytes32 attestationId) external view returns (bytes32 recordHash, uint64 timestamp, address submitter);
    }
}

/// Stable provider id reported in records, events and the TUI.
pub const EVM_PROVIDER_ID: &str = "evm";

/// A real EVM attestation provider (Anvil in development).
pub struct EvmAttestationProvider {
    config: EvmChainConfig,
    wallet: PrivateKeySigner,
    contract_address: Address,
    chain_id_label: String,
}

impl EvmAttestationProvider {
    /// Construct the provider from configuration.
    ///
    /// Validates the config and resolves/parses the private key from the
    /// environment variable named by [`EvmChainConfig::private_key_env`].
    /// No network connection is made here.
    pub fn new(config: EvmChainConfig) -> Result<Self, ImmutaraError> {
        config.validate()?;
        let contract_address = parse_address(&config.contract_address)?;
        let private_key = std::env::var(&config.private_key_env).map_err(|_| {
            ImmutaraError::Config(format!(
                "EVM private key not found: set the {} environment variable \
                 (for local Anvil dev, use the standard anvil dev key)",
                config.private_key_env
            ))
        })?;
        let wallet: PrivateKeySigner = private_key.parse().map_err(|e| {
            ImmutaraError::Config(format!(
                "invalid private key in {}: {e}",
                config.private_key_env
            ))
        })?;
        let chain_id_label = config.chain_id.to_string();
        Ok(Self {
            config,
            wallet,
            contract_address,
            chain_id_label,
        })
    }

    /// Construct the provider from a pre-parsed signer (used by tests).
    pub fn from_signer(config: EvmChainConfig, wallet: PrivateKeySigner) -> Self {
        let chain_id_label = config.chain_id.to_string();
        let contract_address =
            parse_address(&config.contract_address).expect("validated address must parse");
        Self {
            config,
            wallet,
            contract_address,
            chain_id_label,
        }
    }

    /// Read an anchor's stored hash back from the chain.
    ///
    /// Independent of submission, this is what makes re-verification
    /// possible. A slot that was never set reads back as `0x00…0`.
    async fn read_back_anchor<P: Provider>(
        &self,
        provider: &P,
        attestation_id: B256,
    ) -> Result<B256, ImmutaraError> {
        let call = AttestationRegistry::getAttestationCall {
            attestationId: attestation_id,
        };
        let request = TransactionRequest::default()
            .with_to(self.contract_address)
            .with_input(call.abi_encode());
        let data = provider.call(request).await.map_err(evm_error)?;
        let decoded =
            AttestationRegistry::getAttestationCall::abi_decode_returns(&data).map_err(|e| {
                ImmutaraError::Provider {
                    provider: EVM_PROVIDER_ID.to_string(),
                    message: format!("could not decode getAttestation return: {e}"),
                }
            })?;
        Ok(decoded.recordHash)
    }

    /// Re-verify an existing anchor **without writing anything**.
    ///
    /// Reads the chain slot for this record's deterministic attestation id and
    /// compares the stored hash with the locally recomputed record hash.
    /// Useful for independent re-verification of previously attested evidence.
    pub async fn verify_anchor(
        &self,
        record: &AttestationRecord,
    ) -> Result<BlockchainVerification, ImmutaraError> {
        let provider = self.connect().await?;
        let attestation_id = derive_attestation_id(&record.content_hash);
        let on_chain = self.read_back_anchor(&provider, attestation_id).await?;
        Ok(verify_read_back(record_hash(record)?, on_chain))
    }

    async fn connect(&self) -> Result<impl Provider, ImmutaraError> {
        ProviderBuilder::new()
            .wallet(self.wallet.clone())
            .connect(&self.config.rpc_url)
            .await
            .map_err(|e| ImmutaraError::Provider {
                provider: EVM_PROVIDER_ID.to_string(),
                message: format!("connect to {}: {e}", self.config.rpc_url),
            })
    }
}

#[async_trait]
impl AttestationProvider for EvmAttestationProvider {
    async fn attest(
        &self,
        record: &AttestationRecord,
    ) -> Result<AttestationReceipt, ImmutaraError> {
        let provider = self.connect().await?;

        let chain_id = provider.get_chain_id().await.map_err(evm_error)?;
        if chain_id != self.config.chain_id {
            return Err(ImmutaraError::Config(format!(
                "attestation.evm.chain_id mismatch: config={} but node reports {chain_id} \
                 (got {})",
                self.config.chain_id, self.config.rpc_url
            )));
        }

        let record_hash = record_hash(record)?;
        let attestation_id = derive_attestation_id(&record.content_hash);

        // Encode and submit the attestation transaction (signed locally).
        let calldata = AttestationRegistry::attestCall {
            attestationId: attestation_id,
            recordHash: record_hash,
        }
        .abi_encode();
        let pending = provider
            .send_transaction(
                TransactionRequest::default()
                    .with_from(self.wallet.address())
                    .with_to(self.contract_address)
                    .with_input(calldata),
            )
            .await
            .map_err(evm_error)?;
        let tx_hash = *pending.tx_hash();
        let receipt = pending
            .with_timeout(Some(Duration::from_secs(self.config.timeout_seconds)))
            .with_required_confirmations(self.config.confirmations)
            .get_receipt()
            .await
            .map_err(evm_error)?;
        let block_number = receipt
            .block_number
            .ok_or_else(|| ImmutaraError::Provider {
                provider: EVM_PROVIDER_ID.to_string(),
                message: "attestation transaction has no block number (not yet mined)".to_string(),
            })?;
        if !receipt.status() {
            return Err(ImmutaraError::Provider {
                provider: EVM_PROVIDER_ID.to_string(),
                message: format!("attestation transaction {tx_hash} reverted"),
            });
        }

        // Re-verify: compare the stored hash with the locally recomputed one.
        let on_chain_hash = self.read_back_anchor(&provider, attestation_id).await?;
        let blockchain_verification = verify_read_back(record_hash, on_chain_hash);

        Ok(AttestationReceipt {
            tx_hash: format!("{tx_hash}"),
            block_number,
            chain_id: chain_id.to_string(),
            contract_address: format!("{:#x}", self.contract_address),
            attestation_id: format!("0x{attestation_id:x}"),
            on_chain_record_hash: format!("0x{on_chain_hash:x}"),
            blockchain_verification,
        })
    }

    fn provider_id(&self) -> &str {
        EVM_PROVIDER_ID
    }

    fn chain_id(&self) -> &str {
        &self.chain_id_label
    }
}

/// Compare a locally computed record hash with the on-chain value.
pub fn verify_read_back(local_hash: B256, on_chain_hash: B256) -> BlockchainVerification {
    if local_hash == on_chain_hash {
        BlockchainVerification::Verified
    } else {
        BlockchainVerification::Failed
    }
}

fn parse_address(input: &str) -> Result<Address, ImmutaraError> {
    input
        .trim()
        .parse()
        .map_err(|_| ImmutaraError::Config(format!("invalid contract address `{input}`")))
}

fn evm_error<E: std::fmt::Display>(e: E) -> ImmutaraError {
    ImmutaraError::Provider {
        provider: EVM_PROVIDER_ID.to_string(),
        message: e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use immutara_core::config::EvmChainConfig;
    use immutara_core::domain::attestation::AttestationRecord;
    use immutara_core::domain::evidence::{ContentHash, EvidenceId, SchemaVersion};
    use sha2::{Digest, Sha256};

    fn config() -> EvmChainConfig {
        EvmChainConfig {
            rpc_url: "http://127.0.0.1:8545".to_string(),
            chain_id: 31337,
            contract_address: "0x5FbDB2315678afecb367f032d93F642f64180aa3".to_string(),
            private_key_env: "IMMUTARA_TEST_EVM_PRIVATE_KEY".to_string(),
            timeout_seconds: 30,
            confirmations: 1,
        }
    }

    fn record() -> AttestationRecord {
        AttestationRecord {
            schema_version: SchemaVersion(1),
            pipeline_version: "0.1.0".to_string(),
            evidence_id: EvidenceId::new(),
            content_hash: ContentHash("a".repeat(64)),
            metadata_hash: ContentHash("b".repeat(64)),
            verification_result_hash: ContentHash("c".repeat(64)),
            verification_policy_version: SchemaVersion(1),
            provider_id: "test".to_string(),
            chain_id: "31337".to_string(),
            attested_at: Utc::now(),
        }
    }

    #[test]
    fn configuration_validation_accepts_valid() {
        assert!(config().validate().is_ok());
    }

    #[test]
    fn configuration_validation_rejects_bad_fields() {
        let bad = EvmChainConfig {
            rpc_url: "ftp://nope".into(),
            ..config()
        };
        assert!(bad.validate().is_err());

        let bad = EvmChainConfig {
            chain_id: 0,
            ..config()
        };
        assert!(bad.validate().is_err());

        let bad = EvmChainConfig {
            contract_address: "0x123".into(),
            ..config()
        };
        assert!(bad.validate().is_err());

        let bad = EvmChainConfig {
            private_key_env: String::new(),
            ..config()
        };
        assert!(bad.validate().is_err());

        let bad = EvmChainConfig {
            timeout_seconds: 0,
            ..config()
        };
        assert!(bad.validate().is_err());

        let bad = EvmChainConfig {
            confirmations: 0,
            ..config()
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn new_fails_without_private_key_env() {
        // Dedicated variable name that no other test sets, so the result is
        // deterministic regardless of parallel test execution.
        let cfg = EvmChainConfig {
            private_key_env: "IMMUTARA_EVM_PRIVATE_KEY_NEVER_SET".to_string(),
            ..config()
        };
        assert!(EvmAttestationProvider::new(cfg).is_err());
    }

    #[test]
    fn new_parses_key_and_reports_ids() {
        // Standard anvil dev key (documented; tests only). Dedicated var name
        // keeps env-dependent tests independent under parallel execution.
        unsafe {
            std::env::set_var(
                "IMMUTARA_TEST_EVM_KEY_VALID",
                "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
            );
        }
        let cfg = EvmChainConfig {
            private_key_env: "IMMUTARA_TEST_EVM_KEY_VALID".to_string(),
            ..config()
        };
        let provider = EvmAttestationProvider::new(cfg).expect("config valid");
        assert_eq!(provider.provider_id(), EVM_PROVIDER_ID);
        assert_eq!(provider.chain_id(), "31337");
    }

    #[test]
    fn new_rejects_invalid_private_key() {
        unsafe {
            std::env::set_var("IMMUTARA_TEST_EVM_KEY_BAD", "not-a-key");
        }
        let cfg = EvmChainConfig {
            private_key_env: "IMMUTARA_TEST_EVM_KEY_BAD".to_string(),
            ..config()
        };
        assert!(EvmAttestationProvider::new(cfg).is_err());
    }

    #[test]
    fn derive_attestation_id_is_deterministic_and_content_based() {
        let a = ContentHash("a".repeat(64));
        let b = ContentHash("b".repeat(64));
        assert_eq!(derive_attestation_id(&a), derive_attestation_id(&a));
        assert_ne!(derive_attestation_id(&a), derive_attestation_id(&b));
        assert_eq!(derive_attestation_id(&a).len(), 32);
    }

    #[test]
    fn record_hash_is_deterministic_and_sensitive_to_tampering() {
        let r1 = record();
        let mut r2 = record();
        r2.pipeline_version = "0.9.9".to_string();

        let h1 = record_hash(&r1).unwrap();
        let h2_again = record_hash(&r1).unwrap();
        let tampered = record_hash(&r2).unwrap();
        assert_eq!(h1, h2_again);
        assert_ne!(h1, tampered);
    }

    #[test]
    fn verify_read_back_detects_mismatch() {
        let h = B256::from(sha256_bytes(&[1, 2, 3]));
        let other = B256::from(sha256_bytes(&[1, 2, 4]));
        assert_eq!(verify_read_back(h, h), BlockchainVerification::Verified);
        assert_eq!(verify_read_back(h, other), BlockchainVerification::Failed);
        // An never-set slot reads back as zero bytes -> mismatch.
        assert_eq!(
            verify_read_back(h, B256::ZERO),
            BlockchainVerification::Failed
        );
    }

    fn sha256_bytes(data: &[u8]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(data);
        hasher.finalize().into()
    }

    #[test]
    fn calldata_roundtrips_through_sol_bindings() {
        use alloy::primitives::keccak256;
        let id = B256::from([0x11u8; 32]);
        let hash = B256::from([0x22u8; 32]);
        let calldata = AttestationRegistry::attestCall {
            attestationId: id,
            recordHash: hash,
        }
        .abi_encode();
        // selector(4) + 2 × word(32).
        assert_eq!(calldata.len(), 68);
        // Selector must match keccak("attest(bytes32,bytes32)")[:4].
        let expected = keccak256(b"attest(bytes32,bytes32)");
        assert_eq!(calldata[..4], expected[..4]);
        let decoded = AttestationRegistry::attestCall::abi_decode(&calldata).expect("decode args");
        assert_eq!(decoded.attestationId, id);
        assert_eq!(decoded.recordHash, hash);
    }

    #[test]
    fn get_attestation_returns_decode_roundtrips() {
        let id = B256::from([0x33u8; 32]);
        let calldata = AttestationRegistry::getAttestationCall { attestationId: id }.abi_encode();
        assert_eq!(calldata.len(), 36);
        let decoded =
            AttestationRegistry::getAttestationCall::abi_decode(&calldata).expect("decode args");
        assert_eq!(decoded.attestationId, id);

        // ABI-encode the `(bytes32,uint64,address)` return tuple by hand and
        // make sure our `abi_decode_returns` usage is correct.
        let record_hash = B256::from([0x44u8; 32]);
        let timestamp = 1_700_000_000u64;
        let submitter = Address::ZERO;
        let mut encoded = vec![0u8; 96];
        encoded[..32].copy_from_slice(record_hash.as_slice());
        encoded[56..64].copy_from_slice(&timestamp.to_be_bytes());
        encoded[64..84].copy_from_slice(submitter.as_slice());

        let back = AttestationRegistry::getAttestationCall::abi_decode_returns(&encoded)
            .expect("decode returns");
        assert_eq!(back.recordHash, record_hash);
        assert_eq!(back.timestamp, timestamp);
        assert_eq!(back.submitter, submitter);
    }
}
