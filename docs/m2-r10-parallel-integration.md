# Milestone 2 Round 10 parallel integration

Branch: `merge/m2-r10-parallel-16` (base `merge/m2-r9-parallel-16` / `d832290`).

Four lanes merged in order: timers-apic → display-fw → boot-guest → cpu-vm86.

## Landed (16 slices)

| Lane | Branch tip | Highlights | Docs |
|---|---|---|---|
| timers-apic | `6326e5f` | LAPIC TMR edge/level; TPR/PPR priority stub; HPET→IOAPIC Fixed wire; non-Fixed delivery-mode honesty | `docs/lapic-r10-*`, `docs/hpet-r10-*`, `docs/ioapic-r10-*` |
| display-fw | `fa65ca2` | INT 10h AH=02/03 cursor; AH=0F get mode; BDA video polish; option-ROM checksum honesty | `docs/vga-r10-*`, `docs/firmware-r10-*` |
| boot-guest | `88a8346` | INT 13h floppy AH=08/15; CD El Torito AH=41/42/48/4B; FreeDOS/Linux first-failure classify | `docs/storage-r10-*`, `docs/boot-r10-*` |
| cpu-vm86 | `5c7d26e` | VM86→CPL0 9-dword INT frame; far JMP/CALL/RETF while VM=1; INT n IOPL (INTO not IOPL-sensitive) | `docs/cpu-r10-*` |

## Merge notes

- Auto-merged `machine-pc/src/lib.rs` and `docs/sources.md` across device/display/boot lanes.
- Manual conflict only in `plan.md` (lane-local status lines → single R10 merge status).
- CPU lane last: sole writer of `x86-interpreter/src/lib.rs`.
- Quality gate on merge tip: see integrator run (`cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace`).

## Honesty that survives

- No guest LFB / VBE `4Fxx` / SeaVGABIOS-as-INT10 body.
- VME/PVI, soft-int redirect bitmap, 16-bit IDT gates from VM86 still out.
- No real SMM; ADR-0008 `etc/table-loader` still absent.
- FreeDOS/Linux measure stubs report `synthetic-halt` only — **not** prompt/shell / **not** M2 exit.
- CPUID APIC bit remains clear; ExtINT/NMI/SMI IOAPIC modes are explicit non-Fixed honesty.
- SeaBIOS POST still incomplete (prior stop `F000:C897` not claimed fixed by this round).
