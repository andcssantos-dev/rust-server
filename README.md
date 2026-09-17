# Aurenfall Server 2

Authoritative server platform for Aurenfall and the Persistent Procedural Frontier universe.

## Authority laws

1. **The client communicates intent. The server determines reality.**
2. **Every mutable authoritative state has exactly one logical owner/writer.**
3. **Persistent/economic mutations are idempotent, auditable and transactional.**
4. **GameData is authored for humans, validated/compiled before runtime, and read from immutable in-memory snapshots.**
5. **No database, filesystem, network peer or external service may block a world/zone simulation tick.**

## Selected stack

- Server language: Rust 1.98.0, repository-pinned
- Async runtime: Tokio
- Transport: QUIC via Quinn behind a transport boundary
- Wire schemas: FlatBuffers schemas, versioned contracts
- Configuration: TOML
- GameData authoring: YAML -> validated/compiled pack
- Persistence: PostgreSQL
- Cache: specialized in-process Aurenfall caches; distributed cache only when justified
- Simulation: fixed-rate, single-writer zone runtimes
- World: deterministic universe/sector generation + materialized deltas
- Observability: structured tracing from day one

## Workspace

```text
apps/
  world-server/          authoritative runtime process
  gamedata-compiler/     validates/compiles human-authored GameData
  protocol-client/       local protocol/transport probe
  dev-certgen/           persistent local TLS identity generator
crates/
  core/                  strongly typed IDs, time, universe coordinates
  contracts/             versioned protocol contracts
  config/                process configuration
  gamedata/              definitions, validation, immutable snapshots
  domain/                items, ownership and gameplay-domain foundations
  session/               authoritative live session and intent ingress
  simulation/            zones, world ownership and fixed ticks
  transport/             QUIC/session transport adapter
  persistence/           PostgreSQL adapters and durable operations
  cache/                 bounded specialized in-process caches
  observability/         tracing/metrics bootstrap
schemas/                  FlatBuffers protocol schemas
gamedata/                 source GameData
migrations/               PostgreSQL schema migrations
config/                   deploy/runtime config
docs/adr/                 architecture decision records
scripts/windows/          Windows bootstrap/run/preflight helpers
```

## Windows development workflow

The repository pins Rust 1.98.0 plus `rustfmt` and Clippy through `rust-toolchain.toml`.

During normal development, run the local preflight. By default it formats Rust sources first, then verifies formatting and runs the workspace check, Clippy with warnings denied, and all tests:

```powershell
Set-ExecutionPolicy -Scope Process Bypass
.\scripts\windows\preflight.ps1
```

Use check-only mode when you want validation without modifying source formatting:

```powershell
.\scripts\windows\preflight.ps1 -CheckOnly
```

Before a milestone or merge, run the complete bootstrap. It ensures the pinned toolchain/components and local TLS identity exist, invokes the autoformat preflight, and validates GameData:

```powershell
.\scripts\windows\bootstrap.ps1
```

Then start the server normally:

```powershell
.\scripts\windows\run-server.ps1
```

For repeatable AF2-013 water presentation proofs, start the development-only WATER lab mode instead:

```powershell
.\scripts\windows\run-server-water-lab.ps1
```

The WATER lab does not teleport from the client or change the authority model. It enables the existing server-side `AURENFALL_DEV_HYDROLOGY_PROOF_FRONTIER` path: Rust deterministically searches the configured universe around the normal frontier for a WATER-bearing Quadrant, manifests a frontier around that target, and chooses a safe non-WATER WALKABLE spawn cell for the authoritative character state. The mode is rejected outside the `development` environment.

Stop either mode with `Ctrl+C`.
