# Host INT 10h AH=0Eh teletype deepen (R13 display-fw)

Milestone 2, Round 13, display-fw lane — slice 1.

## Goal

Deepen host INT 10h AH=0Eh teletype with classic BIOS-style **scroll** and
**attribute** fill, still text mode / page 0 only.

## Behavior

| Input | Behavior |
|-------|----------|
| Printable | Write at BDA cursor using the cell's current attribute (default `07h`); advance |
| Wrap past last column on last row | Scroll viewport up one row; blank bottom with the **written** attribute; cursor → `(24,0)` |
| `0Ah` LF on last row | Scroll up; blank bottom with default `07h`; cursor stays on last row |
| `0Dh` CR / `08h` BS / `07h` bell | Unchanged from R9 (move / no-op) |
| BH ≠ 0 | No-op (page 0 only) |

Moved rows keep character+attribute pairs. Spec: RBIL INT 10h AH=0Eh; classic
IBM VGA BIOS teletype scroll fill.

## Still unsupported

- Graphics teletype / BL as graphics foreground
- Multi-page regen
- Soft-scroll via CRTC Start Address (host copies cells instead)

## Files

- `crates/machine-pc/src/int10.rs`
- `docs/vga-r13-int10-teletype.md` — this note
