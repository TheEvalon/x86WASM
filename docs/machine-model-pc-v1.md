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
| `0x00`–`0x0F` | 8237A DMA master (ch0–3) — addr/count/mode/mask stubs |
| `0xC0`–`0xDE` (even) | 8237A DMA slave (ch4–7) — same programming model |
| `0x87`/`0x83`/`0x81`/`0x82` | DMA page registers ch0–3 |
| `0x8F`/`0x8B`/`0x89`/`0x8A` | DMA page registers ch4–7 |
| `0x80` | POST/diagnostic (open-bus read `0xFF`; **not** a DMA page) |
| `0xCF8`–`0xCFB` | PCI `CONFIG_ADDRESS` (Type 1 latch; bits 1:0 hardwired 0) |
| `0xCFC`–`0xCFF` | PCI `CONFIG_DATA` (byte/word/dword lanes) |
| `0x1F0`–`0x1F7` | Primary IDE command block — IDENTIFY (`0xEC`) + READ (`0x20`) + WRITE (`0x30`) SECTORS PIO stub |
| `0x3F6` | Primary IDE alternate status (R) / device control (W; SRST/nIEN) |
| `0x170`–`0x177` | Secondary IDE command block — same PIO stub as primary (`IdeSecondary`) |
| `0x376` | Secondary IDE alternate status (R) / device control (W; SRST/nIEN) |
| `0x3D4` / `0x3D5` | VGA color CRTC index / data — noop register file (indexes `0x00`–`0x18`) |
| `0x3C2` / `0x3CC` | VGA Miscellaneous Output — write `0x3C2`, readback `0x3CC` (store only; reset default `0x67`) |
| `0x3F0`–`0x3F5`, `0x3F7` | 82077AA FDC — SRA/SRB/DOR/TDR/MSR\|DSR/FIFO/DIR\|CCR stub (no media) |

Unimplemented ports: read `0xFF…`, write ignored (traced when tracing is enabled). Unused page holes (`0x84`–`0x86`, `0x88`, `0x8C`–`0x8E`) stay open-bus.

Unit models owned by `machine-pc::Machine` and decoded on `MachineBus`: `devices::DualPic` (`pic.rs`), `devices::Pit8254` (`pit.rs`), `devices::CmosRtc` (`cmos.rs`), `devices::I8042` (`i8042.rs`, field `Machine::kbd`), `devices::Dma8237` (`dma.rs`, field `Machine::dma`), `devices::VgaText` (`vga.rs`, field `Machine::vga`, MMIO + CRTC ports), `devices::PciConfig` (`pci.rs`, field `Machine::pci`), `devices::IdePrimary` (`ide.rs`, field `Machine::ide`), `devices::IdeSecondary` (`ide.rs`, field `Machine::ide_secondary`), `devices::Fdc82077` (`fdc.rs`, field `Machine::fdc`). Reset clears PIC/PIT/CMOS/8042/DMA/VGA/PCI/IDE/FDC like serial (IDE keeps attached image). No snapshot schema for these devices yet (serial has none either).

## MMIO

- VGA color text plane: physical `0xB8000`–`0xBFFFF` (32 KiB) owned by `devices::VgaText` on the CPU data bus (`MachineBus` intercepts before `PhysMem`). Reset fills 80×25 with space + attribute `0x07`. Color CRTC index/data ports `0x3D4`/`0x3D5` accept R/W into a 25-register noop file (no timing/cursor render). Misc Output write `0x3C2` / readback `0x3CC` stores a byte (reset default `0x67`; `0x3C2` read returns `0xFF` open-bus; bits do not yet change IOAS/clock/RAM-enable). **Not yet:** sequencer/GC/ATC, Misc Output side effects, mono CRTC map, protect-bit enforcement, planar graphics, VBE, host rendering.
- IDE: legacy primary + secondary fixed ports (no PCI BARs).

## Interrupts

