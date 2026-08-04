# Approved sources

Authoritative references for implementation. Agents must cite these (or Intel SDM sections) instead of inventing behavior.

## CPU and architecture

- Intel 64 and IA-32 Architectures Software Developer's Manual (SDM), Volumes 1–4
- Intel SDM Vol. 2 — LGDT/SGDT (`0F 01 /2`, `/0`); LIDT/SIDT (`0F 01 /3`, `/1`); SMSW/LMSW (`0F 01 /4`, `/6`); m16&32 pseudo-descriptor; LGDT/LIDT register form `#UD`
- Intel SDM Vol. 3 §2.4.1 — GDTR base/limit
- Intel SDM Vol. 3 §2.4.3 — IDTR base/limit
- Intel SDM Vol. 3 §2.5 — CR0 / machine status word (PE sticky under LMSW)
- Intel SDM Vol. 2 — MOV to/from Control Registers (`0F 20`/`0F 22` `/r`); mod field of ModR/M architecturally ignored (register-direct only, no SIB/displacement); CR1/CR5/CR6/CR7 reference → `#UD`; MOV to CR0 (unlike LMSW) may clear PE; MOV to CR zeros CR upper 32 bits outside 64-bit mode
- Intel processor datasheets relevant to Core 2 Conroe/Penryn CPUID presentation

## Platform and devices

