# HPET → I/O APIC Fixed wire — Milestone 2 Round 10

## Why

R7/R8 raised a device-level `irq_line` on Timer 0 comparator match but left
PIC/IOAPIC unwired. Firmware that programs `Tn_INT_ROUTE_CNF` and an I/O APIC
RTE needs an honest Fixed delivery path without inventing legacy-replacement
or MSI.

## Spec

- IA-PC HPET Specification 1.0a — Timer `Tn_INT_ROUTE_CNF` / `Tn_INT_ROUTE_CAP`
- Intel 82093AA — Fixed RTE delivery on the selected GSI

## Model

| Helper | Behavior |
|---|---|
| `HpetMmio::ioapic_gsi` | Advertised route, else default GSI **2** |
| `Machine::advance_hpet` | Counter + device latch only (no PIC/IOAPIC) |
| `Machine::advance_hpet_ioapic` | Advance then `assert_ioapic_gsi(gsi, irq_line)` |
| `Machine::sync_hpet_irq_to_ioapic` | Sync pin without advancing (post-W1C) |

Level HPET + level RTE sets Remote IRR and LAPIC TMR; EOI via
`Machine::eoi_lapic_ioapic`. DualPic is never mirrored.

## Not wired (explicit)

- Legacy-replacement (`LEG_RT_CNF`) / 8259 path
- FSB / MSI interrupt route register
- Auto-advance from the instruction step clock
- Timers 1..N

## Tests

- `crates/devices/src/hpet.rs` (`ioapic_gsi_defaults_to_irq2_when_route_unset`)
- `crates/machine-pc/tests/hpet_mmio.rs`
  - `machine_hpet_comparator_delivers_fixed_via_ioapic`
  - `machine_hpet_level_ioapic_sets_remote_irr`
