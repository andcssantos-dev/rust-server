# ADR 0013 — Server-Owned Initial Frontier

## Status

Accepted for AF2-007A.

## Context

AF2-006 established deterministic global world coordinates, signed discovery `QuadrantCoord`, server-owned quadrant geometry, and deterministic quadrant identity without requiring potential world space to be materialized.

Aurenfall now needs the first concrete set of discovered world space: the small frontier that exists when a universe begins or when a player is initially admitted to that universe.

The implementation must preserve the core world rule:

```text
potential world space
!=
materialized runtime state
```

Creating an effectively enormous universe must not mean creating an enormous collection of `Potential` quadrant objects.

## Decision

### 1. The initial frontier is authoritative domain state

`aurenfall-domain` owns an `InitialFrontier` value built from:

```text
minimum QuadrantCoord
width
height
```

The initial implementation is rectangular because the development universe needs a simple, deterministic 4×4 frontier before more advanced discovery shapes are introduced.

The development configuration is:

```text
min = (-1, -1)
width = 4
height = 4
```

which reveals exactly:

```text
(-1,-1) (0,-1) (1,-1) (2,-1)
(-1, 0) (0, 0) (1, 0) (2, 0)
(-1, 1) (0, 1) (1, 1) (2, 1)
(-1, 2) (0, 2) (1, 2) (2, 2)
```

### 2. Potential quadrants remain implicit

A quadrant outside the revealed frontier does not receive a runtime `Potential` object.

For example:

```text
Quadrant(3, 0)
```

is mathematically addressable through AF2-006, but AF2-007A allocates no discovery-state object, database row, simulation task, Unreal actor, collider, or terrain for it.

In the initial frontier model:

```text
present in InitialFrontier -> revealed by bootstrap
absent from InitialFrontier  -> not materialized by bootstrap
```

A richer lifecycle (`Candidate`, `Prepared`, `Revealed`, `Active`, `Dormant`) may later own explicit state only for quadrants that actually enter that lifecycle.

### 3. The server config owns the bootstrap shape

The server configuration contains:

```toml
[universe.initial_frontier]
min_x = -1
min_y = -1
width = 4
height = 4
```

The client does not choose these values.

The Unreal debug 4×4 grid created during UE-C001B is only a development visualization and is not authoritative world discovery state.

### 4. Frontier enumeration is deterministic

`InitialFrontier::quadrants()` enumerates coordinates row-major:

```text
Y outer loop
X inner loop
```

This provides a stable future input to the AF2-007B wire manifest and avoids depending on `HashMap` iteration order.

### 5. Bootstrap dimensions are bounded before allocation

Each initial frontier axis is bounded to at most 64 quadrants.

The bound protects startup from accidental pathological configuration before `Vec` allocation. It is not a statement that discovered worlds can only ever be 64×64; it applies only to the single bootstrap frontier constructor.

Coordinate overflow at the positive `i64` boundary is rejected before materialization.

### 6. AF2-007A does not send the frontier to clients yet

This slice establishes authoritative domain/config/runtime ownership only.

The following is deliberately deferred to AF2-007B:

```text
InitialFrontier
-> wire FrontierManifest
-> reliable transport
-> client consumption
```

This keeps network encoding separate from domain state and prevents `aurenfall-domain` from depending on transport or contracts.

## Consequences

### Positive

- The server, not Unreal, decides the initial revealed world area.
- Only 16 discovery coordinates are materialized in the development universe.
- The rest of the theoretical universe retains effectively zero discovery-state allocation cost.
- Enumeration is deterministic and ready for a binary manifest.
- Domain ownership remains independent from QUIC and UE5.

### Trade-offs

- The initial bootstrap shape is rectangular for now.
- The runtime does not yet persist the revealed frontier to PostgreSQL.
- The frontier is not yet transmitted to protocol-client or UE5.
- Full lifecycle transitions are intentionally deferred.

## Validation required

AF2-007A must prove:

- a 4×4 frontier from `(-1,-1)` has exactly 16 unique positions in deterministic row-major order;
- `(0,0)` and `(2,2)` are revealed;
- `(3,0)` and `(-2,0)` remain absent/implicit;
- zero and oversized dimensions are rejected before allocation;
- coordinate overflow is rejected;
- server config loads the 4×4 frontier;
- world-server boot logs the authoritative revealed bounds and count;
- existing workspace gates remain green.

## Follow-up

AF2-007B will define an identity-only initial `FrontierManifest` contract and send it through the reliable authenticated session foundation. The UE client will then materialize exactly the quadrant coordinates supplied by the server instead of deciding a local 4×4 grid.
