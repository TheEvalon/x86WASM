# I/O APIC delivery-mode honesty — Milestone 2 Round 10

## Why

R7 delivered Fixed RTEs and silently dropped every other delivery mode. Firmware
that programs SMI/NMI/ExtINT/LowestPriority needs an explicit unsupported record
rather than an invented APIC-bus implementation.

## Spec

- Intel 82093AA I/O APIC — RTE delivery mode bits 10:8
  - `000` Fixed, `001` Lowest Priority, `010` SMI, `100` NMI, `101` INIT,
    `111` ExtINT

## Model

`devices::IoApicMmio`:

| Mode | RTE store/readback | `assert_pin` |
|---|---|---|
| Fixed (`000`) | Yes | Delivers [`IoApicDelivery`] |
| Lowest / SMI / NMI / INIT / ExtINT | Yes (probe honesty) | `None` + [`IoApicUnsupportedDelivery`] |

Helpers: `ioapic_delivery_mode_supported`, `delivery_mode`,
`unsupported_delivery` / `take_unsupported_delivery`.

## Not wired (explicit)

- Full APIC bus / ICR messaging
- ExtINT → DualPic virtual-wire
- Logical destination / IR format
- CPU IDT injection

## Tests

- `crates/devices/src/ioapic.rs`
  - `non_fixed_delivery_modes_unsupported_fixed_still_works`
