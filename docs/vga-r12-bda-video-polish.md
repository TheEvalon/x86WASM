# BDA video columns/page polish (R12 display-fw)

Milestone 2, Round 12, display-fw lane — slice 3.

## Goal

Keep the classic BIOS Data Area video subset coherent after host INT 10h mode
set and the R12 cursor / write-char services. Columns (`0040:004A`) and active
page (`0040:0062`) must survive AH=01h / AH=09h / AH=0Ah.

## Fields (video subset)

| Phys | Field | Mode 03h default |
|------|-------|------------------|
| `0040:0049` | current mode | `03h` |
| `0040:004A` | columns (word) | `50h` (80) |
| `0040:004C` | page / regen size (word) | `1000h` |
| `0040:004E` | page start offset (word) | `0000h` |
| `0040:0050`–`005F` | cursor pages 0–7 | all `(0,0)` on mode set |
| `0040:0060` | cursor type | `0607h` (+ CRTC Start/End) |
| `0040:0062` | active page | `00h` |
| `0040:0063` | CRT controller base | `03D4h` |
| `0040:0084` | rows minus one | `18h` (24) |

Mode 13h via AH=00h sets columns=`28h` (40), page size=`FA00h`, and the same
CRT base / rows-minus-one / active-page=0 contract.

## Spec refs

- Ralf Brown's Interrupt List — BIOS Data Area video fields; INT 10h
  AH=00h/01h/09h/0Ah/0Fh.
- IBM VGA / FreeVGA — color CRTC index at `03D4h`.

## Still unsupported

- Mono CRT base `03B4h` path
- Multi-page active display (AH=05h)
- Guest LFB / VBE `PhysBasePtr` ≠ 0

## Files

- `crates/machine-pc/src/int10.rs`
- `docs/vga-r12-bda-video-polish.md` — this note