- PCI Local Bus Specification — Configuration Mechanism #1 (`CONFIG_ADDRESS` `0xCF8`, `CONFIG_DATA` `0xCFC`, Type 1 address, enable bit 31)
- OSDev Wiki PCI — configuration space access mechanism #1; absent device → `0xFFFFFFFF`
- Intel 440FX (i440FX) host-bridge identity (`vendor 0x8086`, `device 0x1237`) as classic QEMU/SeaBIOS-compatible stub ID (behavior from specs/oracles, not copied source)
- Intel 82371SB (PIIX3) public PCI IDs — ISA bridge `8086:7000`, IDE `8086:7010`, USB UHCI `8086:7020` (config-space identity stubs only; no copied implementation)
- Intel 82371AB (PIIX4) ACPI function public ID `8086:7113` (classic pc `00:01.3` config-space identity stub; class `0x0680`; no PM I/O / SMI)
- ACPI specifications (for later machine/firmware work)
- ATA / ATAPI specifications (IDENTIFY DEVICE `0xEC`, IDENTIFY PACKET DEVICE `0xA1`, READ SECTORS `0x20`, WRITE SECTORS `0x30`, task-file / status bits, error ABRT, LBA28 PIO, device-control nIEN / INTRQ)
- OSDev Wiki ATA PIO Mode — primary ports `0x1F0`–`0x1F7` / `0x3F6` (IRQ14), secondary `0x170`–`0x177` / `0x376` (IRQ15), IDENTIFY/READ/WRITE polling (status clears IRQ; alt status does not); ATAPI probe via `0xA1`
- Intel 8259A / 8259A-2 / 8259A-8 Programmable Interrupt Controller datasheet (ICW1–ICW4 incl. ICW4 bit1 Automatic EOI / AEOI — ISR bit cleared at end of interrupt-acknowledge sequence; OCW; Operation Command Word OCW3 format — `ESMM`/`SMM` Special Mask Mode + Poll Command `P=1` acknowledging command-port read returning bit7 pending + binary level; Special Mask Mode: masked in-service level does not inhibit other unmasked levels; non-specific EOI skips IMR-masked IS bits while SMM active; ICW1 clears Special Mask Mode)
- Intel 8254 Programmable Interval Timer datasheet — control word (SC/RW/M/BCD), counter latch, Read-Back command (`SC=11`; COUNT/STATUS active-low latch bits + CNT2/CNT1/CNT0 select; status byte OUT/NULL COUNT/RW/M/BCD; status-then-count read order when both latched), LSB/MSB access; "Mode Definitions" mode 0 (interrupt on terminal count), mode 1 (hardware retriggerable one-shot), mode 2 (rate generator), mode 3 (square wave), mode 4 (software triggered strobe), mode 5 (hardware triggered strobe); GATE-pin operations summary table (GATE low disables counting in modes 0/2/3/4 and forces OUT high in modes 2/3; GATE rising edge triggers modes 1/2/3/5)
- Motorola MC146818A Real Time Clock Plus RAM datasheet / IBM PC AT CMOS RTC register map (ports `0x70`/`0x71`; index bit7 = NMI disable; "Time, Calendar, and Alarm Locations" `0x00`–`0x09` incl. day-of-week 1–7 with 1 = Sunday; update-cycle time/calendar increment with automatic leap-year compensation; status A UIP/divider/RS; status B SET/DM/24-12/PIE/AIE/UIE; status C PF/AF/UF/IRQF)
- IBM PC/AT CMOS BCD century byte at index `0x32`, later standardized as the ACPI FADT `CENTURY` index field (century is **not** part of the MC146818 register file)
- PS/2 and 8042 controller references — OSDev Wiki "I8042 PS/2 Controller" (<https://wiki.osdev.org/I8042_PS/2_Controller>) + IBM PS/2 keyboard-controller programming model: data `0x60` / status-command `0x64`; status bit0 OBF, bit1 IBF, bit2 system flag, bit3 command/data, bit5 AUX OBF (PS/2; transmit/receive timeout on the original AT); command byte (config) bit0 first-port interrupt (IRQ1), bit1 second-port interrupt (IRQ12), bit4 first-port clock disable, bit5 second-port clock disable, bit6 translation; controller commands `0x20`/`0x60` read/write command byte, `0xAA` self-test → `0x55`, `0xAD`/`0xAE` disable/enable first port, `0xA7`/`0xA8` disable/enable second (auxiliary) port, `0xA9` test second port → `0x00` = no error, `0xD0`/`0xD1` read/write output port (bit1 = A20), `0xD4` write next data byte to the auxiliary device
- Intel SDM Vol. 3 §6.3.3 / §6.7 / §6.15 — `#NMI` (interrupt vector 2; not maskable by `IF`)
- Intel 8237A Programmable DMA Controller datasheet (addr/count/mode/mask/flip-flop; status register bits 3:0 terminal count per channel, cleared by status read or master clear; bits 7:4 request pending — DREQ path not modeled)
- OSDev Wiki ISA DMA — AT port map and page registers (not port `0x80`)
- IBM VGA / classic PC color text frame buffer at physical `0xB8000` (80×25, char+attr)
- OSDev Text UI — VGA text-mode memory layout
- OSDev VGA Hardware / FreeVGA CRT Controller — color CRTC Address `0x3D4`, Data `0x3D5`, indexes `0x00`–`0x18`
- OSDev VGA Hardware / FreeVGA Sequencer Registers — Address `0x3C4`, Data `0x3C5`, indexes `0x00`–`0x04` (Reset, Clocking Mode, Map Mask, Character Map Select, Memory Mode); mode-03h-class defaults `03/00/03/00/02`
- OSDev VGA Hardware / FreeVGA Graphics Registers — Address `0x3CE`, Data `0x3CF`, indexes `0x00`–`0x08` (Set/Reset … Bit Mask); mode-03h-class defaults `00/00/00/00/00/10/0E/00/FF`
- OSDev VGA Hardware / FreeVGA Attribute Controller Registers + Accessing the Attribute Registers — Address/Data `0x3C0` (index/data flip-flop; bit5 PAS), Data Read `0x3C1`; indexes `0x00`–`0x14` (palette `0x00`–`0x0F`, Mode Control `0x10`, Overscan `0x11`, Color Plane Enable `0x12`, Horizontal PEL Panning `0x13`, Color Select `0x14`); read Input Status #1 `0x3DA` (color) resets flip-flop to address; mode-03h-class defaults palette `00/01/02/03/04/05/14/07/38/39/3A/3B/3C/3D/3E/3F` + `0C/00/0F/08/00`
- OSDev VGA Hardware / FreeVGA Miscellaneous Output Register — write `0x3C2`, readback `0x3CC` (write-only at `0x3C2`; common BIOS text-mode value `0x67`)
- Intel 82077AA CHMOS Single-Chip Floppy Disk Controller — DOR/MSR/FIFO/DIR/CCR; Specify (`0x03`) two params (SRT|HUT, HLT|ND), no result/IRQ; Recalibrate (`0x07`) one unit-select param, PCN=0, ST0 Seek End (`0x20|US`), IRQ; Seek (`0x0F`) HD|US + NCN, PCN=NCN, ST0 Seek End (`0x20|US`, H in ST0 always 0), IRQ; Sense Interrupt Status (`0x08`) ST0+PCN (command ST0 latch or post-reset `0xC0|US`); Sense Drive Status (`0x04`) HD|US → ST3; Version (`0x10`) no params, result `0x90` (82077AA id), no IRQ; Configure (`0x13`) three params (unused, EIS|FIFO_DIS|POLL_DIS|FIFOTHR, PRETRK), no result/IRQ; LOCK (`0x14`/`0x94`, §5.3.2) LOCK in command bit7, no params, result `LOCK<<4`, no IRQ; soft DOR/DSR reset does not clear LOCK and preserves Configure EFIFO/FIFOTHR/PRETRK when LOCK=1; hardware reset clears LOCK; DUMPREG (`0x0E`, §5.2.10 / Table 5-1 / §5.3.3) no params, 10-byte result (PCN0–3, SRT|HUT, HLT|ND, SC/EOT, LOCK|0|D3–D0|GAP|WGATE, 0|EIS|EFIFO|POLL|FIFOTHR, PRETRK), no IRQ; DOR bit3 DMA/IRQ enable; IRQ on command complete
- OSDev Wiki Floppy Disk Controller — ports `0x3F0`–`0x3F7` excluding `0x3F6` (IDE); MSR RQM/DIO; Specify timing; Recalibrate/Seek → IRQ then Sense Interrupt; Sense Interrupt clears IRQ; Version returns `0x90` for 82077AA-class; Configure stores EIS/FIFO/POLL/FIFOTHR/PRETRK; Lock/Unlock (`0x94`/`0x14`) via MT bit, result `lock<<4`, no interrupt; DUMPREG dumps internal registers; ISA IRQ6
- IBM PC/AT IRQ assignment — floppy disk controller → IRQ6 (8259 master IR6)

## Firmware

- SeaBIOS documentation
- OVMF / EDK II documentation

## Oracles and tooling (behavior reference — do not copy source)

- QEMU machine model documentation / TCG as behavioral oracle
- Intel XED documentation
- kvm-unit-tests

## Web / Wasm

- WebAssembly specifications
- Browser API documentation (Workers, Canvas, OPFS, IndexedDB, AudioWorklet)

## Project docs

- `plan.md` — product scope and roadmap
- `docs/architecture.md` — crate boundaries (create in Milestone 0)
- `docs/cpu-profile-core2.md` — CPUID and feature exposure
- `docs/machine-model-pc-v1.md` — ports, MMIO, interrupt routing
- `docs/instruction-format.md` — metadata schema
- `docs/testing.md` — oracle hierarchy
- `docs/licensing.md` — license policy

Update this file when a new external reference is approved for use.
