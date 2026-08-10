# LPT parallel-port probe stub — Milestone 2 Round 6

## Why

Post-R5 SeaBIOS POST measurement listed unclaimed LPT probe ports
`0x378` / `0x278` (and nearby `0x3E9` / `0x2E9`). Claiming the classic
three-byte register files stops those bases from appearing as open-bus in
the POST probe.

## Spec

- IBM PC Technical Reference — parallel printer adapter data/status/control.
- [OSDev Wiki — Parallel Port](https://wiki.osdev.org/Parallel_Port):
  - `base+0` Data (R/W)
  - `base+1` Status (R; bit7 Busy **active low** — 1 = not busy)
  - `base+2` Control (R/W)
  - Classic bases LPT1 `0x378`, LPT2 `0x278`

## Model

`devices::ParallelPort` ×2 on `Machine` / `MachineBus`:

| Port | Role | Behavior |
|---|---|---|
| `0x378` / `0x278` | Data | store / readback |
| `0x379` / `0x279` | Status | fixed `LPT_STATUS_NO_PRINTER` (`0xDF`); bit7 = 1 |
| `0x37A` / `0x27A` | Control | store / readback |

Status writes are ignored (input register; IRQ7 clear deferred).

## `0x3E9` / `0x2E9`

These are **not** classic LPT register sites (COM3/COM4 `base+1` IER). They
remain ISA open-bus in this slice; documented so a later UART or probe-note
slice can own them deliberately.

## Unsupported (explicit)

- IRQ7 delivery, ECP/EPP (`base+0x400` / ECR), DMA, bidirectional nibble/byte
  modes beyond plain data R/W, LPT3 at `0x3BC`, actual printer handshake.

## Tests

- `crates/devices/src/lpt.rs` unit tests.
- `crates/machine-pc` unit tests `machine_bus_lpt1_lpt2_claimed_probe_sites_open_bus`.
- `crates/machine-pc/tests/platform_io_r6.rs` probe claim coverage (with LAPIC/HPET/IOAPIC).
