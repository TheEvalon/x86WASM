# Milestone 2 Round 13 parallel integration

Branch: `merge/m2-r13-parallel-16` (base `merge/m2-r12-parallel-16` / `f579e8c`).

Four lanes merged in order: platform-io → display-fw → boot-guest → cpu-vm86.

## Landed (16 slices)

| Lane | Branch tip | Highlights | Docs |
|---|---|---|---|
| platform-io | `732b75d` | LPT1 control idle `0x0C`; LPT2 + LPT3 `0x3BC` open-bus honesty; COM3/COM4 `0x3E8`/`0x2E8` stubs; CMOS `0Fh` shutdown status survives CF9 pulse | `docs/lpt-r13-*`, `docs/platform-r13-*` |
| display-fw | `96b4f78` | INT 10h AH=0Eh teletype deepen; AH=13h write-string; CRTC↔BDA cursor sync; VBE 4F01 mode-info **without** LFB | `docs/vga-r13-*` |
| boot-guest | `c19f6cc` | INT19-candidate HD/floppy attach; FreeDOS measure v6 with media; INT13 AH=00 reset; Linux setup deepen + El Torito media classify | `docs/boot-r13-*`, `docs/storage-r13-*` |
| cpu-vm86 | `021b611` | VME CLI/STI→VIF; PUSHF/POPF VIF/VIP; soft-int redirect deepen; INT3 ignores VME redirect | `docs/cpu-r13-*` |

## POST / CF9 carry-forward

- CF9 from R12 remains in the tree (`devices::Cf9Reset`).
- Without boot media, 20M POST still ends at `F000:9842` reboot loop (see `docs/post-c897-remeasure.md`).
- R13 boot lane adds INT19-candidate media helpers so firmware can leave the no-media loop; FreeDOS **prompt** and Linux **serial shell** are still not claimed.

## Merge notes

- Auto-merged `machine-pc/src/lib.rs`, `devices/src/lib.rs`, `docs/sources.md`.
- Manual `plan.md` resolution for boot vs platform status lines only.
- CPU lane last: sole `x86-interpreter` writer.

## Honesty that survives

- No guest LFB; VBE PhysBasePtr / LFB ModeAttributes stay clear.
- No `CPUID.VME` / `CPUID.APIC`.
- COM3/4 are probe stubs without shared IRQ routing; LPT has no IRQ7/ECP.
- FreeDOS/Linux paths do **not** claim prompt/shell.
- ADR-0008 `etc/table-loader` still absent.
- M2 exits still open until a measured FreeDOS prompt / Linux serial shell / formal POST-complete criterion.
