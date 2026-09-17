# ADR 0004 — Authoritative Character Movement Kernel

## Status

Accepted for AF2-004A, AF2-004B and AF2-004C.

## Context

AF2-003 established an identity-free movement intent path from QUIC DATAGRAM through server-owned session authority, replay/sequence validation, bounded world routing and the zone single-writer.

AF2-004 converts that authorized input into server-owned world state. The client must never become the source of truth for character position or collision outcome.

## Decision

The `ZoneRuntime` owns all mutable character movement state for characters currently resident in that zone.

The authoritative path is:

`MoveIntent -> session authority -> AuthorizedIntent -> WorldRouter -> ZoneCommand -> CharacterState -> fixed zone tick -> traversal resolution -> server-owned position`

A movement intent changes the character's current movement input. It does not contain or directly mutate position.

## Position representation

Authoritative world position uses `WorldPositionMm` with signed `i64` millimeter coordinates for X, Y and Z.

Reasons:

- integer arithmetic is deterministic across supported server platforms;
- millimeter precision is sufficient for authoritative gameplay while still mapping cleanly to UE presentation units;
- signed 64-bit millimeters provide ample range for the intended very large procedural universe;
- Z exists in the position type now even though AF2-004 currently integrates only horizontal X/Y movement;
- overflow is checked rather than wrapped or saturated.

`SectorCoord` remains the macro world coordinate. Sector derivation, local-origin presentation and sector-boundary transitions are deferred to later world-frontier milestones.

## Movement input and normalization

Movement input remains a symmetric signed integer pair in `-32767..=32767`.

The authoritative kernel does not use floating-point math to normalize or integrate movement. Inputs whose vector magnitude exceeds the supported unit circle, such as full keyboard diagonal input, are normalized with integer arithmetic and an integer square-root operation.

Inputs already inside the unit circle preserve their analog magnitude.

This prevents full diagonal input from moving faster than full cardinal input without introducing floating-point nondeterminism into the authoritative kernel.

## Tick integration and fractional remainder

Movement speed is expressed in millimeters per second and integrated on the fixed zone tick.

Integer division can produce sub-millimeter fractions. Each `CharacterState` therefore owns a signed per-axis movement remainder. The remainder is carried into later ticks so repeated truncation does not create long-term speed drift.

For example, a speed that cannot divide evenly into 60 ticks still accumulates to the correct whole-millimeter distance over time.

The remainder never creates movement on its own; it is only fractional state used by later integrations.

## Character movement settings

The server groups baseline movement tuning in `CharacterMovementSettings`:

- movement speed in millimeters per second;
- movement-input timeout in authoritative ticks;
- horizontal character traversal radius in millimeters.

These settings are server-owned. AF2-004 currently supplies development values from the world-server composition root. Final character/archetype tuning belongs in validated server GameData and must not be supplied by the client or UE presentation layer.

## Character state ownership

Each live character in a zone has a `CharacterState` containing:

- server-owned `CharacterId`;
- authoritative `WorldPositionMm`;
- current `MovementInput`;
- last accepted `IntentSequence`;
- authoritative tick at which the last input was received;
- fractional movement remainder;
- last simulated `ServerTick`.

The zone stores these states in its single-writer runtime. No transport/session task mutates position directly.

The zone defensively requires movement sequences to remain monotonically increasing even though the session boundary already enforces replay/freshness. A non-monotonic sequence reaching the zone is treated as an internal consistency failure rather than client authority.

## Movement input freshness and deadman safety

A non-zero movement input is temporary state, not a durable instruction to move forever.

When the zone accepts a `MoveIntent`, it records the current authoritative `ServerTick` as the input receive tick. Each later simulation tick computes the input age using server ticks only.

If the age reaches the configured movement-input timeout, the input is replaced with `MovementInput::ZERO` before displacement for that timeout tick is integrated.

For a timeout of five ticks and an input accepted at tick `N`:

- ticks `N+1` through `N+4` may integrate that input;
- at tick `N+5`, the deadman expires it to zero before movement integration;
- later ticks remain stopped until a newer monotonic input arrives.

This means packet silence, a frozen client process or transient network loss cannot cause indefinite authoritative movement.

The deadman is edge-triggered. Once a stale non-zero input is zeroed, the expiry event is not emitted again on later ticks.

A newer monotonic input received after deadman expiry can resume movement normally.

