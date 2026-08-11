# Milestone 2 Round 14 parallel integration

Branch: `merge/m2-r14-parallel-16` (base `merge/m2-r13-parallel-16` / `9934398`).

Four lanes merged in order: usb-timer → platform-kbd → display-fw → boot-guest.

## Landed (16 slices)

| Lane | Branch tip | Highlights | Docs |
|---|---|---|---|
| usb-timer | `7ffe122` | UHCI→PIRQD→IRQ11 PIC wire; IOC/USBSTS deepen; LAPIC SVR Focus polish (no CPUID.APIC); HPET LEG_RT clear so IRQ0/IRQ8 stay PIT/CMOS | `docs/usb-r14-*`, `docs/apic-r14-lapic-svr.md`, `docs/timer-r14-hpet-legacy.md` |
| platform-kbd | `c67c059` | INT16 AH=02/12 shift status; BDA equipment/keyboard seed for FreeDOS; 8042 IRQ1 modifier→BDA + ring drain | `docs/kbd-r14-*`, `docs/platform-r14-bda-equipment-kbd.md` |
| display-fw | `5774434` | INT10 AH=06/07 scroll; AH=08 read char; VBE 4F02 set-mode **without** LFB; text font/CRTC polish | `docs/vga-r14-*` |
| boot-guest | `e151a6f` | POST-with-media measure (past `F000:9842`); MBR→VBR `load_active_vbr_to_7c00`; FreeDOS measure v7; Linux/El Torito deepen | `docs/boot-r14-*`, `docs/storage-r14-*` |

## POST / boot carry-forward

- CF9 from R12 remains; no-media POST still classifies at `F000:9842`.
- With INT19-candidate HD attached, 20M POST-with-media measure leaves the `9842` reboot class and stops at **`F000:C897`** (`other-stop`, ~40% halt-idle) — see `docs/boot-r14-post-with-media.md`.
- FreeDOS **prompt** and Linux **serial shell** are still not claimed. Host INT 13h/10h/16h remain host stubs, not SeaBIOS bodies.

## Merge notes

- Auto-merged `machine-pc/src/lib.rs` (int16/mbr exports combined), `docs/sources.md`.
- Manual `plan.md` resolutions to keep all four R14 status lines.
- Order avoided dual writers on hot device files during the lane phase.
- Tip briefly drifted when a concurrent usb-timer docs rename FF landed on the merge branch after a reset; restored to this four-lane tip (`394f05a`+) before recording this report.

## Honesty that survives

- No guest LFB; VBE 4F02 LFB requests fail; PhysBasePtr / ModeAttributes LFB stay clear.
- No `CPUID.APIC` / `CPUID.VME`.
- HPET does not silently replace PIT/RTC IRQ0/IRQ8 (`LEG_RT_CAP/CNF` clear).
- FreeDOS/Linux paths do **not** claim prompt/shell.
- ADR-0008 `etc/table-loader` still absent.
- M2 exits still open until a measured FreeDOS prompt / Linux serial shell / formal POST-complete criterion.
