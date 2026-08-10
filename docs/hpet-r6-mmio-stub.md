# HPET MMIO capability stub — Milestone 2 Round 6

## Why

Post-R5 SeaBIOS POST measured unmapped MMIO at `0xFED00000` (HPET). Claiming
the register window stops those probes from appearing as unmapped open-bus.

## Spec

- IA-PC HPET (High Precision Event Timers) Specification, Revision 1.0a
  - General Capabilities and ID @ `00h` (64-bit RO)
  - General Configuration @ `10h` (`ENABLE_CNF` bit 0)
  - Main Counter @ `F0h`

## Model

`devices::HpetMmio` on `Machine` / `MachineBus` (base `0xFED0_0000`, 1 KiB):

| Offset | Register | Behavior |
|---|---|---|
| `00h` | CAPS/ID | RO: rev `0x01`, `NUM_TIM_CAP=0`, vendor `0x8086`, period `69841279` fs |
| `10h` | Config | store/readback of `ENABLE_CNF` only |
| `F0h` | Main counter | always `0` (no freerun; honesty note) |

`COUNT_SIZE_CAP` is clear (32-bit counter advertised; still stuck at zero).

## Unsupported (explicit)

- Comparator timers / IRQ / MSI
- Legacy-replacement routing
- Freerunning counter advance from the step clock
- ACPI HPET table / FADT mapping

## Tests

- `crates/devices/src/hpet.rs`
- `crates/machine-pc/tests/hpet_mmio.rs`
- `machine_bus_hpet_caps_and_probe_claim` in `machine-pc` lib tests
