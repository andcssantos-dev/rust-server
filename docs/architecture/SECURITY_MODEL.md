# Security Model

## Trust model

The client is hostile input. A legitimate UE5 build receives no special trust.

The server alone authorizes movement, combat, RNG, cooldowns, items, ownership, currency, crafting, resource state, world manifestation, AI, progression and persistence.

## Network admission pipeline

Every inbound message passes through bounded stages:

1. transport/TLS validity;
2. connection/session state;
3. frame and payload size budget;
4. protocol/schema version validation;
5. authentication/authorization where required;
6. per-connection and per-command rate limits;
7. sequence/anti-replay checks;
8. semantic validation;
9. bounded zone/service queue.

No unbounded client-controlled allocation is allowed.

## QUIC

QUIC provides encrypted transport. Application protocol remains responsible for authentication, permissions, replay semantics, input budgets and gameplay validation.

## Economic safety

Critical operations use unique operation IDs and database transactions. Retrying the same operation cannot duplicate ownership or currency.

Examples: trade, auction purchase, mail attachment, guild vault mutation, rare loot ownership transfer, crafting output and destructive item operations.

## Secrets

Secrets never live in Git-tracked TOML/YAML. Production credentials come from deployment secret stores/environment injection. Local secrets use ignored files.

## Administrative surfaces

Admin commands are separate from gameplay messages, strongly authenticated, permission-scoped and audited. No hidden client packet becomes an admin command.

## Security testing

The target test suite includes malformed frames, oversized payloads, schema fuzzing, replay attempts, command floods, disconnect races and invariant/property tests around ownership and quantities.
