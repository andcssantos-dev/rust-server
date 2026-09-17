# Persistence and Cache

## Source of truth

PostgreSQL is the durable source of truth for persistent player/world/economic state. Live zone memory is the source of truth for currently owned live simulation state until a durable boundary is required.

## Persistence classes

### Ephemeral

Input direction, transient perception targets and replaceable replication data. Never written per tick to PostgreSQL.

### Recoverable

Position/checkpoints, selected world state and timers. Persist through bounded asynchronous checkpoints/events according to recovery requirements.

### Economically critical

Item ownership, currency, trades, auction/mail/guild-vault transfers, crafting outputs and destructive mutations. Use transactional durable operations with idempotency keys and optimistic aggregate versions.

## Outbox

A transaction that creates a durable external event also writes an outbox record in the same commit. Publication occurs after commit and may be retried safely.

## Tick isolation

No PostgreSQL query is awaited inside a zone mutation/tick path. Zone submits a durable request and later receives completion/failure as a command.

## Cache taxonomy

### GameData snapshot cache

Immutable, content-hash/version keyed. Multiple versions may coexist while existing encounters/sectors are bound to an older snapshot.

### Aggregate cache

Bounded hot character/kingdom/guild aggregates with explicit version and eviction policy. Eviction never loses authoritative dirty state.

### Read-model cache

Discardable cached views such as market searches or public summaries. Safe to rebuild.

### Generation memoization

Only for pure deterministic generator results. Key includes universe/generator version/coordinate and relevant rule hash.

## Cache rules

- never use cache presence as ownership proof;
- every cache has a memory budget and metrics;
- no unbounded maps in long-lived services;
- distributed cache/Valkey is introduced only for a concrete cross-process requirement.
