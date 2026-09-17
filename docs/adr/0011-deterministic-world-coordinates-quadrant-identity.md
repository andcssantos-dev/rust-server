# ADR 0011 — Deterministic World Coordinates & Quadrant Identity

## Status

Accepted for AF2-006A.

## Context

Aurenfall's authoritative world is intended to be effectively enormous without requiring undiscovered space to exist as allocated runtime state, database rows, Unreal actors, streaming cells, or simulation tasks.

The server therefore needs a stable mathematical answer to two questions:

1. Which logical discovery quadrant contains an authoritative world position?
2. Which deterministic seed belongs to that quadrant for a specific universe and generator version?

The existing `aurenfall-core` already owns `WorldPositionMm`, `UniverseSeed`, and legacy `SectorCoord` / `SectorSeed` primitives. AF2-006 introduces the new discovery-world terminology without coupling it to Unreal or runtime zone ownership.

## Decision

### 1. Quadrant is a logical discovery coordinate

Introduce:

```text
QuadrantCoord { x: i64, y: i64 }
```

Coordinates are signed because the world expands in every horizontal direction.

`QuadrantCoord` is deliberately independent from:

- `ZoneId`;
- Unreal World Partition cells;
- network AOI cells;
- rendering cells;
- physics broadphase cells;
- simulation ownership.

A future zone may own many quadrants, and that ownership may change without changing quadrant identity.

### 2. Authoritative physical position remains `WorldPositionMm`

Character movement remains the single writer of authoritative position.

Quadrant membership is a derived observation:

```text
WorldPositionMm
+ QuadrantSizeMm
→ QuadrantCoord
```

No second mutable source of character location is introduced.

Z does not participate in discovery-quadrant identity in AF2-006.

### 3. Quadrant size is strongly typed

Introduce `QuadrantSizeMm`, which can only be constructed from a positive `i64` millimetre value.

The prototype documents use 512,000 mm (512 m), but AF2-006A does not hard-code that value into runtime world logic. Tests use it as the prototype vector.

The runtime source of this value will be connected to server-owned world configuration after AF2-005 is merged, because world geometry configuration belongs to the server and changing quadrant size changes the spatial meaning of an existing universe.

### 4. Negative coordinates use Euclidean division

World-to-quadrant conversion uses integer `div_euclid` semantics.

For a 512,000 mm quadrant:

```text
 0        →  0
 511999   →  0
 512000   →  1
-1        → -1
-512000   → -1
-512001   → -2
```

Truncation toward zero is forbidden because it would make the grid asymmetric around the origin.

### 5. Quadrant seeds are deterministic and domain separated

Introduce:

```text
UniverseSeed::quadrant_seed(QuadrantCoord, GeneratorVersion)
→ QuadrantSeed
```

The V1 derivation uses BLAKE3 keyed by the universe seed and an explicit stable byte sequence:

```text
"AURENFALL_QUADRANT_V1"
GeneratorVersion as little-endian u32
Quadrant X as little-endian i64
Quadrant Y as little-endian i64
```

This deliberately avoids:

- Rust `HashMap` hashing;
- memory-layout serialization;
- native-endian integers;
- wall clock state;
- RNG state;
- database state;
- client state.

The same universe seed, coordinate, and generator version must produce the same `QuadrantSeed` across supported machines, processes, and restarts.

### 6. Potential quadrants require no materialized state

A `QuadrantCoord` and its deterministic seed are addressable without creating:

- a database row;
- a runtime quadrant object;
- a simulation task;
- a UE actor;
- a streaming cell.

This is the first concrete primitive behind Aurenfall's rule that potential world space has effectively zero materialization cost.

### 7. Legacy Sector API is retained temporarily

`SectorCoord`, `SectorSeed`, and `UniverseSeed::sector_seed` remain during AF2-006A because the current world-server composition root still uses the older bootstrap terminology.

New discovery-world code should use the Quadrant API.

After AF2-005 is merged, a later AF2-006 integration slice will migrate the composition root and server configuration deliberately rather than creating cross-branch conflicts now.

## Tests required in AF2-006A

The core tests cover:

- invalid quadrant size;
- origin;
- positive interior and boundaries;
- negative near-origin coordinates;
- negative exact and beyond-boundary coordinates;
- mixed signs;
- extreme valid `i64` world coordinates;
- seed repeatability;
- coordinate variance;
- generator-version variance;
- universe variance;
- explicit domain-separated little-endian seed encoding.

Golden fixed output vectors should be added before AF2-006 is considered fully complete, after the implementation has passed the local Rust gate and the resulting V1 outputs are deliberately frozen.

## Consequences

### Positive

- The theoretical universe can be addressed without materialization.
- Negative coordinates are mathematically correct.
- Discovery identity is independent from runtime ownership and Unreal streaming.
- Future procedural systems gain a stable deterministic parent seed.
- The primitive is cheap enough for hot-path observation: integer division only, with no allocation, I/O, database, or network access.

### Trade-offs

- Changing quadrant size for an existing universe is a world-generation compatibility change.
- The legacy Sector API temporarily coexists with Quadrant terminology.
- AF2-006A intentionally does not implement quadrant lifecycle, manifests, terrain, biomes, POIs, persistence, or Unreal materialization.

## Follow-up

After AF2-005 is on `main`:

1. bind `QuadrantSizeMm` to server-owned world configuration;
2. migrate genesis/bootstrap terminology from Sector to Quadrant where semantically appropriate;
3. freeze golden deterministic seed vectors;
4. then proceed to quadrant lifecycle and the initial 4×4 frontier.
