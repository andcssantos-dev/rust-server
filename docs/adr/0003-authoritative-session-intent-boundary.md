# ADR 0003 — Authoritative Session and Intent Boundary

## Status

Accepted for AF2-003A/AF2-003B/AF2-003C/AF2-003D.

## Decision

Aurenfall separates transport identity from gameplay authority.

A QUIC connection and its runtime `ConnectionId`/`SessionId` prove only that the server admitted a protocol session. They do not identify an account, character or world owner by themselves.

Gameplay authority is created only by a server-owned session state machine:

`ProtocolAdmitted -> Authenticated -> CharacterBound -> WorldActive -> Closing`

Only a `WorldActive` session may produce an `AuthorizedIntent`.

## Server-owned identity derivation

Client intent payloads do not carry authoritative `SessionId`, `AccountId`, `CharacterId` or `ZoneId` values.

The server derives those identities from the admitted session binding after authentication and character/world admission. The client supplies only the action-specific intent data and an intent sequence.

For movement, the AF2-003A wire contract is intentionally small:

- `sequence: u64`
- `axis_x: i16`
- `axis_y: i16`

The payload is 12 bytes and contains no authoritative identity fields. Signed fixed-point input axes are used at the trust boundary instead of floating-point direction values so NaN/infinity cannot enter through the wire contract. The asymmetric `i16::MIN` value is rejected, leaving the supported input range symmetric at `-32767..=32767`.

## Session state invariants

- A newly protocol-admitted session begins in `ProtocolAdmitted`.
- Authentication binds a server-resolved `AccountId`.
- Character admission binds a server-resolved `CharacterId` and `ZoneId`.
- World activation is explicit and occurs only after character binding.
- `Closing` is terminal for gameplay authorization.
- Runtime `ConnectionId` and `SessionId` must be non-zero when constructing an authoritative session.
- Account, character and zone identities must be non-zero when bound.
- Illegal state transitions are rejected instead of silently repairing or skipping phases.

The session state stores the authoritative binding internally so partial combinations such as an authenticated account with an unrelated client-supplied character cannot be represented through the public authorization API.

## Live session registry

AF2-003B introduces a live registry whose public ingress lookup is keyed by the server-owned `ConnectionId`.

The registry owns both indexes:

- `SessionId -> live authoritative session`;
- `ConnectionId -> SessionId`.

Duplicate session IDs and duplicate live connection bindings are rejected. Removing a connection removes its live session binding and its per-session sequence history.

The registry is intentionally a mutable single-owner data structure rather than a shared lock-based global map. The AF2-003C world-server session runtime owns this mutable authority and receives bounded transport events without introducing a shared global lock.

AF2-003D also exposes a read-only world-active membership check by server-owned `SessionId`. The session runtime uses this immediately before routing an already-authorized queued intent. This prevents an `AuthorizedIntent` waiting in the internal queue from reaching the world after its originating session has entered `Closing` or has been removed.

## Sequence and replay semantics

The replay primitive is transport-agnostic and lives in `aurenfall-core`; QUIC is not the owner of sequence semantics.

Each live session has an independent 64-sequence replay window. The authority check happens before sequence consumption, so an intent rejected because the session is not `WorldActive` does not burn a sequence number.

The replay window can distinguish:

- a new highest sequence;
- a previously unseen out-of-order sequence;
- a duplicate sequence;
- a sequence older than the replay window.

For replaceable movement intents, AF2-003B accepts only a new highest sequence. Even an unseen packet that arrives behind a newer movement intent is rejected as stale/out-of-order so old input cannot overwrite newer player intent.

A new live session starts with a fresh sequence window. Sequence values are not credentials and do not grant authority.

## Bounded authorized ingress

AF2-003B adds a bounded output queue for server-authorized movement intents.

Movement follows the validation order `session authority -> sequence/replay -> bounded queue`. Once a movement sequence passes authority and freshness checks, that sequence is consumed even if the bounded queue is full and the replaceable movement intent must be dropped. The client should continue with newer movement sequences rather than retrying stale movement state.

Only `AuthorizedIntent` values can enter this queue. Raw client contracts do not flow directly into world or simulation ownership.

This drop-on-backpressure policy applies only to replaceable movement intents. Economically critical or durable mutations will require operation-specific delivery, acknowledgment and idempotency semantics rather than inheriting movement-drop behavior.

## Live QUIC intent wiring

AF2-003C wires the first real gameplay intent from QUIC into the authoritative session boundary.

The transport-to-session boundary uses a bounded `TransportSessionEvent` channel. The transport adapter can emit only server-observed events:

- protocol session admitted with server-assigned `ConnectionId` and `SessionId`;
- decoded `MoveIntent` associated with the server-owned `ConnectionId` of that live QUIC connection;
- transport disconnect for that same server-owned connection/session pair.

A protocol admission event is fail-closed. If the bounded session-event channel cannot accept the admission, the transport closes the connection instead of creating a network session that has no corresponding authoritative session owner.

Movement events are replaceable and may be dropped by the transport adapter when that bounded event channel is full. A movement event dropped before the session boundary does not consume the per-session replay sequence because it never reached the sequence owner. The client continues sending newer movement state.

