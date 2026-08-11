# Host INT 10h AH=08h read character/attribute (R14 display-fw)

Milestone 2, Round 14, display-fw lane — slice 2.

## Goal

Return the text cell under the current BDA cursor so guests can probe what
was written (menus, DOS console helpers).

## API

| Input | Output |
|-------|--------|
| BH = page | AL = character, AH = attribute |

Constraints:

- Page 0 only; non-zero BH leaves AX unchanged
- Text mode only
- Missing cell defaults to space / attribute `07h`
- Mode 03h host set installs the bring-up font so the cell is renderable
  (`docs/vga-r14-text-font-crtc.md`)

## Spec refs

- Ralf Brown's Interrupt List — INT 10h AH=08h "READ CHARACTER AND ATTRIBUTE
  AT CURSOR POSITION".
- FreeVGA alphanumeric maps 0/1.

## Still unsupported

- Graphics read-char
- Multi-page regen

## Files

- `crates/machine-pc/src/int10.rs`
- `docs/vga-r14-int10-read-char.md` — this note
