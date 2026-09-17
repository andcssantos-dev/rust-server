# ADR-0005: Versioned Binary Protocol over QUIC

Status: Accepted with profiling escape hatch

## Decision

Use QUIC as the transport boundary and FlatBuffers schemas as the initial cross-language contract between Rust server and Unreal C++ client.

Reliable ordered streams are used for state that must arrive exactly/in order. QUIC datagrams may be used for replaceable latest-state traffic such as movement snapshots. A custom bit-packed codec may later replace specific hot-path messages only when profiling proves the need.
