# POST halt-idle accounting — Milestone 2 Round 5

## Why

After APM and ACPI `PM_TMR` unblocked SeaBIOS, a 2M-step probe spent most of
its budget in `HLT` with `IF=1` (`idle-steps ≈ 1.1M`). That is a **wait**, not a
busy opcode spin, but the report only printed `idle-steps=N` and still looked
like a hang.

## Changes

1. **Richer `halt-idle` line** (header unchanged):

```text
  halt-idle      idle-steps=N busy-steps=M idle-pct=P%
```

2. **Idle timer boost** — each halt-idle quantum charges
   [`POST_IDLE_TIMER_CLOCKS`] (64) instruction-count clocks to the PIT/RTC/ACPI
   PM timer path (one already runs inside `Machine::step`). Spec: Intel SDM
   Vol. 2 `HLT` — timers keep ticking while halted. This is a probe model
   choice so firmware yields do not consume the step budget 1:1 with wall time.

## Compatibility

The `post-probe: steps=… stop=…` header line remains byte-identical. Only the
indented `halt-idle` diagnostic grows fields.
