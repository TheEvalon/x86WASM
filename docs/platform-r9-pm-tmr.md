# ACPI PM_TMR freerun via machine PIT quantum — Milestone 2 Round 9

## Why

Round 5 wired `PM_TMR` to the instruction-count [`StepClock`] path only
(`advance_step_clock` → `PciConfig::tick_acpi_pm`). Hosts that advance time
through [`Machine::tick_pit`] alone (or any single PIT quantum) left the
counter stuck, so firmware delay loops that sample PMBASE+`08h` could still
wedge outside `--post-probe`.

## Spec

- Intel 82371AB (PIIX4) — Power Management Timer at PMBASE+`08h`: 24-bit
  free-running counter, frequency **3.579545 MHz** (14.31818 MHz / 4).
- That rate is exactly **three** IBM PC/AT 8254 input clocks (1.193182 MHz).
- ACPI 6.x / PIIX4 — counter is free-running while the PM I/O decode is live;
  this model advances it whenever the machine charges PIT clocks.

## Model (R9)

[`Machine::tick_pit`] is the **single** PM_TMR advance path:

| Caller | Effect |
|---|---|
| `Machine::tick_pit(n)` | PIT ch0/ch2 + `tick_acpi_pm(n × 3)` |
| `StepClock` / `advance_step_clock` | calls `tick_pit` (no second PM poke) |
| Halt-idle POST probe | multi-quantum `advance_step_clock` → same path |

Register authority remains `PciConfig::acpi_pm_io[PM_TMR]` (no second field).

## Unsupported

- No wall-clock accuracy; instruction-count / host quantum only.
- No edit of the PCI register file in this lane — uses existing
  `PciConfig::tick_acpi_pm`.
- `RDTSC` is not advanced here (CPU ownership).
- Channel-1 DRAM-refresh countdown is still a separate `tick_ch1` call
  (see `docs/pit-r9-port61.md` when present).

## Tests

- `crates/machine-pc/tests/acpi_pm_tmr.rs` — step clock, wrap, and
  `tick_pit`-only freerun.
