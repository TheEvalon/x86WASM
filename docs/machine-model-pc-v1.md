# Machine model — PC v1

Classic PC subset for firmware and OS bring-up. See ADR `docs/adr/0001-machine-model.md`.

## Memory (Milestone 1 lab)

- Contiguous RAM from physical `0` (default size configurable; CLI default 16 MiB).
- ROM window mapped at `0xFFFF_0000` (64 KiB) so the Intel reset vector at `CS.base + 0xFFF0` = `0xFFFF_FFF0` fetches ROM.
- Optional alias of the same ROM image at `0x000F_0000` for real-mode `F000:xxxx` tooling later.
- A20 gate on `PhysMem`: when disabled, physical address bit 20 is forced clear on CPU bus read/write. Controlled by 8042 output-port bit1 (`0xD1` write / `0xD0` read). Reset default: A20 enabled.

## Port I/O (M1 + M2 partial)

| Port | Device |
|---|---|
| `0x3F8`–`0x3FF` | COM1 (THR write emits guest serial bytes) |
| `0x402` | Debug console (Bochs/QEMU-style; write = one output byte) |
| `0x20` / `0x21` | 8259A master PIC (command / data) — ICW + OCW1/EOI/IRR/ISR |
| `0xA0` / `0xA1` | 8259A slave PIC (command / data) — ICW + OCW1/EOI/IRR/ISR |
| `0x40` | 8254 PIT channel 0 data — programming + CE/OUT tick (IRQ0) |
| `0x41` | 8254 PIT channel 1 data — stub accept (not fully supported) |
| `0x42` | 8254 PIT channel 2 data — speaker timer (GATE via `0x61`) |
| `0x43` | 8254 PIT control word |
| `0x61` | System control port B subset — bit0 GATE2, bit1 speaker data, bit5 OUT2 (read) |
| `0x70` | CMOS/RTC index (bits 6:0 = register; bit7 = NMI disable latch) |
| `0x71` | CMOS/RTC data |
| `0x60` | 8042 / PS/2 data — OBF∧config INT1 → IRQ1 |
| `0x64` | 8042 / PS/2 status (read) / command (write) |

Unimplemented ports: read `0xFF…`, write ignored (traced when tracing is enabled).

Unit models owned by `machine-pc::Machine` and decoded on `MachineBus`: `devices::DualPic` (`pic.rs`), `devices::Pit8254` (`pit.rs`), `devices::CmosRtc` (`cmos.rs`), `devices::I8042` (`i8042.rs`, field `Machine::kbd`). Reset clears PIC/PIT/CMOS/8042 like serial. No snapshot schema for these devices yet (serial has none either).

## MMIO

Stub dispatcher in M1; no VGA/IDE BARs yet.

## Interrupts

- Software INT/IRET and real-mode IVT fault delivery: see interpreter / M2 CPU work.
- Hardware IRQ path: `MachineBus::poll_external_irq` syncs PIT ch0 OUT → IRQ0, 8042 OBF∧INT1 → IRQ1, and CMOS IRQF → IRQ8 then `DualPic::poll_irq` (INTA-style acknowledge + vector). CPU still delivers via `pending_irq` when `IF=1`.
- 8259A dual PIC: port-wired on `MachineBus`; ICW1–ICW4, OCW1 IMR, OCW2 non-specific/specific EOI, OCW3 IRR/ISR read select, edge IR assert, fully nested priority, cascade on IR2. **Not yet:** Auto-EOI, rotate, special mask, OCW3 poll command, level-triggered runtime.
- 8254 PIT: port-wired on `MachineBus`; channel-0 programming + `ce`/OUT tick (`Pit8254::tick_ch0`, `Machine::tick_pit` also ticks ch2). Modes 0/2/3 OUT rising edges drive IRQ0 (8259A master IR0). Ch0/ch1 GATE assumed high. Channel 2 GATE + speaker-data latch + OUT2 readback via port `0x61` bits 0/1/5 (no host audio). Guest wall-clock rate is **not** host-real-time (explicit tick quantum). **Not yet:** host speaker audio, ch1 DRAM refresh, modes 1/4/5 OUT claims, mode 3 exact 50% duty, port `0x61` NMI/parity/refresh bits.
- CMOS/RTC: port-wired on `MachineBus`; status B PIE/AIE/UIE + status C PF/AF/UF/IRQF (read-to-clear); `CmosRtc::tick` / `Machine::tick_cmos` → IRQ8 (8259A slave IR0). Guest period quantum is **not** host-real-time. **Not yet:** wall-clock sync, NMI delivery, UIP/crystal model.
- 8042 / PS/2: port-wired on `MachineBus` (`0x60`/`0x64`); self-test / config / enable-disable; OBF + config bit0 (INT1) → IRQ1 via `Machine::kbd_place_output` / `poll_external_irq`; make-code inject stub (`I8042::inject_scancode` / `Machine::kbd_inject_scancode`) → OBF when keyboard clock enabled (dropped when config bit4 set); config bit6 translation stored but **not** remapped (passthrough Set1 bytes); output-port `0xD0`/`0xD1` bit1 → `PhysMem` A20. **Not yet:** IRQ12/mouse, full AT keyboard protocol / Set2↔Set1 table, pulse-reset (`0xFE` / output-port bit0).
- APIC deferred to later milestones.

## Spec / oracle notes

- Serial: 16550-compatible programming model (subset).
- Debug port `0x402`: widely used by SeaBIOS/QEMU guests for early console; treat as write-only byte sink for M1.
- 8259A: Intel 8259A Programmable Interrupt Controller datasheet (ICW1–ICW4, OCW1–OCW3 EOI/IMR/IRR/ISR); classic PC cascade on IRQ2.
- 8254: Intel 8254 PIT datasheet — channel 0/2 control word, lo/hi access, latch, modes 0/2/3 OUT + GATE; ch0 OUT → IRQ0 via `Machine::tick_pit` / `poll_external_irq` level follow; ch2 GATE/OUT via port `0x61`. No host audio / DRAM-refresh / host-real-time claims.
- CMOS/RTC: Motorola MC146818 / IBM PC AT — 128-byte register file at `0x70`/`0x71`; NMI bit stored only; status B PIE/AIE/UIE; status C PF/AF/UF/IRQF read-to-clear; IRQ → ISA IRQ8.
- 8042: OSDev I8042 PS/2 Controller + IBM PC AT 8042 programming model — port-wired; config INT1 + OBF → ISA IRQ1; make-code inject → OBF (clock-disable respected; translation passthrough); output-port bit1 → A20; see `devices::I8042` / `Machine::kbd`.
