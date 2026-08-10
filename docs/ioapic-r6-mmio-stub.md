# I/O APIC MMIO stub — Milestone 2 Round 6

## Why

SeaBIOS / firmware commonly probes `0xFEC00000` next to Local APIC / HPET.
Claiming the I/O APIC window and wiring all Round-6 platform devices completes
the platform-io ownership area.

## Spec

- Intel 82093AA I/O Advanced Programmable Interrupt Controller datasheet
  - `IOREGSEL` @ `00h`, `IOWIN` @ `10h`
  - Indirect `IOAPICID` / `IOAPICVER` / `IOAPICARB`
  - Redirection table from index `10h` (two dwords per IRQ; 24 entries typical)

## Model

`devices::IoApicMmio` on `Machine` / `MachineBus` (base `0xFEC0_0000`, 4 KiB):

| Path | Behavior |
|---|---|
| IOREGSEL | store/readback index |
| IOAPICID | APIC ID bits 27:24 store/readback |
| IOAPICVER | RO `0x00170011` (version `0x11`, MaxREDTBL `0x17` → 24 entries) |
| IOAPICARB | mirrors ID in this stub |
| REDTBL[0..23] | low/high dword store/readback; **no delivery** |

## Wire-all (this slice)

`MachineBus` routes:

- LPT1/LPT2 ports `0x378`–`0x37A` / `0x278`–`0x27A`
- Local APIC MMIO `0xFEE00000`
- HPET MMIO `0xFED00000`
- I/O APIC MMIO `0xFEC00000`

## Unsupported (explicit)

- IRQ remapping / delivery from IOAPIC to Local APIC / CPU
- EOI / remote IRR side effects
- Mask/trigger polarity beyond store/readback

## Tests

- `crates/devices/src/ioapic.rs`
- `crates/machine-pc/tests/ioapic_mmio.rs`
- `machine_bus_ioapic_and_platform_wire_all` in `machine-pc` lib tests
