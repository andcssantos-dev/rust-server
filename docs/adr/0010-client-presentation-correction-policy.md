# ADR 0010 — Client Presentation Correction Policy

Status: Accepted for AF2-005E

## Context

AF2-005A through AF2-005D established snapshot ordering, acknowledgement reconciliation, a monotonic client prediction clock, temporal replay, a server-owned prediction profile, positional prediction, and acknowledged held-input extrapolation.

The client can now calculate a temporary gameplay prediction and compare it with authoritative movement snapshots. A prediction mismatch must never delay or weaken server authority, but applying every mismatch directly to the rendered character would create visible snapping and rubber-banding.

Three positions therefore need distinct semantics:

1. **Authoritative position** — decided by the Rust Zone single-writer.
2. **Predicted gameplay position** — temporary client estimate based on the latest authoritative baseline plus locally known replay.
3. **Rendered position** — presentation-only position shown to the player.

## Decision

The authoritative snapshot immediately replaces the client's gameplay reconciliation baseline. Presentation smoothing is implemented only as a decaying visual offset layered on top of the current predicted gameplay position.

The client never delays authoritative reconciliation while smoothing.

### Server-owned correction tuning

Correction policy parameters are stored in `config/server.toml`, validated by `aurenfall-config`, included in the reliable `MovementPredictionProfile`, and consumed by the client.

The profile is upgraded to version 2 and includes:

- `absorb_max_mm`
- `smooth_max_mm`
- `hard_snap_threshold_mm`
- `smooth_duration_ms`
- `rapid_duration_ms`

Development defaults are:

- absorb at or below 25 mm,
- smooth at or below 250 mm over 120 ms,
- rapid below 1000 mm over 50 ms,
- hard snap at or above 1000 mm.

The client build baseline is raised to 3 so an older probe cannot complete bootstrap and then decode the incompatible prediction-profile payload.

### Correction modes

`Absorb` clears the presentation offset immediately for visually negligible mismatch.

`Smooth` preserves the old rendered position as a presentation offset and linearly decays that offset over the server-provided smooth duration.

`Rapid` uses the same offset model with the shorter server-provided rapid duration.

`HardSnap` clears presentation offset immediately for a large mismatch.

Explicit gameplay events such as teleport or respawn may force hard snap in a later protocol extension regardless of distance. AF2-005E establishes distance-based policy only.

### Continuity across repeated corrections

A new authoritative snapshot may arrive while an earlier presentation correction is still active.

Before creating a new correction, the client samples the existing presentation offset and computes:

```text
old_rendered = old_gameplay_prediction + current_presentation_offset
new_offset   = old_rendered - new_gameplay_prediction
```

This allows a new smooth or rapid correction to begin without introducing an additional visual discontinuity.

### Backpressure and authority

Presentation reconciliation exists only inside the client probe. It does not send rendered position, correction mode, correction distance, or presentation offsets back to the server.

A modified client may ignore or alter smoothing parameters, but this affects only its own display. The Rust server remains authoritative for movement, traversal, active-input state, and snapshots.

## Consequences

The future UE client can maintain responsive prediction while visually absorbing ordinary network correction without treating the rendered transform as gameplay truth.

Collision/traversal prediction remains separate. A client that predicts through an unknown authoritative blocker may accumulate a visible correction offset when the server remains at the blocker boundary. The correction policy masks presentation discontinuity but does not make the client authoritative over traversal.

The protocol-client now provides a deterministic test kernel for presentation correction modes and offset decay before this behavior is implemented in UE5.

## Deferred

AF2-005E does not implement:

- UE5 integration,
- render-frame sampling,
- spring/critically damped interpolation,
- explicit teleport/respawn correction reasons,
- client-side traversal prediction,
- remote-entity interpolation,
- AOI replication,
- latency/jitter adaptive smoothing.
