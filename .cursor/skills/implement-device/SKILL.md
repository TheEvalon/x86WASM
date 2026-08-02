---
name: implement-device
description: Implements a bounded PC device feature slice (registers, reset, IRQ, tests, snapshot). Use when working on PIC, PIT, IDE, VGA, APIC, serial, keyboard, or other machine devices — not CPU opcodes.
---

# Implement device

## Preconditions

- Device and register/command subset named (e.g. `8259 ICW1–ICW4` only).
- Machine-model notes in `docs/machine-model-pc-v1.md` or ADR exist or are created in-slice.

## Workflow

```text
Device Progress:
- [ ] 1. Document ports/MMIO and reset state
- [ ] 2. Failing device tests
- [ ] 3. Implement register behavior only in scope
- [ ] 4. IRQ / DMA wiring if in scope
- [ ] 5. Snapshot round-trip if state is persisted
- [ ] 6. Trace hooks for debugging
- [ ] 7. quality-gate
```

## Rules

- Stay behind bus traits (port I/O / MMIO); no browser types in device crates.
- Prefer QEMU/SeaBIOS-compatible behavior for the classic PC subset — from specs and observed behavior, not copied source.
- Out-of-scope commands return documented unimplemented behavior; do not silently ignore forever without a test/note.
- One device concern per slice (e.g. ICW init without OCW).

## Example

```text
8259 PIC ICW1–ICW4 only:
- reset state
- single and cascaded mode
- vector offsets
- invalid init sequence
- master/slave wiring
- snapshot round trip
- no OCW in this issue
```
