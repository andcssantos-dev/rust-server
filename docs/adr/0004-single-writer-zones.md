# ADR-0004: Single-Writer Zone Simulation

Status: Accepted

## Decision

Each active zone has exactly one logical simulation writer. Network tasks, persistence workers and other zones submit bounded commands/events rather than mutating zone state directly.

## Why

This removes large classes of lock ordering, race and duplicate-loot bugs and makes tick cost measurable per zone.
