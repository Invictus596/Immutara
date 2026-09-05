#!/usr/bin/env bash
# Deploy the AttestationRegistry contract and print the resulting address.
#
# Usage:
#   IMMUTARA_EVM_PRIVATE_KEY=<privkey> ./scripts/deploy_contract.sh [rpc-url] [chain-id]
#
# Defaults target a local Anvil dev node. The private key is read strictly
# from the environment — never pass it on the command line and never commit it.
#
# Example (local dev, standard Anvil dev account 0):
#   IMMUTARA_EVM_PRIVATE_KEY=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 \
#     ./scripts/deploy_contract.sh
set -euo pipefail

RPC_URL="${1:-http://127.0.0.1:8545}"
CHAIN_ID="${2:-31337}"

if [[ -z "${IMMUTARA_EVM_PRIVATE_KEY:-}" ]]; then
    echo "error: IMMUTARA_EVM_PRIVATE_KEY is not set (env only, never commit keys)" >&2
    exit 1
fi

cd "$(dirname "$0")/.." # repo root

if ! command -v forge >/dev/null 2>&1; then
    echo "error: 'forge' not found — install Foundry first (https://book.getfoundry.sh)" >&2
    exit 1
fi

echo "> Building contracts..."
forge build --root contracts

echo "> Deploying AttestationRegistry to ${RPC_URL} (chain ${CHAIN_ID})..."
OUTPUT=$(forge create contracts/src/AttestationRegistry.sol:AttestationRegistry \
    --rpc-url "${RPC_URL}" \
    --chain-id "${CHAIN_ID}" \
    --private-key "${IMMUTARA_EVM_PRIVATE_KEY}" \
    --broadcast)

echo "${OUTPUT}"

# Extract the deployed address so callers can pipe it straight into config.
# --broadcast prints "Deployed to: 0x…"; fall back to the JSON "address" field.
ADDRESS=$(grep -oP 'Deployed to: \K0x[0-9a-fA-F]{40}' <<< "${OUTPUT}" || true)
if [[ -z "${ADDRESS}" ]]; then
    ADDRESS=$(grep -oP '"address":\s*"\K0x[0-9a-fA-F]{40}' <<< "${OUTPUT}" | head -1 || true)
fi
if [[ -n "${ADDRESS}" ]]; then
    echo
    echo "Contract address: ${ADDRESS}"
    echo "Set [attestation.evm] contract_address = \"${ADDRESS}\" in config."
else
    echo "warning: could not parse the deployed address from forge output" >&2
fi