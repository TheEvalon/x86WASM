# Machine model — PC v1

Classic PC subset for firmware and OS bring-up. See ADR `docs/adr/0001-machine-model.md`.

## Memory (Milestone 1 lab)

- Contiguous RAM from physical `0` (default size configurable; CLI default 16 MiB).
- ROM window mapped at `0xFFFF_0000` (64 KiB) so the Intel reset vector at `CS.base + 0xFFF0` = `0xFFFF_FFF0` fetches ROM.
- Optional alias of the same ROM image at `0x000F_0000` for real-mode `F000:xxxx` tooling later.

## Port I/O (M1 + M2 partial)

| Port | Device |
|---|---|
| `0x3F8`–`0x3FF` | COM1 (THR write emits guest serial bytes) |
| `0x402` | Debug console (Bochs/QEMU-style; write = one output byte) |
| `0x70` | CMOS/RTC index (bits 6:0 = register; bit7 = NMI disable latch) |
| `0x71` | CMOS/RTC data |

Unimplemented ports: read `0xFF…`, write ignored (traced when tracing is enabled).

CMOS unit model lives in `devices::CmosRtc` (`crates/devices/src/cmos.rs`). It is **not** yet wired into `machine-pc` / `MachineBus`.

## MMIO

Stub dispatcher in M1; no VGA/IDE BARs yet.

## Interrupts

Not required for HELLO ROM. PIC/APIC deferred to later milestones. CMOS/RTC does **not** raise IRQ8 (no PIE/AIE/UIE delivery) and does not sync to host wall-clock yet.

## Spec / oracle notes

- Serial: 16550-compatible programming model (subset).
- Debug port `0x402`: widely used by SeaBIOS/QEMU guests for early console; treat as write-only byte sink for M1.
- CMOS/RTC: Motorola MC146818 / IBM PC AT — 128-byte register file at `0x70`/`0x71`; NMI bit stored only; status C read-to-clear modeled with sticky-zero flags.
