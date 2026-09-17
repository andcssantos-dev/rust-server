# ADR 0006 — Client Snapshot Ordering and Reconciliation Baseline

- Status: Accepted for AF2-005A
- Date: 2026-08-27

## Context

AF2-004 established a complete authoritative movement loop:

```text
client MoveIntent
→ authoritative session
→ zone single-writer
→ deterministic movement
→ traversal
→ authoritative position
→ SelfMovementSnapshot
→ client
```

QUIC DATAGRAM does not provide ordered or reliable delivery semantics for realtime gameplay datagrams. A client therefore cannot assume movement snapshots arrive exactly once or in server-tick order.

The server also includes `last_processed_input_sequence` in `SelfMovementSnapshot`. This value is the authoritative acknowledgement boundary for client input reconciliation.

Before client-side prediction is introduced, the client needs a small deterministic kernel that can safely decide which snapshots advance its authoritative baseline and which locally sent inputs remain unacknowledged.

## Decision

AF2-005A introduces a client-side reconciliation kernel in the protocol probe.

### Snapshot ordering

`server_tick` is the ordering key for one live session.

- A snapshot with a strictly newer server tick may advance the authoritative client baseline.
- A snapshot with an older server tick is stale and is ignored.
- A byte-equivalent logical snapshot for the same server tick is treated as a harmless duplicate and ignored.
- A snapshot for the same server tick with different authoritative contents is a protocol consistency error.

Server tick wrap is not supported. The authoritative server already treats tick-space exhaustion as fatal rather than wrapping.

### Input acknowledgement

The client keeps a bounded history of movement intents that were accepted by the local transport send path.

`last_processed_input_sequence` is interpreted as the newest input sequence the server has processed for this client.

For an accepted newer snapshot:

- an acknowledgement may remain unchanged;
- an acknowledgement may advance to a sequence that exists in the client's pending sent-input history;
- all pending inputs at or below the newly acknowledged sequence are pruned;
- an acknowledgement must never regress;
- once an acknowledgement exists, a newer snapshot must not return to `None`;
- the server must not acknowledge a sequence the client does not know as sent and pending, except for repetition of the already accepted acknowledgement boundary.

These checks make authority mistakes and protocol corruption visible instead of silently rewriting local reconciliation state.

### Bounded history

Pending input history is bounded.

AF2-005A does not silently evict unacknowledged inputs when the history is full. Exhaustion is a fail-closed reconciliation error. A future production client may respond by pausing new predicted input, requesting a resynchronization, or reconnecting according to policy.

The development protocol probe uses a capacity of 256 pending movement intents.

### No fake movement replay in AF2-005A

AF2-005A intentionally does not replay pending `MoveIntent` records as one displacement each.

A `MoveIntent` is a held directional state. The authoritative server may apply one accepted input over multiple fixed simulation ticks until a newer intent or the movement deadman replaces it. Therefore:

```text
one MoveIntent != one movement delta
```

Treating each pending intent as one displacement would create a client prediction model with incorrect temporal semantics.

AF2-005A instead establishes:

```text
ordered authoritative snapshot
+ authoritative correction baseline
+ monotonic processed-input ACK
+ bounded unacknowledged input history
```

A later AF2-005 slice will introduce the client prediction clock/timeline required to replay unacknowledged input over time.

## Authority boundary

Client reconciliation never changes server authority.

The client may later predict a temporary presentation position, but:

- the authoritative baseline always comes from the newest accepted server snapshot;
- stale or duplicate realtime delivery cannot rewind that baseline;
- local input history exists only to predict/reconcile presentation;
- Rust server movement and traversal remain definitive.

## Scope

Included in AF2-005A:

- bounded sent-input history;
- strict monotonic client send sequencing;
- server-tick snapshot ordering;
- stale snapshot rejection;
- duplicate snapshot handling;
- conflicting duplicate detection;
- monotonic processed-input acknowledgement validation;
- pruning of acknowledged input history;
- live protocol-probe reconciliation logging;
- unit tests for ordering, acknowledgement and bounded-history invariants.

Deferred:

- client prediction simulation clock;
- replay of unacknowledged input over elapsed prediction time;
- render interpolation;
- correction smoothing;
- UE5 implementation;
- AOI entity snapshot ordering;
- `NetEntityId` replication;
- packet-loss/jitter simulation harness;
- resynchronization policy after reconciliation-history exhaustion.
