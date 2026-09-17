# Aurenfall Server 2 — Architecture Overview

## System shape

Aurenfall starts as a **modular monolith** with explicit boundaries that can later become process boundaries without rewriting the domain.

```text
UE5 Client
   |
   | QUIC + versioned binary contracts
   v
Transport / Session
   |
   | bounded intents
   v
Zone Runtime (single logical writer)
   |
   +--> Domain foundations
   |      ItemInstance / Container / Ownership / Resource / Character / Interaction
   |
   +--> Spatial index / AOI / replication
   |
   +--> durable-operation requests ----> Persistence workers ----> PostgreSQL
   |
   +--> immutable GameData snapshots
```

## World model

A universe may have an enormous logical coordinate space without materializing it. A sector has three conceptual states:

1. **Potential** — determinable from universe seed + coordinate + generator version; not stored.
2. **Materialized** — generated because a frontier required it.
3. **Mutated** — persistent deltas exist because authoritative gameplay changed it.

The generator must be deterministic and versioned. Existing materialized state never silently changes because a newer generator ships.

## Performance model

- zone state has one logical writer;
- work enters through bounded command queues;
- database/network I/O does not occur inside simulation mutation code;
- AOI prevents global broadcast;
- world spatial indexes serve collision, interaction, AI perception and replication;
- inactive sectors use coarse/dormant simulation rather than active ticks;
- hot paths use typed numeric IDs and compact data structures rather than string lookup.

## Persistence model

PostgreSQL is the durable source of truth for persistent/economic state. Critical ownership/currency mutations use idempotent operation IDs and transactional commits. An outbox records externally publishable durable events.

## Client boundary

The UE5 client owns presentation, local input capture and optional prediction. It never owns final movement, combat, RNG, item ownership, economy, world generation decisions or persistent state.
