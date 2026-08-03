# Approved sources

Authoritative references for implementation. Agents must cite these (or Intel SDM sections) instead of inventing behavior.

## CPU and architecture

- Intel 64 and IA-32 Architectures Software Developer's Manual (SDM), Volumes 1–4
- Intel SDM Vol. 2 — LGDT/SGDT (`0F 01 /2`, `/0`); LIDT/SIDT (`0F 01 /3`, `/1`); m16&32 pseudo-descriptor; register form `#UD`
- Intel SDM Vol. 3 §2.4.1 — GDTR base/limit
- Intel SDM Vol. 3 §2.4.3 — IDTR base/limit
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
- PS/2 and 8042 controller references
- Intel 8259A Programmable Interrupt Controller datasheet (ICW1–ICW4 / OCW)
- Intel 8254 Programmable Interval Timer datasheet
- Motorola MC146818 / IBM PC AT CMOS RTC register map (ports `0x70`/`0x71`; index bit7 = NMI disable)
- Intel SDM Vol. 3 §6.3.3 / §6.7 / §6.15 — `#NMI` (interrupt vector 2; not maskable by `IF`)
- Intel 8237A Programmable DMA Controller datasheet (addr/count/mode/mask/flip-flop)
- OSDev Wiki ISA DMA — AT port map and page registers (not port `0x80`)
- IBM VGA / classic PC color text frame buffer at physical `0xB8000` (80×25, char+attr)
- OSDev Text UI — VGA text-mode memory layout
- OSDev VGA Hardware / FreeVGA CRT Controller — color CRTC Address `0x3D4`, Data `0x3D5`, indexes `0x00`–`0x18`
- Intel 82077AA CHMOS Single-Chip Floppy Disk Controller — DOR/MSR/FIFO/DIR/CCR; Specify (`0x03`) two params (SRT|HUT, HLT|ND), no result/IRQ; Recalibrate (`0x07`) one unit-select param, PCN=0, ST0 Seek End (`0x20|US`), IRQ; Sense Interrupt Status (`0x08`) ST0+PCN (command ST0 latch or post-reset `0xC0|US`); DOR bit3 DMA/IRQ enable; IRQ on command complete
- OSDev Wiki Floppy Disk Controller — ports `0x3F0`–`0x3F7` excluding `0x3F6` (IDE); MSR RQM/DIO; Specify timing; Recalibrate → IRQ then Sense Interrupt; Sense Interrupt clears IRQ; ISA IRQ6
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
