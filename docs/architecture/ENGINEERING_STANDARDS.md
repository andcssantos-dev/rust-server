# Engineering Standards

## Code shape

- Prefer cohesive modules and small types over god objects.
- ~300-400 LOC is ordinary; >600 LOC triggers responsibility review; >800 LOC requires explicit justification.
- Do not split code mechanically just to satisfy a line count.
- Network dispatchers orchestrate; they do not duplicate domain rules.
- One invariant has one canonical implementation.

## Types

- Stable authoring keys may be human-readable strings; runtime hot-path identities resolve to strongly typed numeric/newtype IDs.
- Do not interchange CharacterId, EntityId, ContainerId, DefinitionId or ItemInstanceId through generic integers/strings.
- Prefer enums/state machines over magic strings.
- Represent units explicitly where confusion is possible.

## Mutation

Use `validate -> plan -> commit` for multi-object mutations such as inventory transfers. Validation must not partially mutate state. Failed operations preserve source state.

## Rust

- `unsafe_code = forbid` by default.
- avoid `unwrap`/`expect` in runtime paths;
- errors crossing module boundaries are typed or contextualized;
- bounded channels instead of unbounded work queues;
- profiling precedes micro-optimization.

## Tests

Every domain foundation should eventually have:

- unit tests;
- invariant/property tests;
- integration tests;
- deterministic replay tests where appropriate;
- concurrency/race tests at ownership boundaries;
- fuzz tests for parsers/protocols;
- benchmarks for declared hot paths;
- load/soak scenarios for server topology.

Critical invariants include unique ownership, quantity/currency conservation, no cyclic containers, idempotent retries and deterministic generation.

## Architecture records

Material architectural choices require an ADR. Examples from design documents are not automatically numeric gameplay commitments.
