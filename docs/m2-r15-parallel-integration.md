# Milestone 2 Round 15 parallel integration

Branch: `merge/m2-r15-parallel-16` (base `merge/m2-r14-parallel-16` / `f7f7a60`).

Lanes merged: platform-post → storage-int13 → display-fw → boot-guest → usb-timer.

(Orchestrator lanes were boot/display/platform/usb; a concurrent **storage-int13** worktree was also merged. Boot reverted INT13 AH=02 deepen to keep storage as the INT13 owner.)

## Landed

| Lane | Branch tip | Highlights | Docs |
|---|---|---|---|
| platform-post | `40837b1` | INT15 AH=88/E801; CMOS↔BDA equipment; BDA 40:6C ticks; PIT/PIC/RTC wait_irq deepen | `docs/platform-r15-*`, `docs/int15-r15-*`, `docs/cmos-r15-*` |
| storage-int13 | `59c6656` | INT13 AH=02 multi-sector; AH=42 DAP; CF/AH+BDA status; CD El Torito AH=42 | `docs/storage-r15-*` |
| display-fw | `5625853` | INT10 AH=05 page; AH=0E CR/LF/BS/BEL; AH=09/0A + AH=13 polish; VBE 4F03 get-mode; no LFB | `docs/vga-r15-*` |
| boot-guest | `cacdaca` | C897-with-media = wait_irq yield; host INT19→7C00 halt class; FreeDOS v8 KERNEL.SYS locate; Linux setup-entry deepen | `docs/boot-r15-*`, `docs/post-r15-*` |
| usb-timer | `2351851` | UHCI PORTSC reset; TD short-packet/stall; LAPIC LDR/DFR; HPET main-counter freerun (LEG_RT clear) | `docs/uhci-r15-*`, `docs/lapic-r15-ldr-dfr.md`, `docs/hpet-r15-main-counter.md` |

## POST / boot carry-forward

- No-media POST still `F000:9842` reboot loop (CF9 path).
- With INT19-candidate HD, 20M still stops at **`F000:C897`** (`wait_irq` sampling). Platform IRQ deepen did **not** clear that class; SeaBIOS firmware INT19 → `0000:7C00` still open.
- Host INT19 helper reaches `guest-halted-at-boot-sector` (not firmware INT19 success).
- FreeDOS next-gap: `kernel-name-located-missing-load` (not prompt).
- Linux: `setup-executed-missing-protected-kernel` (not serial shell).

## Merge notes

- Auto-merged `machine-pc/src/lib.rs`, device sources, `docs/sources.md`.
- Manual `plan.md` resolutions for multi-lane status lines.
- Boot lane reverted INT13 AH=02 deepen to avoid dual-write with storage.

## Honesty that survives

- No guest LFB; VBE LFB requests fail; PhysBasePtr clear.
- No `CPUID.APIC` / `CPUID.VME`.
- Host INT 10h/13h/15h/16h are not SeaBIOS IVT bodies.
- FreeDOS/Linux paths do **not** claim prompt/shell.
- ADR-0008 `etc/table-loader` still absent.
- M2 exits still open: SeaBIOS POST complete with boot media, FreeDOS prompt, Linux serial shell.
