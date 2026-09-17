# ADR-0002: Rust for the Authoritative Server

Status: Accepted

## Decision

Use stable Rust for authoritative server processes.

## Rationale

The project prioritizes predictable native performance, strong compile-time typing, memory safety without a garbage collector, safe concurrency primitives and long-lived infrastructure. The Unreal client remains C++ and communicates only through versioned contracts.

## Constraint

Avoid unsafe Rust in first-party server crates unless a later ADR documents a measured requirement and the safety invariants.
