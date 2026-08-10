# Milestone 2 Round 11 parallel integration

Branch: `merge/m2-r11-parallel-16` (base `merge/m2-r10-parallel-16` / `b46e5f0`).

Four lanes merged in order: usb-timer → platform-post → boot-guest → cpu-vm86.

## Landed (16 slices)

| Lane | Branch tip | Highlights | Docs |
|---|---|---|---|
| usb-timer | `e65f818` | UHCI frame-list N-walk + QH hop; PORTSC CCS/PED/PR; LAPIC LVT Timer polish; HPET 32-bit wrap + level EOI re-assert | `docs/uhci-r11-*`, `docs/lapic-r11-*`, `docs/hpet-r11-*` |
| platform-post | `1147f83` | IRQ1→BDA `40:1E` ring; APM INT 15h AH=53 install/connect stub; port61 parity/IOCHK NMI; POST remeasure still `F000:C897` | `docs/kbd-r11-*`, `docs/apm-r11-*`, `docs/platform-r11-*` |
| boot-guest | `377d42f` | INT13 AH=4A/4B El Torito terminate/status; HD AH=08 geometry; FreeDOS BDA equipment seed; Linux boot-protocol header inspect | `docs/storage-r11-*`, `docs/boot-r11-*` |
| cpu-vm86 | `094b259` | VM86 PUSH/POP Sreg; VIP/VIF sticky without VME; INT3 frame + ICEBP unsupported; opsize-32 far JMP/CALL/RETF | `docs/cpu-r11-*` |

## Merge notes

- Auto-merged `machine-pc/src/lib.rs`, `devices/src/lib.rs`, `docs/sources.md` without manual conflict resolution.
- CPU lane last: sole writer of `x86-interpreter/src/lib.rs`.
- Platform lane was stalled mid-flight; orchestrator finished commits + gate in the worktree.

## Honesty that survives

- POST still stops at `F000:C897` (unchanged vs R9/R10 platform remeasure).
- No real SMM; APM INT15 is host stub only.
- No guest LFB / VBE `4Fxx`; no CR4.VME / CPUID.VME.
- FreeDOS/Linux measure paths still do **not** claim prompt/shell.
- ADR-0008 `etc/table-loader` still absent; CPUID APIC bit remains clear.
- UHCI isochronous / deep QH chains still out.
