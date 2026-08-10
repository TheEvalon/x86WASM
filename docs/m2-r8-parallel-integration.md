# Milestone 2 Round 8 parallel integration

Branch: `merge/m2-r8-parallel-16` (base `merge/m2-r7-parallel-16` / `8249092`).

Four lanes merged in order: usb-timer → pci-acpi → boot-guest → cpu-vm86.

## Landed (16 slices)

| Lane | Branch tip | Highlights | Docs |
|---|---|---|---|
| usb-timer | `91bdec6` | UHCI one-TD schedule stub; LAPIC IRR/ISR + EOI; HPET Timer0 periodic; IOAPIC Remote IRR/EOI | `docs/uhci-r8-*`, `docs/lapic-r8-*`, `docs/hpet-r8-*`, `docs/ioapic-r8-*` |
| pci-acpi | `767e76f` | PM1a SLP_EN soft-off/sleep latch; PCI Status RMA/STA honesty; SCI_EN + optional SCI→PIRQ; fw_cfg `etc/system-states` | `docs/acpi-r8-*`, `docs/pci-r8-*`, `docs/fwcfg-r8-*` |
| boot-guest | `109c488` | INT 13h AH=03 write; AH=41h/42h extensions; El Torito no-emul load→0x7C00; guest measure harness v2 | `docs/storage-r8-*`, `docs/boot-r8-*` |
| cpu-vm86 | `5882589` | CALL-form TSS32/task-gate; IRET NT nested return; ARPL (VM86 deferred); paging A/D pin + INVLPG | `docs/cpu-r8-*` |

## Merge notes

- Auto-merged without manual conflict resolution (`pci.rs`, `devices/lib.rs`, `machine-pc/lib.rs`, `sources.md`).
- USB lane touched `pci.rs` for UHCI BAR presence; pci-acpi then extended Status/SCI on the same file — ort kept both.
- Boot restored `eltorito_load` (host no-emulation load to `0x7C00`); still not a full guest CD INT 13h BIOS.
- CPU deferred full VM86 enter/leave; shipped ARPL + blockers doc (`docs/cpu-r8-vm86-minimal.md`).

## Honesty that survives

- Minimal VM86 still deferred (ARPL only).
- El Torito load is host-side; no full guest CD BIOS path.
- INT 13h remains a host-installed subset (now 00/02/03/08/41/42), not SeaBIOS.
- `etc/table-loader` still absent (ADR-0008); `etc/system-states` is published.
- SeaVGABIOS Windows host build still infeasible.
- UHCI is one-TD stub only — no full USB stack.
- SeaBIOS POST / FreeDOS prompt / Linux serial shell still open.
