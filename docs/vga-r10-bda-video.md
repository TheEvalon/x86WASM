# BDA video area polish (R10 display-fw)

Milestone 2, Round 10, display-fw lane — slice 3.

## Goal

Keep the classic BIOS Data Area **video subset** coherent after host INT 10h
AH=00h (mode set) and the R10 cursor / get-mode services.

## Fields (video subset)

| Phys | Field | Mode 03h default |
|------|-------|------------------|
| `0040:0049` | current mode | `03h` |
| `0040:004A` | columns (word) | `50h` (80) |
| `0040:004C` | page / regen size (word) | `1000h` (4 KiB) |
| `0040:004E` | page start offset (word) | `0000h` |
| `0040:0050` | cursor page 0 (col,row) | `(0,0)` then AH=02h |
| `0040:0060` | cursor type (end,start) | `07h`,`06h` (`0607h`) |
| `0040:0062` | active page | `00h` |

Mode 13h via AH=00h sets columns=`28h` (40) and page size=`FA00h` (320×200).

The phrase "40:00–40:10 subset" here means the first video-area bytes starting
at `0040:0049` through the early page/cursor words (not COM/LPT at `0040:0000`).

## Spec refs

- Ralf Brown's Interrupt List — BIOS Data Area memory map video fields;
  INT 10h AH=00h/02h/03h/0Fh.
- IBM VGA / FreeVGA — mode 03h text geometry.

## Still unsupported

- CRT controller base port at `0040:0063`
- Multi-page cursor table fill for pages 1–7
- Guest LFB / VBE `4Fxx`

## Files

- `crates/machine-pc/src/int10.rs`
- `docs/vga-r10-bda-video.md` — this note