- Software INT/IRET and real-mode IVT fault delivery: see interpreter / M2 CPU work.
- Hardware IRQ path: `MachineBus::poll_external_irq` syncs PIT ch0 OUT → IRQ0, 8042 OBF∧INT1 → IRQ1, CMOS IRQF → IRQ8, primary IDE INTRQ∧¬nIEN → IRQ14, and secondary IDE → IRQ15 then `DualPic::poll_irq` (INTA-style acknowledge + vector). CPU still delivers via `pending_irq` when `IF=1`.
- 8259A dual PIC: port-wired on `MachineBus`; ICW1–ICW4, OCW1 IMR, OCW2 non-specific/specific EOI, OCW3 IRR/ISR read select, edge IR assert, fully nested priority, cascade on IR2. **Not yet:** Auto-EOI, rotate, special mask, OCW3 poll command, level-triggered runtime.
- 8254 PIT: port-wired on `MachineBus`; channel-0 programming + `ce`/OUT tick (`Pit8254::tick_ch0`, `Machine::tick_pit` also ticks ch2). Modes 0/2/3 OUT rising edges drive IRQ0 (8259A master IR0). Ch0/ch1 GATE assumed high. Channel 2 GATE + speaker-data latch + OUT2 readback via port `0x61` bits 0/1/5 (no host audio). Guest wall-clock rate is **not** host-real-time (explicit tick quantum). **Not yet:** host speaker audio, ch1 DRAM refresh, modes 1/4/5 OUT claims, mode 3 exact 50% duty, port `0x61` NMI/parity/refresh bits.
- CMOS/RTC: port-wired on `MachineBus`; status B PIE/AIE/UIE + status C PF/AF/UF/IRQF (read-to-clear); `CmosRtc::tick` / `Machine::tick_cmos` → IRQ8 (8259A slave IR0); Status A UIP + `CmosRtc::tick_second` / `Machine::tick_cmos_second` full BCD calendar update cycle (SET inhibits); port `0x70` bit7 NMI mask stored (`CmosRtc::nmi_masked`) / `Machine::nmi_delivery_enabled`; `Machine::inject_nmi` latches CPU `#NMI` (IVT vector 2) when unmasked, drops when masked. Guest period/second quantum is **not** host-real-time.
  - Time/calendar register map: `0x00` sec, `0x01` sec alarm, `0x02` min, `0x03` min alarm, `0x04` hour, `0x05` hour alarm, `0x06` day of week (1 = Sunday … 7 = Saturday), `0x07` date of month, `0x08` month, `0x09` year, `0x0A`–`0x0D` status A–D, `0x32` century (PC/AT + ACPI FADT convention, not part of the MC146818 file).
  - Update-cycle cascade (each `tick_second` when status B SET=0): sec `0x59`→`0x00` → min `0x59`→`0x00` → hour `0x23`→`0x00` → date of month per actual month length (February 29 in Gregorian leap years computed from century+year) → month `0x12`→`0x01` → year `0x99`→`0x00` → century `0x32`. Day of week advances 1–7 (wrapping 7→1) on every date rollover, independent of the month/date arithmetic.
  - Reset state: all 128 bytes zero except status A `0x26`, status B `0x02` (BCD, 24-hour), status C `0x00`, status D `0x80` — i.e. time/calendar/century zeros, which are **not** a valid date. The cascade is total for such states: an unrecognized month uses a 31-day fallback length, a date past that length wraps to `0x01` and steps the month, an unrecognized month steps to January without a year carry, and an unrecognized day of week resets to 1. No host time is sampled; guests (SeaBIOS) program a valid date first.
  - **Not yet:** host wall-clock/NTP sync, SMRAM/SMI / NMI nesting, exact crystal UIP pulse width, binary (status B `DM`) data mode and 12-hour mode conversions (BCD/24-hour only), square-wave output, ACPI extended CMOS beyond 128 bytes.
- 8042 / PS/2: port-wired on `MachineBus` (`0x60`/`0x64`); self-test / config / enable-disable; OBF + config bit0 (INT1) → IRQ1 via `Machine::kbd_place_output` / `poll_external_irq`; make-code inject stub (`I8042::inject_scancode` / `Machine::kbd_inject_scancode`) → OBF when keyboard clock enabled (dropped when config bit4 set); config bit6 translation stored but **not** remapped (passthrough Set1 bytes); output-port `0xD0`/`0xD1` bit1 → `PhysMem` A20. **Not yet:** IRQ12/mouse, full AT keyboard protocol / Set2↔Set1 table, pulse-reset (`0xFE` / output-port bit0).
- APIC deferred to later milestones.

## Spec / oracle notes

