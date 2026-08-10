# UHCI one-TD stub — skipped (R7 timers/apic lane)

## Decision

Optional slice 4 (one UHCI transfer-descriptor stub in `pci.rs`) is **not**
implemented in this lane.

## Why

- Slices 1–3 (HPET comparator, LAPIC timer/LVT, IOAPIC RTE→LAPIC) are complete.
- UHCI lives in shared `crates/devices/src/pci.rs`, which other R7 lanes may
  also touch (storage/guest, PCI honesty). Editing it here risks merge fights.
- Mission allowed skip-with-document when contested.

## Remaining for a dedicated owner

- ~~One-TD schedule walk (frame list → QH → TD) with host mem callbacks~~ → **done in R8** (`docs/uhci-r8-one-td.md`)
- Multi-TD / port / IRQ / real USB device stack still deferred
- Honesty: no full UHCI HC, no real USB devices
