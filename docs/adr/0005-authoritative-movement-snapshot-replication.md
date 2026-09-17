# ADR 0005 — Authoritative Movement Snapshot Replication

## Status

Accepted for AF2-004D.

## Context

AF2-004A established server-owned character position, AF2-004B added movement-input freshness/deadman safety, and AF2-004C inserted server-authoritative traversal resolution before position commit.

The next boundary is returning that server-owned reality to the controlling client without allowing simulation code to depend on QUIC or network connection objects.

## Decision

The movement replication path is:

`ZoneRuntime -> MovementSnapshot -> bounded snapshot queue -> SessionRuntime/SelfSnapshotBridge -> connection-scoped realtime capability -> QUIC DATAGRAM -> SelfMovementSnapshot`

The `ZoneRuntime` remains the source of truth for the position and last processed input sequence.

The transport remains an adapter. Simulation does not know about `quinn::Connection`, TLS, ALPN or datagram framing.

## Internal movement snapshot

The simulation emits an internal `MovementSnapshot` containing:

- server-owned `CharacterId`;
- authoritative `ServerTick`;
- authoritative `WorldPositionMm`;
- last movement `IntentSequence` processed by the zone, if one exists.

`CharacterId` is allowed here because this event never crosses the client trust boundary. It is used by the application replication bridge to resolve the controlling live connection.

The zone records the last processed sequence only after `CharacterState::apply_movement_input` accepts the command. The replication acknowledgement therefore represents zone-processed input rather than merely received or routed input.

## Self-scoped wire contract

The first movement snapshot packet is intentionally scoped to the client's own controlled character:

`SelfMovementSnapshot`

It contains:

- `server_tick: u64`;
- authoritative `x_mm: i64`;
- authoritative `y_mm: i64`;
- authoritative `z_mm: i64`;
- optional `last_processed_input_sequence: u64`.

It does **not** contain `SessionId`, `AccountId`, `CharacterId` or `ZoneId`.

The authenticated/admitted connection provides the scope: a self movement snapshot received on a connection describes the character that the server has bound to that connection.

This avoids prematurely placing persistent gameplay identifiers in the realtime hot packet.

## Future entity replication identity

AF2-004D does not define the final identity used when replicating other entities in AOI.

When other players, creatures, resources and structures are replicated, the server should introduce a compact connection/session-appropriate replication identifier such as `NetEntityId`, rather than placing persistent `CharacterId` or other durable IDs blindly into every hot packet.

That mapping belongs with AOI/entity replication and is deferred.

## Bounded snapshot production

Movement snapshots are replaceable realtime state.

The zone publishes them through a bounded `movement_snapshot_capacity` channel using `try_send`.

If the queue is full, the current snapshot is dropped instead of blocking the single-writer simulation. A newer snapshot is more useful than making the zone wait behind stale position data.

If the configured snapshot receiver disappears while the zone runtime is active, the zone treats that as a supervised runtime failure rather than silently running forever with a broken configured replication pipeline.

## Connection-scoped realtime capability

On successful QUIC admission, transport creates a `RealtimeConnectionSender` bound to exactly that `ConnectionId` and includes it in the admitted session event.

The capability has no API for selecting another connection. This reduces accidental cross-session routing compared with exposing a global `send(connection_id, ...)` transport surface.

For the Quinn backend, `RealtimeConnectionSender` uses QUIC DATAGRAM and the configured bounded Quinn datagram send buffer. Congestion/send failure for a replaceable snapshot does not block the simulation.

A bounded channel backend exists to test the capability without exposing Quinn to application tests.

## Application replication bridge

`SelfSnapshotBridge` lives in the world-server composition/application layer.

It maintains runtime-only mappings between server-owned characters and their connection-scoped realtime capabilities.

The bridge translates an internal `MovementSnapshot` into the identity-free `SelfMovementSnapshot` wire contract and asks the connection capability to send it.

On disconnect, the connection capability and its character mapping are removed. A snapshot racing with teardown after that point is dropped rather than reopening or recreating authority.

## Client observation

The development `protocol-client` now listens for `SelfMovementSnapshot` datagrams after sending its movement intent.

This proves the complete loop:

`client intent -> session authority -> zone simulation -> traversal -> authoritative position -> snapshot -> QUIC -> client observation`

The protocol client only observes the authoritative snapshot. AF2-004D does not implement client prediction or reconciliation logic yet.

## Snapshot cadence

For this foundation, the zone can emit a snapshot for each resident character on each simulation tick, including stationary ticks.

This is deliberately simple for correctness testing. It is **not** the final MMO replication budget.

Later replication milestones must introduce:

- AOI relevance;
- replication frequency/LOD budgets;
- change detection and delta snapshots;
- batching;
- stale snapshot dropping/coalescing;
- entity replication IDs;
- bandwidth and packet budgets;
- prioritization under congestion.

## Deferred

AF2-004D intentionally does not implement:

- UE5 client integration;
- client prediction;
- reconciliation/replay of local inputs;
- interpolation of remote entities;
- AOI;
- `NetEntityId` mapping;
- remote entity snapshots;
- delta compression or bit packing;
- replication LOD/budgets;
- snapshot acknowledgement from client;
- reliable delivery of realtime position state.

The invariant established here is that authoritative movement state originates in the zone and is returned to the client as server-owned reality over a replaceable realtime path.
