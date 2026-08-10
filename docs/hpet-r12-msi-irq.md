# HPET MSI / IRQ route honesty — Milestone 2 Round 12

## Why

R10/R11 wire Timer 0 to I/O APIC GSI 2 and test 32-bit wrap + level re-assert.
Some HPET implementations advertise FSB/MSI (`Tn_FSB_INT_DEL_CAP`). Claiming
that without a message route would lie to firmware. This slice locks capability
honesty and deepens advertised-vs-default IRQ route tests.

## Spec

- IA-PC HPET Specification 1.0a
  - General Capabilities `LEG_RT_CAP` (bit 15) — legacy replacement
  - Timer Config `Tn_FSB_EN_CNF` (bit 14) / `Tn_FSB_INT_DEL_CAP` (bit 15)
  - `Tn_INT_ROUTE_CNF` / `Tn_INT_ROUTE_CAP` — I/O APIC IRQ routing

## Model

| Piece | Behavior |
|---|---|
| `Tn_FSB_INT_DEL_CAP` | **Clear** (no MSI/FSB message delivery) |
| `Tn_FSB_EN_CNF` writes | Dropped; never stick |
| `LEG_RT_CAP` | **Clear** (no 8254/RTC replacement) |
| `INT_ROUTE_CAP` | IRQ2 only (unchanged) |
| Unadvertised `Tn_INT_ROUTE_CNF` | Cleared; `ioapic_gsi()` → default GSI 2 |
| Advertised IRQ2 | Sticks; comparator IRQ path unchanged |

No MSI route is implemented because the capability is honestly clear.

## Not wired (explicit)

- FSB/MSI address/value registers / message delivery
- Legacy replacement mapping to IRQ0/IRQ8
- Additional `INT_ROUTE_CAP` IRQs beyond IRQ2
- ACPI HPET table

## Tests

- `crates/devices/src/hpet.rs`
  - `fsb_msi_capability_clear_and_fsb_en_dropped`
  - `irq_route_only_advertised_gsi2_sticks`
