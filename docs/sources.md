# Approved sources

Authoritative references for implementation. Agents must cite these (or Intel SDM sections) instead of inventing behavior.

## CPU and architecture

- Intel 64 and IA-32 Architectures Software Developer's Manual (SDM), Volumes 1–4
- Intel processor datasheets relevant to Core 2 Conroe/Penryn CPUID presentation

## Platform and devices

- PCI specifications (as needed for the classic PC subset)
- ACPI specifications (for later machine/firmware work)
- ATA / ATAPI specifications
- PS/2 and 8042 controller references
- Intel 8259A Programmable Interrupt Controller datasheet (ICW1–ICW4 / OCW)
- Intel 8254 Programmable Interval Timer datasheet
- Motorola MC146818 / IBM PC AT CMOS RTC register map (ports `0x70`/`0x71`)

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
