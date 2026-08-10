# Milestone 2 Round 7 parallel integration

Branch: `merge/m2-r7-parallel-16` (base `merge/m2-r6-parallel-16` / `def88ae`).

Four lanes merged in order: timers-apic → display-boot → storage-guest → cpu-pm.

## Landed (15 slices; UHCI skipped)

| Lane | Highlights | Docs |
|---|---|---|
| timers-apic | HPET Timer0 comparator IRQ; LAPIC ICR/CCR/DCR + LVT/SVR; IOAPIC Fixed RTE→LAPIC | `docs/hpet-r7-*`, `docs/lapic-r7-*`, `docs/ioapic-r7-*`, `docs/uhci-r7-one-td-skipped.md` |
| display-boot | SeaVGABIOS Windows infeasibility note; option-ROM far-call; bring-up font; host VGA frame | `docs/firmware-r7-*`, `docs/vga-r7-*` |
| storage-guest | Host INT 13h HD subset; floppy→0x7C00; UART RX+RDA IRQ; guest measure harness | `docs/storage-r7-*` |
| cpu-pm | JMP TSS32/task-gate; GDT call-gate CALL; VERR/VERW; LLDT/SLDT + LDT call-gate | `docs/cpu-r7-*` |

## Merge notes

- `crates/machine-pc/src/lib.rs`: kept OptionRom invoke errors from display; **did not** restore El Torito `load_*` / `MachineError::ElTorito*` (R6 inspect-only policy on `def88ae`).
- Storage `mod eltorito_load` dropped at merge for the same reason; kept `guest_boot` + `int13` + timer wire mods.
- CPU lane merged cleanly (interpreter-only surface).

## Honesty that survives

- UHCI one-TD deferred (`docs/uhci-r7-one-td-skipped.md`).
- El Torito remains inspect-only (no guest CD INT 13h / no `load_eltorito_to_7c00`).
- SeaVGABIOS Windows host build still infeasible; Linux/WSL path documented.
- INT 13h is host-installed HD subset (AH=00/02/08), not full BIOS.
