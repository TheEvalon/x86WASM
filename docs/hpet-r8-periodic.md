# HPET Timer 0 periodic VAL_SET / comparator — Milestone 2 Round 8

## Why

R7 could re-arm Timer 0 after a fire, but `Tn_VAL_SET_CNF` snapped the period
from the *current* comparator at config-write time. Software that follows the
HPET 1.0a sequence (set VAL_SET, then write the comparator) needs the period
to update on that comparator write, then allow a later write to set the next
match without changing the period.

## Spec

- IA-PC HPET Specification 1.0a
  - Timer n Configuration: `Tn_TYPE_CNF`, `Tn_VAL_SET_CNF` (W1, not retained)
  - Timer n Comparator: period load under VAL_SET; otherwise next-match value
  - Periodic fire: comparator ← main_counter + period (32-bit mask here)

## Model

`devices::HpetMmio` Timer 0:

| Step | Behavior |
|---|---|
| Write config with `VAL_SET` | Arm `t0_val_set_pending`; bit not visible on read |
| Dword comparator write while pending + periodic | `t0_periodic_period = value`; seed comparator; clear pending |
| Later comparator write (no VAL_SET) | Updates next-match only; period unchanged |
| `advance_main_counter` crosses match | STS/IRQ; comparator ← after + period |

Period side effects commit on low-dword lane 3 so byte-wise MMIO assembly does
not snapshot a partial value.

## Not wired (explicit)

- Timer 1..N (CAPS `NUM_TIM_CAP` still 0 → Timer 0 only)
- Legacy-replacement, FSB/MSI routes, PIC/IOAPIC auto-delivery
- 64-bit counter / comparator mode

## Tests

- `crates/devices/src/hpet.rs`
  - `timer0_periodic_rearms_comparator`
  - `timer0_periodic_val_set_then_next_match_write`
