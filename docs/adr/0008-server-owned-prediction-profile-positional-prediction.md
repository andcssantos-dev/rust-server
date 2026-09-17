# ADR 0008 — Server-Owned Prediction Profile and Pending Positional Prediction

- Status: Accepted for AF2-005C
- Date: 2026-08-27

## Context

AF2-005A established ordered authoritative snapshots and processed-input acknowledgement. AF2-005B added a monotonic client prediction clock and temporal replay plans for unacknowledged movement intents.

The client can now answer how long an unacknowledged input was locally active, but it must not invent gameplay parameters such as character speed, simulation tick rate, traversal radius or movement deadman duration.

## Decision

AF2-005C introduces a versioned `MovementPredictionProfile` produced by the server composition root from the same server-owned movement settings used to construct the authoritative `ZoneRuntime`.

The profile contains:

```text
profile_version
speed_mm_per_second
simulation_tick_hz
character_radius_mm
movement_input_timeout_ticks
```

These values are prediction hints, not authority. Changing them on a modified client can only damage that client's local presentation. The Rust simulation remains definitive.

### Reliable initial delivery

The profile is delivered after an accepted `ServerHello` using a server-initiated reliable QUIC stream.

The transport receives only an opaque initial frame consisting of `MessageKind + bytes`. It does not interpret movement settings. This preserves the dependency boundary:

```text
world-server / gameplay configuration
→ contracts encode profile
→ transport delivers opaque reliable frame
→ client decodes prediction profile
```

The protocol probe waits for this profile before enabling positional prediction or sending movement input.

### Integer positional prediction

The client predictor uses integer arithmetic and the same movement-axis normalization shape as the authoritative movement kernel. Replay durations are measured in client microseconds.

For each unacknowledged replay segment:

```text
authoritative baseline position
+ server-owned speed
+ normalized local input
+ local elapsed microtime
→ temporary predicted presentation position
```

Prediction duration for one input state is capped by the server-provided movement deadman duration derived from `movement_input_timeout_ticks / simulation_tick_hz`.

### Pending replay only in AF2-005C

AF2-005C predicts only replay segments that are still unacknowledged.

An important semantic distinction remains:

```text
ACK means "server processed this input"
ACK does NOT mean "server stopped applying this held input state"
```

The authoritative server may continue applying an acknowledged non-zero movement input until a newer input or deadman replaces it. Therefore a replay plan becoming empty after ACK is not sufficient to extrapolate held movement between later snapshots.

AF2-005C does not pretend otherwise. Correction metrics are meaningful only when an unacknowledged replay segment exists. A later slice will retain/extrapolate the active held-input timeline across acknowledgement boundaries.

## Authority boundary

- Server movement settings remain server-owned.
- The client receives a prediction profile only to reproduce temporary presentation.
- Client predicted position is never sent back as authoritative position.
- Server snapshots always replace the client authoritative baseline.
- Traversal/collision remains authoritative on the server; local prediction may diverge and be reconciled.

## Scope

Included:

- versioned movement prediction profile contract;
- reliable initial profile delivery after accepted handshake;
- client blocks prediction until profile arrives;
- integer microtime positional predictor;
- server-deadman prediction cap;
- live probe creates a second unacknowledged input window;
- comparison of pending prediction against the following authoritative snapshot.

Deferred:

- extrapolation of already-acknowledged held input state;
- client-side traversal prediction;
- render smoothing;
- UE5 implementation;
- dynamic movement-profile changes during a live session;
- profile acknowledgement/resynchronization;
- AOI entity prediction.
