# ADR-0001: Authority and State Ownership Laws

Status: Accepted

## Decision

**Client intent law:** the client communicates intent; the server determines reality.

**State ownership law:** every mutable authoritative state has exactly one logical owner/writer at a time.

## Consequences

- no client-supplied damage, inventory quantity, RNG, cooldown completion, spawn result or final position is trusted;
- zones own live world mutation;
- cross-zone work is message-based;
- database commits are not alternate live writers;
- ownership transfers must be atomic and idempotent.
