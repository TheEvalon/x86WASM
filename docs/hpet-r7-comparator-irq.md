# HPET Timer 0 comparator IRQ stub — Milestone 2 Round 7

## Why

Round-6 claimed the HPET MMIO window but left comparators / IRQ unwired.
Firmware and early OS bring-up need at least one programmable comparator that
can raise a visible interrupt latch.

## Spec

- IA-PC HPET Specification 1.0a
  - General Interrupt Status @ `20h` (write-1-to-clear)
  - Timer 0 Configuration/Capability @ `100h`
  - Timer 0 Comparator @ `108h`
  - `Tn_INT_ENB_CNF` (bit 2), `Tn_TYPE_CNF` / `Tn_PER_INT_CAP`, `Tn_INT_ROUTE_CNF`

## Model

`devices::HpetMmio` (base `0xFED0_0000`):

| Path | Behavior |
|---|---|
| CAPS / Config / Main | As R6, plus main counter **writable** (32-bit mask) |
| Intr status `20h` | Bit 0 = `T0_INT_STS`; W1C |
| Timer 0 config | RO: `PER_INT_CAP`, `INT_ROUTE_CAP=IRQ2`; RW: INT_ENB / TYPE / ROUTE / INT_TYPE |
| Comparator | Store/readback (32-bit) |
| `advance_main_counter` | Host-driven tick; fires when counter crosses comparator |
| `irq_line()` | Device-level request (level follows STS; edge latches until W1C) |

`Machine::advance_hpet` is a thin host helper around `advance_main_counter`.
It does **not** assert PIC or I/O APIC lines.

## Wired

- Device interrupt latch + status bit
- One-shot and periodic (periodic re-arms comparator by period)

## Not wired (explicit)

- Auto-advance from the instruction step clock
- Legacy-replacement routing (`LEG_RT_CNF`)
- Delivery onto 8259 PIC or I/O APIC RTE (see IOAPIC R7 slice)
- MSI / FSB interrupt route register
- Timers 1..N, 64-bit counter mode, ACPI HPET table

## Tests

- `crates/devices/src/hpet.rs` (comparator / oneshot / periodic / gating)
- `crates/machine-pc/tests/hpet_mmio.rs` (`advance_hpet`)
