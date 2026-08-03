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
| `0x20` / `0x21` | 8259A master PIC (command / data) — ICW + OCW1/EOI/IRR/ISR |
| `0xA0` / `0xA1` | 8259A slave PIC (command / data) — ICW + OCW1/EOI/IRR/ISR |
| `0x40` | 8254 PIT channel 0 data — **programming only** |
| `0x41` | 8254 PIT channel 1 data — stub accept (not fully supported) |
| `0x42` | 8254 PIT channel 2 data — stub accept (not fully supported) |
| `0x43` | 8254 PIT control word |
| `0x70` | CMOS/RTC index (bits 6:0 = register; bit7 = NMI disable latch) |
| `0x71` | CMOS/RTC data |

Unimplemented ports: read `0xFF…`, write ignored (traced when tracing is enabled).

Unit models owned by `machine-pc::Machine` and decoded on `MachineBus`: `devices::DualPic` (`pic.rs`), `devices::Pit8254` (`pit.rs`), `devices::CmosRtc` (`cmos.rs`). Reset clears PIC/PIT/CMOS like serial. No snapshot schema for these devices yet (serial has none either).

### Unit models not yet on MachineBus

| Port | Device | Notes |
|---|---|---|
| `0x60` / `0x64` | 8042 / PS/2 controller (`devices::I8042`, `i8042.rs`) | **Unit model only** — self-test `0xAA`→`0x55`, config byte `0x20`/`0x60`, disable/enable `0xAD`/`0xAE`, status OBF/IBF. **Not** decoded on `MachineBus` yet. No IRQ1, mouse, or A20 side effects. |

## MMIO

Stub dispatcher in M1; no VGA/IDE BARs yet.

## Interrupts

- Software INT/IRET and real-mode IVT fault delivery: see interpreter / M2 CPU work.
- Hardware IRQ path: `MachineBus::poll_external_irq` → `DualPic::poll_irq` (INTA-style acknowledge + vector). CPU still delivers via `pending_irq` when `IF=1`.
- 8259A dual PIC: port-wired on `MachineBus`; ICW1–ICW4, OCW1 IMR, OCW2 non-specific/specific EOI, OCW3 IRR/ISR read select, edge IR assert, fully nested priority, cascade on IR2. **Not yet:** Auto-EOI, rotate, special mask, OCW3 poll command, level-triggered runtime, PIT/CMOS→PIC wiring.
- 8254 PIT: port-wired on `MachineBus`; channel-0 programming only; does **not** raise IRQ0 (no gate/OUT→PIC wiring).
- CMOS/RTC: port-wired on `MachineBus`; index/data bank only; does **not** raise IRQ8 (no PIE/AIE/UIE delivery) and does not sync to host wall-clock yet.
- APIC deferred to later milestones.

## Spec / oracle notes

- Serial: 16550-compatible programming model (subset).
- Debug port `0x402`: widely used by SeaBIOS/QEMU guests for early console; treat as write-only byte sink for M1.
- 8259A: Intel 8259A Programmable Interrupt Controller datasheet (ICW1–ICW4, OCW1–OCW3 EOI/IMR/IRR/ISR); classic PC cascade on IRQ2.
- 8254: Intel 8254 PIT datasheet — channel 0 control word, lo/hi access, latch; no IRQ0 pulse / speaker / DRAM-refresh claims yet.
- CMOS/RTC: Motorola MC146818 / IBM PC AT — 128-byte register file at `0x70`/`0x71`; NMI bit stored only; status C read-to-clear modeled with sticky-zero flags.
- 8042: OSDev I8042 PS/2 Controller + IBM PC AT 8042 programming model — unit stub only; see `devices::I8042`.