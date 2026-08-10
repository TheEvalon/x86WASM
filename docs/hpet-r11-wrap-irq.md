# HPET 32-bit wrap + level IRQ re-assert — Milestone 2 Round 11

## Why

R7/R8/R10 advanced a 32-bit main counter and wired Timer 0 to I/O APIC GSI 2,
but wrap crossings lacked an explicit test and level EOI left a still-asserted
HPET pin idle (Remote IRR cleared without re-drive).

## Spec

- IA-PC HPET Specification 1.0a — main counter with `COUNT_SIZE_CAP` clear
  (32-bit); comparator match across wrap
- Intel 82093AA — level RTE: EOI clears Remote IRR; pin still high may
  re-deliver

## Model

| Piece | Behavior |
|---|---|
| `HpetMmio::advance_main_counter` | 32-bit wrap; fire if comparator in crossed range |
| `Machine::eoi_lapic_ioapic` | EOI + `sync_hpet_irq_to_ioapic` so a still-high level line re-asserts |

Multi-wrap inside a single huge `delta` is **not** fully simulated (at most one
wrap edge evaluated) — call hosts with bounded deltas.

## Not wired (explicit)

- 64-bit counter mode (`COUNT_SIZE_CAP` set)
- Legacy-replacement / FSB / MSI
- Auto-advance from the instruction step clock

## Tests

- `crates/devices/src/hpet.rs`
  - `main_counter_32bit_wrap_crosses_comparator`
  - `main_counter_wrap_fires_when_comparator_near_top`
- `crates/machine-pc/tests/hpet_mmio.rs`
  - `machine_hpet_level_ioapic_sets_remote_irr` (re-assert after EOI)
  - `machine_hpet_level_eoi_no_reassert_after_status_clear`
