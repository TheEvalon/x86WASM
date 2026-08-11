# Text font / CRTC polish for scroll + read (R14 display-fw)

Milestone 2, Round 14, display-fw lane — slice 4.

## Goal

Keep host text mode usable after INT 10h mode set for AH=06h/07h scroll and
AH=08h read-char: install the procedural bring-up font, reaffirm Maximum Scan
Line, and expose a typed CRTC Start Address setter for viewport soft-scroll
harnesses — still **no** guest LFB.

## Behavior

| Surface | Effect |
|---------|--------|
| AH=00h / AX=4F02h → mode `03h` | `vga.reset()`, then `install_bringup_font()`, Max Scan Line `0x0F`, cursor type defaults |
| `VgaText::text_font_installed` | `true` after mode 03h host set |
| `VgaText::set_text_start_address(addr)` | Writes CRTC `0x0C`/`0x0D` |
| Host `char_at` / `put_char` / AH=08h | Relative to Start Address viewport |
| INT 10h AH=02h cursor Location | `StartAddress + row * pitch + col` |

Protect does not block Start Address indexes. Max Scan Line is programmed after
`reset()` (Protect clear). Bring-up glyphs are procedural markers — **not** IBM
CP437 (`docs/vga-r7-font-install.md`, `docs/vga-r4-font-provenance.md`).

## Spec refs

- FreeVGA Fonts — map 2 glyph banks; character generator fetch.
- FreeVGA CRT Controller — Maximum Scan Line (`0x09`); Start Address High/Low
  (`0x0C`/`0x0D`); Cursor Location High/Low (`0x0E`/`0x0F`).
- Ralf Brown's Interrupt List — INT 10h AH=00h/08h; BDA cursor vs CRTC Location.

## Still unsupported

- Soft-scroll replacing AH=06h/07h cell copies
- Guest LFB aperture
- Automatic Start Address updates from teletype / window scroll (those still
  copy cells)
- SeaVGABIOS font load / CP437 ROM

## Files

- `crates/machine-pc/src/int10.rs` — mode-03 font + Max Scan Line
- `crates/devices/src/vga.rs` — `set_text_start_address`
- `docs/vga-r14-text-font-crtc.md` — this note
