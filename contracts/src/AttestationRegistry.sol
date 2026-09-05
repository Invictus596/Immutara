// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @title AttestationRegistry
/// @notice Stores cryptographic attestation anchors for Immutara.
/// @dev Only hashes and minimal metadata are written on-chain. No raw
///      media, biometrics, or private data ever touch this contract.
contract AttestationRegistry {
    struct Attestation {
        bytes32 recordHash;
        uint64 timestamp;
        address submitter;
    }

    mapping(bytes32 => Attestation) public attestations;

    event Attested(
        bytes32 indexed attestationId,
        bytes32 indexed recordHash,
        uint64 timestamp
    );

    /// @notice Record a new attestation anchor.
    /// @param attestationId Deterministic key for this evidence item.
    /// @param recordHash    SHA-256 of the canonical attestation record.
    function attest(bytes32 attestationId, bytes32 recordHash) external {
        // casting to 'uint64' is safe because block.timestamp fits well within
        // uint64's range (the EVM's existing timestamp overflow problem).
        // forge-lint: disable-next-line(unsafe-typecast)
        uint64 timestamp = uint64(block.timestamp);
        attestations[attestationId] = Attestation({
            recordHash: recordHash,
            timestamp: timestamp,
            submitter: msg.sender
        });
        // forge-lint: disable-next-line(unsafe-typecast)
        emit Attested(attestationId, recordHash, timestamp);
    }

    /// @notice Read the anchor back for a given evidence item.
    /// @return recordHash The stored record hash.
    /// @return timestamp  The block timestamp when the anchor was written.
    /// @return submitter  The address that submitted the anchor.
    function getAttestation(
        bytes32 attestationId
    )
        external
        view
        returns (bytes32 recordHash, uint64 timestamp, address submitter)
    {
        Attestation storage a = attestations[attestationId];
        return (a.recordHash, a.timestamp, a.submitter);
    }
}