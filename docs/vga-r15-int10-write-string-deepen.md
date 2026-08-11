# Host INT 10h AH=13h write-string deepen (R15 display-fw)

Milestone 2, Round 15, display-fw lane — slice 2.

## Goal

Extend the R13 AH=13h stub for FreeDOS-ish text: scroll on page overflow and
honest cursor placement for AL=01h/03h attribute variants.

## Deepen vs R13

| Topic | R13 stub | R15 |
|-------|----------|-----|
| Page overflow | Stop writing | Scroll up one row (fill = last written attr) and continue |
| Cursor after overflow | Clamp to `(24,79)` | Next empty cell after wrap/scroll (often `(24,0)`) |
| AL=03h | Bits accepted | Explicit char+attr pairs + cursor update covered by tests |
| BH | Page 0 only | 0 or active page (after AH=05h) |

AL bits 2–7 non-zero remain a no-op. CX still capped at `INT10_WRITE_CHAR_MAX_COUNT`.
Scroll shares the AH=06h cell-copy helper (`scroll_text_window_cells`).

## Spec refs

- Ralf Brown's Interrupt List — INT 10h AH=13h "WRITE STRING".
- Prior: `docs/vga-r13-int10-write-string.md`; scroll path shares R14 AH=06h cells.

## Still unsupported

- CR/LF/BS interpretation inside the string
- Graphics write-string
- SeaVGABIOS body

## Files

- `crates/machine-pc/src/int10.rs`
- `docs/vga-r15-int10-write-string-deepen.md` — this note