An explicit zero input also stops movement immediately and clears the fractional movement remainder. This prevents fractional movement accumulated under an earlier direction from becoming residual movement debt after the player has explicitly stopped.

The AF2-004B development baseline is five ticks. At the current 20 Hz development tick rate, this corresponds to 250 ms of input silence.

## Authoritative traversal foundation

AF2-004C inserts an explicit traversal boundary between desired displacement and committed authoritative position.

The movement kernel now produces a desired horizontal delta. Before `CharacterState` commits a new position, the zone asks its authoritative `TraversalWorld` to resolve that displacement.

The path becomes:

`current position -> desired movement delta -> TraversalWorld -> resolved position -> CharacterState commit`

The client does not participate in the traversal decision.

### Initial collision representation

The first traversal backend uses deterministic static axis-aligned blockers in X/Y millimeter space.

Each blocker is an AABB with validated min/max bounds. The character is represented for this foundation by a server-owned horizontal radius. Collision is resolved by expanding blockers by the character radius and moving the character center against those expanded bounds.

This is a deliberately simple mathematical server representation. It is not UE collision geometry and does not depend on meshes, Nanite, materials, animation or presentation assets.

### Swept axis resolution

Traversal resolves horizontal movement axis-by-axis in a deterministic X-then-Y order.

For each axis, the resolver checks whether the desired segment crosses an expanded blocker boundary and clamps the character center to that boundary. This prevents large single-tick deltas from tunneling through thin blockers.

Resolving axes separately also provides an initial wall-slide behavior: if one axis is constrained while the other remains valid, the valid axis may still move.

This axis-separated approach is a foundation rather than the final collision model. More advanced swept shapes, terrain, slopes, steps and dynamic blockers can replace or extend the backend without changing the invariant that traversal is decided server-side before position commit.

### Spawn validation

`SpawnCharacter` is validated against the same authoritative traversal world before the character is inserted into the zone. A spawn whose footprint overlaps a blocker is rejected as a server consistency error.

Movement never implicitly teleports or forces a character through an invalid placement.

### Development wall probe

In the current `development` environment only, the world-server composition root installs one deterministic static wall in front of the development spawn so the live protocol probe can demonstrate authoritative collision.

The development wall is not a production world feature. Production traversal blockers will eventually be derived from manifested server-owned world/sector data.

With the current development radius of 250 mm and wall beginning at X=650 mm, the character center is constrained at X=400 mm.

## Explicit spawn and despawn lifecycle

Movement does not implicitly create a character.

Server-owned lifecycle commands are explicit:

- `SpawnCharacter { character_id, position }`;
- `DespawnCharacter { character_id }`;
- `MoveIntent { character_id, sequence, input }`.

For the current development-only session admission path, the application:

1. authenticates and binds temporary server-owned account/character IDs;
2. resolves the server-owned world binding;
3. enqueues `SpawnCharacter` at a server-owned development spawn position;
4. only then advances the session to `WorldActive`.

On disconnect, the application captures the world binding before `Closing`, marks the session closing, enqueues `DespawnCharacter`, and then removes the live session binding.

This prevents a disconnected character from remaining resident after session cleanup. The deadman separately protects against movement continuing while the connection is still technically alive but no fresh movement input is arriving.

## Backpressure semantics

Movement remains replaceable state and may be dropped at bounded movement queues as established by ADR 0003.

Spawn/despawn lifecycle is not replaceable. If a zone lifecycle command cannot be enqueued because the zone queue is full or closed, the application treats that as a server consistency failure rather than silently dropping the lifecycle transition.

Future production admission may introduce a dedicated control lane or stronger lifecycle acknowledgement if profiling/load tests justify it.

## Deferred

AF2-004A/B/C intentionally do not implement:

- terrain height / authoritative Z integration;
- slopes, stairs, step-up or ledge traversal;
- dynamic blocker movement;
- broadphase/spatial indexing for large blocker sets;
- capsule/convex swept collision beyond the initial horizontal radius model;
- acceleration, deceleration or inertia;
- sprint, crouch, stamina or encumbrance modifiers;
- character archetype movement GameData;
- sector-boundary migration;
- replication of authoritative position back to clients;
- client prediction, reconciliation or interpolation;
- persistence of live movement state;
- adaptive timeout tuning based on network conditions;
- anti-speedhack heuristics beyond server-owned integration itself.

Those features build on the invariant established here: the client supplies fresh movement intent; the zone single-writer decides whether it is still valid, resolves traversal and commits authoritative position.
