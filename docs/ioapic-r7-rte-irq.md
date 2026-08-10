# I/O APIC RTE → IRQ path — Milestone 2 Round 7

## Why

Round-6 stored/read back the 24-entry redirection table with no delivery.
Firmware that programs RTEs needs a guest-visible interrupt path.

## Spec

- Intel 82093AA I/O APIC
  - Redirection-table entry: vector, delivery mode, mask, trigger, destination
  - Reset: mask bit set on each RTE
  - Edge vs level pin semantics

## Model

`devices::IoApicMmio::assert_pin(gsi, high)` → optional [`IoApicDelivery`]
(Fixed mode only). `Machine::assert_ioapic_gsi` latches the vector on
`LocalApicMmio` when `dest_apic_id == lapic.apic_id()` and SVR software-enable
is set.

## Coordination with DualPic

| Path | Behavior |
|---|---|
| ISA devices (PIT/kbd/…) | Still `DualPic::set_irq_line` only |
| I/O APIC GSI pins | Separate; **no** automatic PIC mirror |
| ExtINT / virtual-wire | Not modeled |

## Wired

- Unmasked Fixed RTE → Local APIC `inject_fixed` / `take_interrupt`
- Edge rising-edge; level while-high (no remote-IRR suppress yet)
- RTE mask-at-reset

## Not wired (explicit)

- NMI / SMI / ExtINT / Lowest Priority delivery modes
- Logical destination mode / IR format
- EOI broadcast / remote IRR
- CPU IDT injection (same honesty as LAPIC R7)
- Automatic device→GSI routing (HPET route, PCI INTx→GSI)

## Tests

- `crates/devices/src/ioapic.rs`
- `crates/machine-pc/tests/ioapic_mmio.rs`
