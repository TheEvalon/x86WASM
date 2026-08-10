# CRTC / BDA cursor sync polish (R13 display-fw)

Milestone 2, Round 13, display-fw lane — slice 3.

## Goal

Keep FreeVGA CRTC Cursor Location High/Low (`0x0E`/`0x0F`) coherent with BDA
page-0 cursor (`0040:0050`) after host INT 10h writes that move the cursor.

## Sync points

| Path | Effect |
|------|--------|
| AH=00h mode set | BDA cursors cleared; CRTC location → 0 |
| AH=02h set cursor | BDA + CRTC location = `StartAddress + row*pitch + col` |
| AH=0Eh teletype | After advance/scroll, BDA + CRTC updated |
| AH=13h (AL bit0) | Optional cursor update writes BDA + CRTC |

## Spec refs

- FreeVGA CRT Controller — Cursor Location High/Low.
- Ralf Brown's Interrupt List — BDA `0040:0050`; INT 10h AH=02h/0Eh/13h.

## Still unsupported

- Multi-page CRTC/BDA tables (`0040:0052`…)
- Guest firmware writing CRTC without updating BDA (host does not reverse-sync)

## Files

- `crates/machine-pc/src/int10.rs`
- `docs/vga-r13-crtc-bda-cursor-sync.md` — this note