- Serial: 16550-compatible programming model (subset).
- Debug port `0x402`: widely used by SeaBIOS/QEMU guests for early console; treat as write-only byte sink for M1.
- 8259A: Intel 8259A Programmable Interrupt Controller datasheet (ICW1–ICW4, OCW1–OCW3 EOI/IMR/IRR/ISR); classic PC cascade on IRQ2.
- 8254: Intel 8254 PIT datasheet — channel 0/2 control word, lo/hi access, latch, modes 0/2/3 OUT + GATE; ch0 OUT → IRQ0 via `Machine::tick_pit` / `poll_external_irq` level follow; ch2 GATE/OUT via port `0x61`. No host audio / DRAM-refresh / host-real-time claims.
- CMOS/RTC: Motorola MC146818A Real Time Clock Plus RAM datasheet / IBM PC AT — 128-byte register file at `0x70`/`0x71`; index bit7 NMI mask R/W + `nmi_masked` / `Machine::nmi_delivery_enabled` + `inject_nmi` → interpreter `#NMI` vector 2; status A UIP (read-only to guest); status B PIE/AIE/UIE/SET; status C PF/AF/UF/IRQF read-to-clear; "Time, Calendar, and Alarm Locations" + update-cycle increment with automatic leap-year compensation (BCD/24-hour only); century byte `0x32` per the PC/AT + ACPI FADT `CENTURY` index convention; periodic tick; IRQ → ISA IRQ8.
- 8042: OSDev I8042 PS/2 Controller + IBM PC AT 8042 programming model — port-wired; config INT1 + OBF → ISA IRQ1; make-code inject → OBF (clock-disable respected; translation passthrough); output-port bit1 → A20; see `devices::I8042` / `Machine::kbd`.
- 8237A DMA: Intel 8237A + OSDev ISA DMA — dual controllers + AT page regs on `MachineBus`; programming accepted (flip-flop/addr/count/mode/mask); **no** memory transfer engine / DREQ/DACK/TC. Port `0x80` remains POST.
- VGA text: IBM VGA / OSDev Text UI / OSDev VGA Hardware / FreeVGA — `0xB8000` char+attr plane + CRTC `0x3D4`/`0x3D5` noop + Misc Output `0x3C2`/`0x3CC` store/readback on `MachineBus`; **not** sequencer/GC/ATC/Misc side effects/timing/render.
- PCI config: PCI Local Bus Mechanism #1 + OSDev PCI — `0xCF8`/`0xCFC` on `MachineBus`; host bridge `00:00.0` identity `8086:1237` (i440FX-class stub), class host bridge, header type 0; PIIX-style stubs at `00:01.0` ISA bridge `8086:7000` (multi-function, class `0x0601`), `00:01.1` IDE `8086:7010` (class `0x0101`, prog IF `0x80`), `00:01.2` USB UHCI `8086:7020` (class `0x0C03`, prog IF `0x00`), and `00:01.3` ACPI `8086:7113` (class `0x0680`); absent devices and enable-bit-clear data reads return `0xFFFFFFFF`. **Not yet:** USB host controller / ACPI PM I/O / SMI / tables, BAR MMIO decode, bus mastering, caps/MSI/PCIe.
- Primary IDE: ATA/ATAPI + OSDev ATA PIO — `IdePrimary` on `0x1F0`–`0x1F7`/`0x3F6`; IDENTIFY DEVICE + READ/WRITE SECTORS (LBA28) PIO; IDENTIFY PACKET DEVICE (`0xA1`) → ERR+ABRT on ATA master (SeaBIOS ATAPI probe reject; master stays ATA); DRQ/BSY/DRDY/ERR; backing `Vec<u8>` image; IRQ14 via `irq_line()` → `DualPic` when nIEN=0 (status read clears; alt status does not).
- Secondary IDE: `IdeSecondary` thin remap of the same PIO stub on `0x170`–`0x177`/`0x376`; IRQ15 → `DualPic` when nIEN=0. **Not yet:** ATAPI PACKET/`0xA0` media, slave, DMA, LBA48, PCI BAR remap, SeaBIOS boot.
- FDC: Intel 82077AA + OSDev FDC — `Fdc82077` on `0x3F0`–`0x3F5`/`0x3F7` (not `0x3F6`); DOR/MSR/FIFO/DIR/CCR accept; MSR RQM when DOR nRESET set; Specify (`0x03`) → two parameter bytes (SRT|HUT, HLT|ND) stored, no result/IRQ; Recalibrate (`0x07`) → one unit-select param, `pcn=0`, latches ST0 Seek End (`0x20|US`) and asserts IRQ; Seek (`0x0F`) → HD|US + NCN, `pcn=NCN`, latches ST0 Seek End (`0x20|US`; H in ST0 always 0) and asserts IRQ; Sense Interrupt Status (`0x08`) → result ST0+PCN with MSR RQM|DIO and IRQ clear (uses Recalibrate/Seek ST0 latch when present, else post-reset/`assert_irq6` stub `0xC0|DOR[1:0]`); Sense Drive Status (`0x04`) → one HD|US param, no execution phase, result ST3 (T0 stub set when shared `pcn==0`, WP stub always 0, HD/US echoed from param, reserved bits 3/5 always 1), no IRQ; Version (`0x10`) → no params, 1-byte result `0x90` (82077AA id) with MSR RQM|DIO, no IRQ; Configure (`0x13`) → three params stored (`configure_byte0`, `configure_eis_fifo_poll_thr`, `configure_pretrk`), no result/IRQ (bit side effects not enforced); `assert_irq6`/`clear_irq6` → `irq_line` (DOR nRESET∧DMA/IRQ bit3) → `MachineBus::poll_external_irq` → DualPic IRQ6 (vector `0x0E` with default ICW2 base `0x08`). **Not yet:** READ/WRITE/LOCK/PERPENDICULAR/DUMPREG, Configure FIFO/EIS/POLL enforcement, media image, DMA ch2 transfers, seek step timing, disk-change timing.
