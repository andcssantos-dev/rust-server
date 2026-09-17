# Persistent Procedural Universe Model

## Principle

A server is one universe. The universe may have an enormous logical coordinate space while almost none of it is materialized.

## Sector states

### Potential

Only mathematics exists: `UniverseSeed + SectorCoord + GeneratorVersion` can determine the sector. No persistent row is required.

### Materialized

A frontier expansion or other authoritative event caused the sector to be generated. The generated identity/version becomes part of world history.

### Mutated

Gameplay changed durable facts: construction, depleted resource, discovered structure, ownership, destroyed object, persistent ecology delta or other state. Persist deltas/invariants, not unnecessary decorative geometry.

## Determinism

Generation is versioned. The same seed, coordinate and generator version produces the same logical result. Shipping generator V2 never silently rewrites sectors already bound to V1.

## Hierarchical generation

```text
Universe
  -> macro fields
  -> regions
  -> sectors
  -> points of interest
  -> structures
  -> room/interior graphs
  -> gameplay-significant entities
  -> client visual recipe
```

Large features such as roads, rivers, settlements and cave systems must be generated from higher-level continuity rules rather than independent per-sector random choices.

## Player placement

Normal players are placed far enough apart for solitary early progression but within a social distribution that makes eventual frontier contact plausible. `isolate` placement uses a separate distance policy without creating a separate universe.

## Knowledge is not existence

The world is shared. Player map knowledge is personal. Two frontiers touching makes the geography physically connected; it does not automatically reveal all discovered information to either player.

## Simulation LOD

- S0 Potential: no simulation.
- S1 Macro: aggregate population/ecology/economy values.
- S2 Dormant materialized: event/deadline based updates.
- S3 Active sector: concrete entities and normal world simulation.
- S4 Interaction bubble: highest-frequency AI/combat/perception/collision work.

Empty space never consumes a permanent 20 Hz tick.

## Unreal boundary

The server manifests gameplay reality. UE5 manifests presentation. Decorative grass, debris and other non-authoritative detail may be reproduced client-side from a visual seed/profile; interactive trees, doors, mobs, loot, resources and structures remain server-owned.
