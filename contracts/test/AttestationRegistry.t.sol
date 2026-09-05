// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {AttestationRegistry} from "../src/AttestationRegistry.sol";

/// @notice Self-contained Foundry tests (no forge-std dependency): Foundry
///         treats any reverting test function as a failure, so `require`
///         assertions are sufficient.
contract AttestationRegistryTest {
    function test_attest_get_readback_roundtrip() public {
        AttestationRegistry registry = new AttestationRegistry();
        bytes32 id = keccak256("evidence-1");
        bytes32 recordHash = keccak256("record-bytes-1");

        registry.attest(id, recordHash);

        (bytes32 got, uint64 timestamp, address submitter) =
            registry.getAttestation(id);
        require(got == recordHash, "stored record hash mismatch");
        require(timestamp > 0, "timestamp was not recorded");
        require(submitter == address(this), "submitter must be the caller");
    }

    function test_unset_slot_reads_back_empty() public {
        AttestationRegistry registry = new AttestationRegistry();
        (bytes32 got, uint64 timestamp, address submitter) =
            registry.getAttestation(keccak256("never-written"));
        require(got == bytes32(0), "unset slot must read as zero hash");
        require(timestamp == 0, "unset slot must read as zero timestamp");
        require(submitter == address(0), "unset slot must read as zero submitter");
    }

    function test_attest_idempotently_overwrites_same_slot() public {
        AttestationRegistry registry = new AttestationRegistry();
        bytes32 id = keccak256("evidence-2");

        registry.attest(id, keccak256("first"));
        registry.attest(id, keccak256("second"));

        (bytes32 got, uint64 timestamp, address submitter) = registry.getAttestation(id);
        require(got == keccak256("second"), "later attestation must win");
        require(submitter == address(this), "submitter unchanged");
        require(timestamp > 0, "timestamp still recorded after overwrite");
    }
}