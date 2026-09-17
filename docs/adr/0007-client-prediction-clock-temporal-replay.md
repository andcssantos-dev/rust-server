# ADR 0007 — Client Prediction Clock and Temporal Replay Plan

- Status: Accepted for AF2-005B
- Date: 2026-08-27

## Context

AF2-005A established the client reconciliation baseline:

```text
ordered authoritative snapshots
+ monotonic processed-input acknowledgement
+ bounded unacknowledged input history
```

The next requirement is client prediction. A naive implementation would replay each pending `MoveIntent` as one movement delta, but that would be incorrect because a movement intent is a held directional state that may remain active across multiple authoritative ticks.

Prediction therefore needs temporal information before it needs positional integration.

A second architectural constraint is equally important: authoritative movement tuning remains server-owned. The protocol client must not invent or hard-code authoritative movement speed, simulation tick rate, traversal radius, or deadman policy merely to make prediction appear functional.

## Decision

AF2-005B introduces a monotonic client prediction clock and a temporal replay plan for unacknowledged movement intents.

### Client prediction clock

The development protocol client starts a monotonic clock after the authoritative session handshake succeeds.

Clock values are represented as integer microseconds from that local origin:

```text
ClientTimeUs(u64)
```

The clock exists only for local prediction and presentation bookkeeping. It has no authority over server time or `ServerTick`.

Client prediction time must never regress. Arithmetic is checked and overflow/regression is surfaced as an error.

### Timed pending input history

Each locally sent movement intent is recorded only after the local QUIC datagram send path accepts it, along with the current client prediction time:

```text
TimedPendingInput
├─ MoveIntent
└─ sent_at: ClientTimeUs
```

Both input sequence and client send time are monotonic.

The existing bounded pending-history rule remains unchanged: unacknowledged inputs are never silently evicted.

### Temporal replay plan

For the current set of pending inputs, the reconciler can build an immutable replay plan at a supplied local time.

Example:

```text
sequence 2 sent at 100 ms
sequence 3 sent at 160 ms
plan generated at 230 ms

ReplayPlan
├─ sequence 2 active for 60 ms
└─ sequence 3 active for 70 ms
```

The plan describes the local temporal history of still-unacknowledged input states. It does not mutate the authoritative baseline and does not claim that local time equals server time.

When an authoritative snapshot acknowledges inputs, those inputs are pruned first. A replay plan generated after reconciliation therefore contains only still-pending input states.

### Live probe observation

For each received authoritative snapshot, the protocol probe builds a replay plan before applying the snapshot and another after reconciliation.

This makes the acknowledgement boundary observable:

```text
before snapshot ACK:
pending sequence 1 → temporal replay segment exists

snapshot ACK=1 accepted

post reconciliation:
sequence 1 pruned → replay plan empty
```

### No positional prediction yet

AF2-005B intentionally does not convert temporal replay duration into millimeters.

Doing so requires server-owned prediction parameters, at minimum the movement model/tuning needed to reproduce presentation movement. Hard-coding the current development values in the client would violate the server-authoritative configuration rule and create hidden coupling.

The intended progression is:

```text
AF2-005A
snapshot ordering + ACK

AF2-005B
client clock + temporal replay plan

AF2-005C
server-owned prediction profile + positional prediction/replay
```

A future client may use the temporal replay plan together with a server-provided movement profile and local collision approximation to produce responsive presentation movement. Rust remains definitive.

## Authority boundary

The client prediction clock:

- does not advance `ServerTick`;
- does not alter authoritative position;
- does not decide movement speed;
- does not decide traversal/collision;
- does not change server deadman policy;
- exists only to reconstruct local presentation timing.

The newest accepted `SelfMovementSnapshot` remains the authoritative correction baseline.

## Security and ordering note

Server anti-replay and movement semantics remain distinct.

The live session registry already rejects `ReplayDecision::AcceptedOutOfOrder` movement sequences from becoming authorized intents. The Zone/CharacterState also rejects non-monotonic movement sequences as a defense-in-depth invariant.

Client prediction therefore must preserve strictly increasing local input sequence numbers and must not attempt to reinterpret stale input as newer state.

## Scope

Included in AF2-005B:

- monotonic microsecond client prediction clock;
- timestamped bounded pending input history;
- monotonic client input send-time validation;
- deterministic temporal replay-plan construction;
- checked replay duration arithmetic;
- replay-plan pruning through authoritative ACK reconciliation;
- live pre/post reconciliation replay observability;
- tests for replay timing and clock regressions.

Deferred:

- server-provided movement prediction profile;
- positional client prediction;
- server/client tick-phase synchronization;
- RTT/clock-offset estimation;
- replay of movement through local traversal approximation;
- correction smoothing;
- UE5 implementation;
- remote-entity interpolation;
- packet-loss/jitter/reordering harness.
