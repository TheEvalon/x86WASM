# Host INT 10h AH=01h set cursor type (R12 display-fw)

Milestone 2, Round 12, display-fw lane — slice 1.

## Goal

Extend the host INT 10h stub with SET CURSOR TYPE against the BIOS Data Area
and FreeVGA CRTC Cursor Start/End registers.

## API

| Service | Behavior |
|---------|----------|
| AH=01h | CH=start scanline, CL=end scanline → BDA `0040:0060` (CX layout) and CRTC `0x0A`/`0x0B` |

CH bit5 (`20h`) is the VGA cursor-disable flag and is stored/programmed as-is.
AH=03h continues to return the BDA type after AH=01h.

## Spec refs

- Ralf Brown's Interrupt List — INT 10h AH=01h "SET CURSOR TYPE"; BDA
  `0040:0060`.
- FreeVGA CRT Controller — Cursor Start (`0x0A`), Cursor End (`0x0B`), disable
  bit.

## Still unsupported

- Multi-page cursor type tables
- AH=05h active page select
- Guest LFB / VBE `4Fxx` delivery (host VBE info blocks remain separate)
- Full SeaVGABIOS / guest INT 10h body

## Files

- `crates/machine-pc/src/int10.rs`
- `docs/vga-r12-int10-cursor-type.md` — this note
