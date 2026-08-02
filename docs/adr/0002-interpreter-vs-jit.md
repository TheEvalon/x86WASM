# ADR-0002: Interpreter is semantic truth; Wasm JIT later

- Status: Accepted
- Date: 2026-08-02

## Context

Full-system x86 correctness is hard; a JIT alone is a poor specification.

## Decision

Ship a **reference interpreter** first. Any Wasm JIT must match interpreter architectural state (lockstep/differential tests). Browser APIs never enter core CPU crates.

## Consequences

Milestone 1–3 focus on interpreter correctness. JIT crate deferred until Milestone 4. Performance work cannot claim correctness without interpreter parity.
