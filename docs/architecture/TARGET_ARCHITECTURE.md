# Target Architecture

This document describes the intended long-lived server platform. Implementation is incremental; boundaries are not provisional.

## Logical topology

```text
UE5 Client
   |
   v
Edge / Session Boundary
   |  QUIC, TLS, handshake, admission, rate limits, replay protection
   v
World Routing
   |
   +------> Zone Runtime A ----+
   +------> Zone Runtime B ----+--> durable-operation queue --> Persistence --> PostgreSQL
   +------> Zone Runtime N ----+
               |
               +--> Domain
               +--> Spatial/AOI
               +--> Replication
               +--> GameData snapshot
               +--> local caches/indexes
```

The first deployment may host these logical components in one process. A process split is allowed only when the logical contract already exists and measurements justify the operational cost.

## Modules

- `core`: strongly typed identifiers, units, deterministic world coordinates/time.
- `contracts`: cross-language/versioned network and durable event contracts.
- `config`: process/deployment configuration; never gameplay balance.
- `gamedata`: authoring loader, schema/semantic validation, ID resolution and immutable snapshots.
- `domain`: reusable authoritative foundations such as ItemInstance, Container, Ownership, ResourceInstance, Character and Interaction.
- `simulation`: fixed tick, zone ownership, commands/events, spatial indexes, AI/combat scheduling and replication preparation.
- `transport`: QUIC, sessions, admission, protocol budgets and network serialization.
- `persistence`: PostgreSQL adapters, transactional durable operations, optimistic versions and outbox.
- `cache`: specialized bounded caches. Cache is never source of truth.
- `observability`: logs, traces, metrics and profiling integration.

## Dependency law

Infrastructure depends on domain/core contracts. Domain code does not depend on Quinn, PostgreSQL, Unreal or filesystem APIs.

## Future process extraction

Likely candidates, only when justified:

- gateway/session edge;
- account/authentication;
- world coordinator/placement;
- zone workers;
- chat/social;
- market/search read models;
- persistence/outbox workers;
- administration/telemetry.

These are not day-one microservices.
