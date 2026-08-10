# Host INT 10h AH=13h write string stub (R13 display-fw)

Milestone 2, Round 13, display-fw lane — slice 2.

## Goal

Bounded host INT 10h AH=13h WRITE STRING for bring-up text output. Not a full
VGA BIOS string path.

## API

| Field | Stub behavior |
|-------|---------------|
| AL bit0 | Update BDA+CRTC cursor after write |
| AL bit1 | String is `(char,attr)` pairs; else chars + BL attribute |
| AL bits 2–7 | Non-zero → no-op (reserved) |
| BH | Page 0 only |
| CX | Capped at `INT10_WRITE_CHAR_MAX_COUNT` (80×25) |
| DH/DL | Start row/col |
| ES:BP | String bytes in guest RAM |

Stops at the end of the 80×25 page (**no** scroll). Text mode only.

## Spec refs

- Ralf Brown's Interrupt List — INT 10h AH=13h "WRITE STRING".

## Still unsupported

- Scroll on page overflow
- Multi-page / graphics write-string
- SeaVGABIOS body at the IVT target

## Files

- `crates/machine-pc/src/int10.rs`
- `docs/vga-r13-int10-write-string.md` — this note
