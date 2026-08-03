# Approved sources

Authoritative references for implementation. Agents must cite these (or Intel SDM sections) instead of inventing behavior.

## CPU and architecture

- Intel 64 and IA-32 Architectures Software Developer's Manual (SDM), Volumes 1–4
- Intel processor datasheets relevant to Core 2 Conroe/Penryn CPUID presentation

## Platform and devices

- PCI Local Bus Specification — Configuration Mechanism #1 (`CONFIG_ADDRESS` `0xCF8`, `CONFIG_DATA` `0xCFC`, Type 1 address, enable bit 31)
- OSDev Wiki PCI — configuration space access mechanism #1; absent device → `0xFFFFFFFF`
- Intel 440FX (i440FX) host-bridge identity (`vendor 0x8086`, `device 0x1237`) as classic QEMU/SeaBIOS-compatible stub ID (behavior from specs/oracles, not copied source)
- ACPI specifications (for later machine/firmware work)
- ATA / ATAPI specifications (IDENTIFY DEVICE `0xEC`, READ SECTORS `0x20`, task-file / status bits, LBA28 PIO)
- OSDev Wiki ATA PIO Mode — primary ports `0x1F0`–`0x1F7` / `0x3F6`, IDENTIFY/READ polling
- PS/2 and 8042 controller references
- Intel 8259A Programmable Interrupt Controller datasheet (ICW1–ICW4 / OCW)
- Intel 8254 Programmable Interval Timer datasheet
- Motorola MC146818 / IBM PC AT CMOS RTC register map (ports `0x70`/`0x71`)
- Intel 8237A Programmable DMA Controller datasheet (addr/count/mode/mask/flip-flop)
- OSDev Wiki ISA DMA — AT port map and page registers (not port `0x80`)
- IBM VGA / classic PC color text frame buffer at physical `0xB8000` (80×25, char+attr)
- OSDev Text UI — VGA text-mode memory layout

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
