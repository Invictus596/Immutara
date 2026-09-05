# Smart Contracts

Immutara's on-chain attestation layer: a single minimal EVM contract that
stores a **hash anchor** for every attested evidence item.

## Design principles

- **No raw data on-chain.** Only `recordHash` (SHA-256 of the canonical
  `AttestationRecord` bytes), its deterministic `attestationId`, a timestamp,
  and the submitting address are stored. No media, biometrics, or private
  data ever touch the contract.
- **Minimal surface.** One mapping, two functions (`attest` / `getAttestation`),
  one event. The smallest possible trusted code.
- **Deterministic ids.** The `attestationId` is derived from the evidence
  content hash, so the same evidence maps to the same slot — enabling
  tamper-proof re-verification without any lookup index.

## Contract: `src/AttestationRegistry.sol`

| Field      | Description                                             |
| ---------- | ------------------------------------------------------- |
| `recordHash` | SHA-256 of the canonical `AttestationRecord` bytes.    |
| `timestamp`  | `block.timestamp` cast down to `uint64` (safe: EVM timestamps fit). |
| `submitter`  | `msg.sender` at attestation time.                      |

```solidity
function attest(bytes32 attestationId, bytes32 recordHash) external;
function getAttestation(bytes32 attestationId)
    external view returns (bytes32 recordHash, uint64 timestamp, address submitter);
```

## Tooling

- **Build / lint / test:** `cd contracts && forge build && forge lint src/ && forge test`
- **Deploy:** `IMMUTARA_EVM_PRIVATE_KEY=<key> ../scripts/deploy_contract.sh [rpc-url] [chain-id]`
  (defaults to a local Anvil node on `http://127.0.0.1:8545`, chain `31337`).
- The deployment address is **not** hardcoded anywhere — paste the printed
  address into `[attestation.evm].contract_address` in your config.

## How the hash anchor is used

1. A submitter creates an `AttestationRecord` for evidence; the pipeline
   serializes it canonically and computes `recordHash`.
2. `attest(contentHash, recordHash)` stores the anchor.
3. The `evm` provider (in `immutara-pipeline`) re-reads the slot with
   `getAttestation` and compares it against the locally recomputed hash. Only
   if they match exactly is the block verified; any mismatch (tampered record
   or corrupted storage) is reported as a verification failure.

## Keepout

- Keys stay in environment variables (`IMMUTARA_EVM_PRIVATE_KEY`) — never in
  config or source.
- `out/`, `cache/`, `broadcast/` are gitignored build artifacts.