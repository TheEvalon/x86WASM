# I/O APIC Remote IRR / EOI — Milestone 2 Round 8

## Why

R7 delivered level-triggered Fixed RTEs on every `assert_pin(..., true)` while
the pin stayed high, with no Remote IRR. Real 82093AA hardware sets Remote IRR
on level delivery and suppresses further issues until EOI.

## Spec

- Intel 82093AA I/O APIC
  - Redirection-table Remote IRR (bit 14 of low dword) — RO for software
  - Level trigger + Fixed delivery: set Remote IRR on issue; clear on EOI
  - Edge trigger: Remote IRR remains clear

## Model

`devices::IoApicMmio`:

| Event | Behavior |
|---|---|
| Level Fixed `assert_pin` success | Set Remote IRR; return delivery |
| Level pin still high + Remote IRR | No delivery |
| `eoi(vector)` | Clear Remote IRR on matching level RTEs |
| Software RTE write | Preserves hardware Remote IRR bit |

`machine-pc` helper `eoi_lapic_ioapic` / `ioapic_wire::eoi_lapic_and_ioapic`
clears Local APIC ISR then broadcasts the vector to the I/O APIC.

## Not wired (explicit)

- Directed EOI / APIC ID matching beyond vector match
- ExtINT / NMI / SMI / Lowest Priority
- Automatic device→GSI routing; CPU IDT injection

## Tests

- `crates/devices/src/ioapic.rs` (`level_rte_sets_remote_irr_until_eoi`,
  `edge_rte_never_sets_remote_irr`)
