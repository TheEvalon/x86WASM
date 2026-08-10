# Milestone 2 Round 6 parallel integration

Branch: `merge/m2-r6-parallel-16` (base `0a2bd20`).

Four lanes merged in order: platform-io → pci-fwcfg → atapi-vga → cpu.

## Landed (16 slices)

| Lane | Highlights | Docs |
|---|---|---|
| platform-io | LPT1/LPT2 probe; LAPIC/HPET/IOAPIC MMIO stubs | `docs/lpt-r6-*`, `docs/lapic-r6-*`, `docs/hpet-r6-*`, `docs/ioapic-r6-*` |
| pci-fwcfg | Honest absent `etc/table-loader` ADR; PM1a_EN SCI; bootorder | `docs/adr/0008-*`, `docs/pci-r5-acpi-pm.md`, `docs/fwcfg-r4-selectors.md` |
| atapi-vga | MODE SENSE / TOC / PREVENT; El Torito host load to `0x7C00` | `docs/atapi-r6-*.md` |
| cpu | CMPXCHG8B+CX8; `IA32_APIC_BASE`; LAR/LSL; same-CPL far CALL | `docs/cpu-r6-*.md` |

## Merge notes

- Conflict surface was `crates/machine-pc/src/lib.rs` imports (platform MMIO types + `FW_CFG_DEFAULT_BOOT_ORDER`); both sides kept.
- No tests dropped; all device modules retained.

## Honesty that survives

- LAPIC/HPET/IOAPIC are presence/capability stubs — no timer IRQ / RTE→IRQ yet.
- `etc/table-loader` remains intentionally absent (ADR-0008).
- El Torito is host-side load helper; no guest INT 13h CD path.
- Far CALL is same-CPL only; no task gates / VM86 / LDT call gates.
