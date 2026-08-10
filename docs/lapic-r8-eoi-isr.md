# Local APIC EOI + ISR/IRR readback — Milestone 2 Round 8

## Why

R7 latched a local vector and cleared a private `in_service` flag on EOI, but
software that probes IRR/ISR MMIO saw zeros. Firmware and early OS bring-up
expect those bitmaps to track accept / EOI.

## Spec

- Intel SDM Vol. 3A Chapter 10
  - §10.8.3 — Interrupt Request Register (IRR) at `200H`–`270H`
  - §10.8.4 — In-Service Register (ISR) at `100H`–`170H`
  - §10.8.5 — EOI register at `B0H` clears the highest-priority ISR bit

## Model

`devices::LocalApicMmio`:

| Event | IRR | ISR |
|---|---|---|
| Timer fire / `inject_fixed` | set vector bit; latch `pending_vector` | unchanged |
| `take_interrupt` | clear bit | set bit; track `in_service` |
| EOI write | unchanged | clear tracked (or highest) ISR bit |

Dword reads of ISR/IRR offsets return the bitmap; writes are claimed and ignored.

## Not wired (explicit)

- Multiple outstanding IRR bits / priority arbitration beyond single pending
- TMR (trigger mode) register, TPR/PPR, ExtINT EOI broadcast to I/O APIC
  (see IOAPIC R8 Remote IRR slice for the complementary stub)
- CPU IDT injection; CPUID `APIC` bit remains clear

## Tests

- `crates/devices/src/lapic.rs` (`irr_isr_readback_and_eoi_clears_isr_bit`,
  `timer_path_sets_irr_then_isr`)