The application enforces `network.max_datagram_bytes` when decoding QUIC DATAGRAM frames. The initial live movement datagram is an Aurenfall frame header plus the 12-byte identity-free `MoveIntent` payload.

QUIC DATAGRAM is the initial realtime carrier, consistent with ADR 0002. Session/domain authority does not depend on Quinn and may later receive the same logical transport events from another authenticated realtime carrier if benchmarking justifies one.

## Development-only automatic admission

AF2-003C needs a locally observable authorized movement path before account authentication and persistence are implemented.

When `server.environment = "development"`, the world server therefore applies a temporary development-only admission policy after protocol admission. It derives temporary `AccountId` and `CharacterId` values from the already server-assigned runtime `SessionId`, binds the configured development zone, and advances that session to `WorldActive`.

This is not authentication and is not a production identity model. No identity value in this development policy comes from the client. In any non-development environment, this automatic binding is disabled and a protocol-admitted session remains unable to authorize gameplay until a real server-side authentication/character admission path advances it.

## Authorized world routing

AF2-003D introduces a world-router boundary in the `world-server` composition layer.

The router does not choose gameplay identity or trust client routing data. It consumes an `AuthorizedIntent`, reads the server-derived `ZoneId`, resolves that zone to a bounded `ZoneCommand` sender, and converts the authorized payload into the simulation command expected by the zone single-writer.

Routing policy is:

- an unregistered authoritative `ZoneId` is a server consistency error;
- a closed zone command channel is a runtime consistency error;
- a full zone command queue drops replaceable movement rather than blocking the session owner;
- a queued `AuthorizedIntent` is discarded before routing if its server-owned `SessionId` is no longer `WorldActive`.

The initial process owns one Zone 1 route, but the router uses a `ZoneId -> Sender<ZoneCommand>` map so additional zone owners can be registered without changing the authority semantics.

## Internal movement command

AF2-003D removes raw `u64` sequence values and floating-point movement arrays from `ZoneCommand::MoveIntent`.

The zone command now carries:

- server-derived `CharacterId`;
- strong `IntentSequence`;
- validated `MovementInput` containing the signed integer input axes.

`MovementInput` rejects `i16::MIN`, preserving the same symmetric axis domain established at the wire boundary. The command intentionally remains integer input rather than converting prematurely to `[f32; 2]`. Actual speed, diagonal normalization, acceleration, collision, position integration and movement-state validation belong to the authoritative simulation step, not to network ingress or routing.

The client still never sends a new position. The server will later combine this movement input with authoritative character state and movement rules during a zone tick.

## Authorized intent type

`AuthorizedIntent` is an internal server type whose authoritative identity fields are private. It can be constructed only inside the `aurenfall-session` crate after a `WorldActive` session authorizes an intent.

Downstream world routing and simulation consume this server-authorized form rather than raw client contracts.

The implemented AF2-003D path is:

`QUIC DATAGRAM -> frame/shape validation -> server ConnectionId -> bounded transport event -> live session lookup -> session authority -> sequence/replay -> bounded AuthorizedIntent queue -> live-session recheck -> WorldRouter -> bounded ZoneCommand queue -> zone single writer`

The intended complete path remains:

`wire intent -> decode/shape validation -> live session lookup -> session authority -> sequence/replay -> rate limit -> permission/state validation -> bounded router -> zone single writer -> authoritative simulation -> replication`

AF2-003A implements the session boundary and the first identity-free movement contract. AF2-003B implements the live registry, per-session sequence/replay enforcement and the first bounded authorized movement queue. AF2-003C implements the real QUIC DATAGRAM-to-session wiring and supervised world-server session runtime. AF2-003D implements live-session revalidation, authoritative world/zone routing and the strongly typed zone movement command bridge.

## Dependency direction

The session authority crate depends on core identifiers/replay primitives and Tokio bounded channels, but not on Quinn or transport implementation details.

Wire contracts remain in `aurenfall-contracts`. The QUIC transport adapter decodes wire payloads and emits transport session events. The world-server composition root feeds those events into `aurenfall-session`, which derives server-owned gameplay identity, and then routes the resulting `AuthorizedIntent` into `aurenfall-simulation` through a bounded zone command sender.

Gameplay/domain and simulation code must not receive a client-provided character identity or world position as authority.

## Explicitly deferred

AF2-003A/AF2-003B/AF2-003C/AF2-003D do not yet implement:

- real account credential verification;
- persistent account/character lookup;
- production character selection/admission;
- per-IP/session rate limiting;
- permission/state validation beyond session phase and movement sequence freshness;
- authoritative movement state/integration;
- collision and traversal validation;
- movement result/state replication;
- reconnect/session replacement policy;
- live character zone-transfer generations/epochs;
- distributed gateway/world session routing.

When live zone transfer is introduced, the authority binding must gain an explicit generation/epoch so an intent authorized for an older zone binding cannot be routed after a newer transfer commits.

Those build on this boundary without changing the rule that the server derives gameplay identity.
