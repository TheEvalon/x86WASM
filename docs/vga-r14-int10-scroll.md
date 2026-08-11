# Host INT 10h AH=06h / AH=07h scroll window (R14 display-fw)

Milestone 2, Round 14, display-fw lane — slice 1.

## Goal

Bounded host INT 10h scroll-window services for classic text-mode guests
(boot messages, DOS-style consoles) without claiming a full VGA BIOS.

## API

| Service | Behavior |
|---------|----------|
| AH=06h | Scroll **up** inside CH/CL–DH/DL; AL=lines (`0` = blank whole window); BH=blank attribute |
| AH=07h | Scroll **down** with the same register contract |

Constraints for this stub:

- Text mode only (`VgaRenderMode::Text`)
- Window corners clamped to BDA columns × 25 rows
- Moved cells keep character+attribute pairs
- Vacated rows filled with spaces + BH
- Cursor / CRTC Location unchanged

## Spec refs

- Ralf Brown's Interrupt List — INT 10h AH=06h "SCROLL UP WINDOW", AH=07h
  "SCROLL DOWN WINDOW".
- FreeVGA / IBM VGA — alphanumeric map 0/1 cell layout used by host
  `char_at` / `attr_at` / `put_char`.

## Still unsupported

- Graphics scroll
- Soft-scroll via CRTC Start Address alone (this service copies cells)
- Multi-page regen windows

## Files

- `crates/machine-pc/src/int10.rs`
- `docs/vga-r14-int10-scroll.md` — this note
