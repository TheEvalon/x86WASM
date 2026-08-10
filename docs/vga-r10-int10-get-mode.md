# Host INT 10h AH=0Fh get video mode (R10 display-fw)

Milestone 2, Round 10, display-fw lane — slice 2.

## Goal

Return the current BIOS video mode, column count, and active page from BDA /
VGA-backed state via host INT 10h AH=0Fh.

## API

| Out | Source |
|-----|--------|
| AL | `0040:0049` current mode |
| AH | low byte of `0040:004A` columns |
| BH | `0040:0062` active page |

After AH=00h AL=03h: AL=`03h`, AH=`50h` (80), BH=`00h`.  
After AH=00h AL=13h: AL=`13h`, AH=`28h` (40), BH=`00h`.

## Spec refs

- Ralf Brown's Interrupt List — INT 10h AH=0Fh "GET CURRENT VIDEO MODE"
- IBM PC BDA video fields `0040:0049` / `0040:004A` / `0040:0062`

## Still unsupported

- Modes other than 03h/13h via AH=00h
- Multi-page displays
- VBE `AX=4F00h`+ / guest LFB

## Files

- `crates/machine-pc/src/int10.rs`
- `docs/vga-r10-int10-get-mode.md` — this note
