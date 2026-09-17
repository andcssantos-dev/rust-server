# ADR 0009 — Acknowledged Held-Input Extrapolation

- Status: Accepted for AF2-005D
- Date: 2026-08-27

## Context

AF2-005A established snapshot ordering and processed-input acknowledgement. AF2-005B added a monotonic client prediction clock and temporal replay for unacknowledged inputs. AF2-005C added a server-owned prediction profile and positional prediction.

One temporal gap remained. `last_processed_input_sequence` is an acknowledgement boundary, not a statement that movement stopped. The authoritative server keeps the latest non-zero movement input active over multiple simulation ticks until one of the following happens:

- a newer movement intent replaces it;
- an explicit zero input replaces it;
- the server movement deadman expires it.

Therefore pruning an acknowledged input from pending history must not automatically make client prediction stop.

## Decision

AF2-005D separates two server facts in self-movement replication:

```text
last_processed_input_sequence
= newest client input the server has processed

active_movement_sequence
= processed input that is still driving movement now
```

A snapshot may therefore contain:

```text
processed = Some(2)
active    = Some(2)
```

while movement input 2 is still active, and later:

```text
processed = Some(2)
active    = None
```

when the deadman or an explicit zero state has stopped movement.

## Snapshot contract evolution

The existing 41-byte `SelfMovementSnapshot` contract is preserved as V1.

AF2-005D introduces:

```text
SelfMovementSnapshotV2
MessageKind::SelfMovementSnapshotV2
```

V2 is 50 bytes and adds one optional active movement sequence:

```text
server_tick                       u64
x_mm                              i64
y_mm                              i64
z_mm                              i64
processed_presence                u8
last_processed_input_sequence     u64
active_presence                   u8
active_movement_sequence          u64
```

The V1 type remains available for the established reconciliation baseline and older transport-level tests. V2 converts losslessly to the V1 reconciliation baseline by discarding only the additional active-state field.

The development client build is incremented to 2 and `config/server.toml` requires build 2 so an older development probe is rejected at bootstrap rather than connecting with an incompatible realtime snapshot expectation.

## Server ownership

`CharacterState` exposes the active movement sequence only when its authoritative movement input is non-zero.

Consequently:

- non-zero accepted input -> active sequence is present;
- traversal blocking does not clear active input;
- explicit zero input -> active sequence is absent;
- deadman expiry -> active sequence is absent;
- `last_processed_input_sequence` remains the acknowledgement boundary even after movement becomes inactive.

The Zone single-writer publishes both values in `MovementSnapshot` after simulation for the tick.

## Client extrapolation

The protocol probe introduces a bounded `HeldInputExtrapolator` that remembers client-sent intents independently from the pending reconciliation queue.

For an applied V2 authoritative snapshot:

- `active_movement_sequence`, when present, must equal `last_processed_input_sequence`;
- the active sequence must refer to an input the client actually sent;
- a zero movement input may not be marked active;
- stale or duplicate snapshots do not update held-input state because the tracker is updated only after the existing reconciler accepts a snapshot.

The next prediction plan combines:

```text
acknowledged held input since previous authoritative baseline receipt
+
unacknowledged pending input timeline
```

If a newer pending input was sent after the baseline, the held input contributes only until that local send time. The pending input then supersedes it.

## Traversal correction

Held-input extrapolation is still presentation prediction. It does not duplicate server traversal authority.

A character blocked at a wall may therefore produce:

```text
server snapshot: active=Some(2), x=400
client extrapolation: x>400
next server snapshot: x=400
client correction: back to x=400
```

This is expected. Future client presentation may use local approximate collision to reduce visible corrections, but Rust traversal remains definitive.

## Authority boundary

The client never decides whether a movement input remains authoritative.

The server publishes the active sequence. A modified client may ignore it and display incorrect prediction, but cannot alter the Zone-owned position, deadman, traversal or acknowledgement state.

## Scope

Included in AF2-005D:

- V2 self movement snapshot contract;
- explicit processed-vs-active movement state;
- active sequence sourced from authoritative `CharacterState`;
- deadman transitions active movement to `None` without erasing ACK history;
- bounded client sent-input lookup for active sequence;
- held-input extrapolation between accepted snapshots;
- composition of held and pending temporal replay;
- client build gate for the new development snapshot contract;
- unit tests for active, inactive, superseded and invalid active-sequence cases.

Deferred:

- render-frame prediction loop in UE5;
- client-side approximate traversal/collision prediction;
- correction smoothing and visual error thresholds;
- network clock synchronization;
- AOI entity extrapolation;
- reconnect/session-resume semantics;
- production protocol-version compatibility policy beyond the current development build gate.
