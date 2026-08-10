# Host INT 10h AH=09h/0Ah write character (R12 display-fw)

Milestone 2, Round 12, display-fw lane — slice 2.

## Goal

Bounded host INT 10h write-character services that poke the text buffer at the
current BDA cursor without claiming a full VGA BIOS.

## API

| Service | Behavior |
|---------|----------|
| AH=09h | AL=char, BH=page, BL=attr, CX=count → write char+attr at cursor; **no** cursor advance |
| AH=0Ah | AL=char, BH=page, CX=count → write char only (keep existing attr); **no** cursor advance |

Constraints for this stub:

- Page 0 only; non-zero BH is a no-op
- Text mode only (`VgaRenderMode::Text`)
- CX capped at `INT10_WRITE_CHAR_MAX_COUNT` (80×25) and stops at page end
- Horizontal wrap within the page using BDA columns

## Spec refs

- Ralf Brown's Interrupt List — INT 10h AH=09h "WRITE CHARACTER AND ATTRIBUTE
  AT CURSOR POSITION", AH=0Ah "WRITE CHARACTER ONLY AT CURSOR POSITION".
- FreeVGA / IBM VGA — alphanumeric map 0/1 cell layout used by host
  `put_char` / `attr_at`.

## Still unsupported

- Graphics teletype / write-char
- Multi-page regen buffers
- Scroll on page overflow
- Guest LFB / VBE `4Fxx` delivery

## Files

- `crates/machine-pc/src/int10.rs`
- `docs/vga-r12-int10-write-char.md` — this note
