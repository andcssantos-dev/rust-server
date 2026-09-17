# ADR-0003: Modular Monolith Before Distributed Services

Status: Accepted

## Decision

Start as a modular monolith. Module boundaries are real dependency boundaries, but process boundaries are introduced only for measured operational/scaling reasons.

## Rule

Domain code cannot depend on QUIC, PostgreSQL or Unreal-specific types. Infrastructure adapts to the domain, not the reverse.
