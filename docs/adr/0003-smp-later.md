# ADR-0003: Single vCPU first; SMP later

- Status: Accepted
- Date: 2026-08-02

## Context

SMP introduces nondeterminism and APIC complexity before the uniprocessor path boots OSes.

## Decision

**One virtual CPU** through early OS bring-up. Cooperative/deterministic scheduling and a second vCPU come only after Windows 7 x64–class uniprocessor goals (see `plan.md`).

## Consequences

No IOAPIC/x2APIC multi-CPU work in M1. CPUID must not advertise multi-core topologies we do not implement.
