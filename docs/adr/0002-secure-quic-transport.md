# ADR 0002 — Secure QUIC Transport Foundation

## Status

Accepted for AF2-002; realtime ingress activated by AF2-003C.

## Decision

Aurenfall uses QUIC with TLS 1.3 as the secure session and reliable transport foundation. The Rust server uses Quinn. The UE client will implement the same protocol contract through a compatible QUIC implementation.

Realtime gameplay transport remains capability-based rather than domain-coupled to Quinn. QUIC DATAGRAM is the initial realtime implementation, but a dedicated authenticated UDP lane remains an allowed future implementation if production-equivalent benchmarks justify it. Gameplay/domain code must not depend directly on a Quinn connection type.

The transport layer owns connection admission, cryptographic transport, stream/datagram budgets, idle/handshake timeouts and connection capacity. It does not own gameplay rules.

## Bootstrap protocol

Before normal gameplay contracts are accepted, every connection performs a small fixed binary bootstrap handshake over the first bidirectional bootstrap stream:

`ClientHello -> ServerHello`

The bootstrap stream is intentionally short-lived. It is not the future persistent authenticated control channel. After AF2-002, authenticated control/reliable streams may be established under the admitted session contract.

The bootstrap envelope is deliberately tiny and dependency-light so protocol compatibility can be determined before higher-level schemas are parsed. Post-handshake gameplay contracts remain intended for the versioned FlatBuffers layer.

Protocol major versions must match. A client minor version may be accepted only when it is less than or equal to the server minor version; the server returns the negotiated minor version in `ServerHello`.

The entire bootstrap, including QUIC establishment, bootstrap-stream acceptance, `ClientHello` parsing and `ServerHello` transmission, is constrained by one absolute handshake deadline rather than a fresh timeout per stage.

A rejected bootstrap must remain observable to a conforming client. After writing a rejected `ServerHello`, the server does not immediately issue QUIC `CONNECTION_CLOSE`, because doing so can race with delivery of the response to the peer application. The client closes after consuming the rejection. A bounded server-side rejection-close grace period prevents a non-conforming peer from retaining connection capacity indefinitely; after that grace period the server forces the close.

## Runtime identity semantics

`ConnectionId` and `SessionId` are server-generated runtime identifiers.

- `ConnectionId` is allocated only after the cryptographic QUIC connection is established.
- `SessionId` is allocated only after protocol/build admission succeeds.
- Numeric value `0` is reserved to mean “not assigned”; a rejected `ServerHello` therefore carries `session_id = 0`.
- Both identifiers are scoped to one server-process lifetime and may be reused after a process restart.
- They are not authentication credentials, secrets, account IDs, character IDs or durable persistence identities.
- A future multi-process gateway/world topology must pair runtime identity with explicit process/instance routing semantics rather than assuming these counters are globally unique.
- Monotonic allocation is checked and must fail rather than silently wrap.

## Security foundations

- TLS 1.3 only.
- Custom ALPN `aurenfall/1`.
- Explicit protocol major/minor compatibility check.
- Minimum supported client build.
- Persistent local self-signed certificate for Windows development; production uses an operational certificate.
- Maximum concurrent connection budget, held across incomplete handshakes as well as admitted sessions.
- Maximum bootstrap-frame payload budget before allocation.
- One absolute bootstrap-handshake deadline, a bounded rejection-close grace period and a separate post-admission idle timeout.
- Bounded QUIC datagram send/receive buffers.
- Sliding 64-sequence replay window foundation for future intent/datagram channels.
- Server-generated connection/session identity; client IDs are never authoritative.
- Connection tasks are supervised and reaped by the transport runtime instead of being permanently detached.

`max_datagram_bytes` is an application protocol budget. AF2-003C enforces it before decoding live QUIC DATAGRAM frames; oversized application datagrams do not enter the session boundary.

## Reliability classes

Current mapping:

- reliable transport: QUIC streams for bootstrap and future authentication, inventory, crafting, ownership/economy mutations, party/social and world transitions;
- realtime transport: a transport-agnostic lane for replaceable time-sensitive intents/state. AF2-003C uses QUIC DATAGRAM for the first live movement intent; authenticated raw UDP remains benchmark-gated.

No economically critical mutation will rely solely on an unreliable realtime lane.

## Process supervision

The world-server process treats the zone runtime, authoritative session runtime and transport runtime as supervised core tasks. If a core runtime exits before a requested shutdown, the process initiates shutdown of the siblings and exits with an error rather than remaining apparently healthy with a missing subsystem.

Transport binding and TLS configuration are completed before simulation tasks are spawned so startup failure cannot leave an unsupervised zone loop behind.

## Explicitly deferred to AF2-003+

AF2-002 does not claim to provide authenticated account sessions, per-IP rate limiting, gameplay intent authorization, datagram sequence enforcement, session replacement/reconnect policy or distributed session routing. AF2-003 adds these capabilities incrementally on top of the transport foundation; credential authentication, rate limiting, reconnect policy and distributed routing remain deferred after AF2-003C.

A production-equivalent realtime transport benchmark must compare QUIC DATAGRAM with a dedicated authenticated UDP implementation before the hot-path transport is treated as final.

## Local development

`aurenfall-dev-certgen` creates `config/tls/dev-cert.der` and `dev-key.der`. These are ignored by Git. The protocol test client trusts the generated certificate explicitly and verifies `localhost`.

The bootstrap script creates the identity if missing. It is intentionally persisted rather than regenerated every server start so local trust is stable. The repository-local `rust-toolchain.toml` selects Rust 1.98.0; the bootstrap script does not change the developer's global default Rust toolchain.
