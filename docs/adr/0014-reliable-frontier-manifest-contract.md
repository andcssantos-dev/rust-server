# ADR 0014 — Reliable Frontier Manifest Contract

## Status

Accepted for AF2-007B.

## Context

AF2-007A gives the authoritative server a concrete initial discovery frontier while leaving all other quadrants as implicit mathematical potential.

The client must now learn which quadrants are revealed without becoming an authority over discovery state. The bootstrap already sends a server-owned movement prediction profile over authenticated reliable QUIC control delivery after `ServerHello` acceptance.

The frontier belongs to the world domain, but its serialized representation belongs to `aurenfall-contracts`. The transport must remain ignorant of frontier semantics.

## Decision

### 1. Introduce `FrontierManifest` as a versioned wire contract

The initial wire contract contains:

```text
manifest_version: u32
generator_version: u32
quadrant_size_mm: i64
revision: u64
quadrant_count: u16
quadrants: repeated (x: i64, y: i64)
```

All integer fields use explicit little-endian encoding.

The current version is `FRONTIER_MANIFEST_VERSION = 1`.

### 2. Message kind 200 is renamed from the unused Sector placeholder

`MessageKind` value `200` is now:

```text
FrontierManifest = 200
```

The previous `SectorManifest` name was a placeholder and had no implemented payload contract. Reusing the reserved world-manifest value avoids carrying obsolete Sector terminology into the discovery protocol.

### 3. The manifest contains exact revealed coordinates

Although AF2-007A currently creates a rectangular 4×4 frontier, the network contract sends the exact ordered list of revealed `QuadrantCoord` values rather than only rectangle bounds.

This is deliberate. Future discovery frontiers may be irregular due to coastlines, gates, progression, world rules, or administrative state. The client must render exactly what the server declares rather than reconstructing policy from shape metadata.

The initial 4×4 manifest therefore carries 16 coordinates in deterministic row-major order:

```text
(-1,-1) (0,-1) (1,-1) (2,-1)
(-1, 0) (0, 0) (1, 0) (2, 0)
(-1, 1) (0, 1) (1, 1) (2, 1)
(-1, 2) (0, 2) (1, 2) (2, 2)
```

### 4. Geometry accompanies discovery state

`quadrant_size_mm` and `generator_version` are included in the manifest.

The Unreal client therefore does not need to assume the prototype 512 m size or independently choose a generator compatibility version. The server-provided values can configure the client world subsystem before materialization.

### 5. Manifest revisions start at one

The initial frontier uses:

```text
revision = 1
```

A revision of zero is invalid.

AF2-007B does not yet implement live frontier expansion, but including the revision now establishes the ordering primitive required for future reveal updates. A client can later reject stale or duplicate manifest revisions without using wall-clock time.

### 6. Duplicate coordinates are invalid

The contract rejects manifests containing duplicate quadrant coordinates.

This prevents a malformed authoritative payload from producing duplicate materialization work or ambiguous client ownership of a visual quadrant.

### 7. Reliable bootstrap supports multiple independent frames

`QuicServer` now stores multiple immutable `InitialReliableFrame` values.

After an authenticated and accepted `ServerHello`, each frame is sent over a reliable QUIC unidirectional control stream. The transport validates each payload against `max_control_frame_bytes` before server startup completes.

The first two server-owned bootstrap contracts are:

```text
MovementPredictionProfile
FrontierManifest
```

They are semantically independent. Clients must classify them by `MessageKind` rather than assuming arrival order.

### 8. Bootstrap payloads are shared across sessions

The configured initial reliable frames are stored behind shared `Arc` ownership. Per-connection admission clones only shared references rather than cloning frontier payload bytes for every connection.

### 9. Potential world space is still not transmitted

The server sends revealed quadrants only.

There is no list of unrevealed quadrants and no `PotentialQuadrant` payload. Absence from the manifest continues to mean that the coordinate is mathematically addressable but not revealed/materialized.

### 10. A dedicated Rust probe validates the boundary before UE integration

`apps/protocol-client/examples/frontier_probe.rs` performs a real TLS/QUIC handshake and requires both initial reliable contracts.

It decodes the `FrontierManifest` and logs:

- universe ID;
- manifest version and revision;
- generator version;
- server-owned quadrant size;
- revealed quadrant count;
- first and last coordinates.

This provides a transport-level E2E gate before the Unreal client consumes the contract.

## Validation required

AF2-007B is not complete until all of the following pass locally:

1. workspace format/check/Clippy/tests/GameData preflight;
2. `FrontierManifest` round-trip tests;
3. invalid geometry, duplicate coordinate, truncated payload, and trailing payload rejection tests;
4. world-server boot with a manifest payload below the configured control-frame maximum;
5. real `frontier_probe` connection receiving both reliable bootstrap contracts;
6. initial manifest reports 16 quadrants from `(-1,-1)` through `(2,2)` with `quadrant_size_mm = 512000` and `revision = 1`;
7. clean working tree with deliberate `Cargo.lock` review.

## Consequences

### Positive

- The server is the only authority deciding which quadrants are revealed.
- The client receives exact materialization coordinates rather than deriving discovery policy.
- World geometry is transmitted alongside discovery state.
- The contract supports irregular future frontiers.
- Revision ordering exists before live frontier expansion is introduced.
- Multiple bootstrap states can be added without teaching the transport their semantics.
- Potential world space still has zero network representation cost.

### Trade-offs

- Explicit coordinate lists are larger than rectangle-only descriptors.
- Large future manifests must remain within the configured reliable control-frame budget or move to a chunked/snapshot protocol.
- AF2-007B publishes the initial snapshot only; it does not yet implement runtime reveal deltas.

## Follow-up

After AF2-007B is validated:

1. implement the corresponding Frontier Manifest decoder in the Unreal C++ client;
2. configure `UAurenfallWorldSubsystem` from server-owned geometry;
3. materialize exactly the received Quadrant coordinates;
4. remove the debug actor as the source of truth for the 4×4 layout;
5. later introduce revision-ordered frontier reveal updates and lifecycle transitions.
