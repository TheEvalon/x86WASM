# VGA character generator and text-mode display fetch

Milestone 2, round 3, slice 1. Companion to `docs/vga-plane-memory-model.md`
(CPU-side plane decode) and `docs/vga-r2-mmio-entry-point.md` (the guest MMIO
entry point). Those two rounds built everything the CPU can *write*; nothing
read display memory back out as a picture. This slice adds the read side.

## Approved sources used here

- **FreeVGA** (already listed in `docs/sources.md`):
  - "VGA Text Mode Operation" — Display Memory Organization, Attributes,
    Fonts, Cursor
  - Sequencer Registers — Clocking Mode (index `01h`), Character Map Select
    (index `03h`), Memory Mode (index `04h`)
  - CRT Controller Registers — Maximum Scan Line (`09h`), Cursor Start (`0Ah`),
    Cursor End (`0Bh`), Start Address High/Low (`0Ch`/`0Dh`), Cursor Location
    High/Low (`0Eh`/`0Fh`), Offset (`13h`), Underline Location (`14h`), CRTC
    Mode Control (`17h`)
  - Attribute Controller Registers — Attribute Mode Control (`10h`), Color
    Select (`14h`), Internal Palette (`00h`–`0Fh`)
  - Color Registers — PEL Mask
- **IBM PS/2 Hardware Interface Technical Reference — Video Subsystems**
  (form 42G2193, Sep 1992), chapter 2: Figure 2-74 Miscellaneous
  (Graphics/Alphanumeric), Figure 2-79 Mode Control (Line Graphics Enable).

`docs/sources.md` should gain the IBM PS/2 Video Subsystems reference; that file
is owned by the integration coordinator for this round.

## What the fetch does

For each of the 80×25 character cells:

1. The CRTC address counter value is `StartAddress + row * pitch + col`, where
   `pitch = Offset * 2` character cells (`VgaText::text_row_pitch_chars`, kept
   from round 1). The counter is 16 bits wide and wraps there.
2. That counter is multiplied by the addressing mode's factor — 4 with
   Underline Location `DW`, 1 with CRTC Mode Control byte mode, otherwise 2 —
   and wrapped inside the enabled map size, giving a map offset
   (`VgaText::display_map_offset`).
3. The character code comes from map 0 at that offset and the attribute from
   map 1 at the *same* offset (FreeVGA "VGA Text Mode Operation", Display
   Memory Organization).
4. Attribute bit 3 chooses Character Set A or Character Set B; the selected
   3-bit field names one of eight 8 KiB font banks in map 2. Glyph rows are
   `bank + code * 32 + scan`.
5. Foreground and background DAC indices come from the existing host text
   helpers, so the frame passes exactly the chain the device already modeled:
   attribute → ATC Internal Palette → Mode Control `P54S` / Color Select →
   PEL Mask.
6. Scan lines inside the cursor's Start/End range, and the Underline Location
   row of an underline attribute, are drawn entirely in the foreground color.
7. Dot 9 (when Clocking Mode selects 9-dot cells) is background, except that
   Line Graphics Enable makes codes `C0h`–`DFh` repeat their eighth dot.

## Why the display address is doubled

Round 1 chose the *byte* form of the CPU odd/even offset — host address with A0
cleared — and recorded that "CRTC byte/word (`Count by Two`) compensation" was
missing. This slice supplies that compensation on the display side: in word
mode (CRTC Mode Control bit 6 clear, which is both the reset value here and the
mode-03h programming) the address counter is shifted left by one before it
indexes a map. Counter value *n* therefore reads map offset `2n`, which is
exactly where an odd/even CPU write to `0xB8000 + 2n` puts the character and
where `0xB8000 + 2n + 1` puts the attribute. The two halves now agree.

Word mode's Address Wrap bit (`17h` bit 5), which rotates MA13 or MA15 onto
MA0 rather than shifting in a zero, is **not** modeled; this is a plain shift.

## API

```rust
vga.render_mode() -> VgaRenderMode              // Text | Unsupported
vga.render_frame(blink_off_half: bool) -> Option<VgaFrame>
vga.render_text_frame(blink_off_half: bool) -> VgaFrame

VgaFrame { width, height, pixels: Vec<u8>, mode }   // pixels are DAC indices
VgaFrame::index_at(x, y) -> Option<u8>
VgaFrame::row(y) -> Option<&[u8]>

vga.dac_rgb6(dac_index) -> [u8; 3]              // raw DAC RAM, no PEL Mask
vga.frame_rgba8(&frame) -> Vec<u8>              // 8-bit RGBA for a host canvas
```

`render_frame` returns `None` for any programming this model does not fetch,
rather than rendering text that the hardware would not display. The caller owns
the blink phase because there is no vertical-retrace timer.

`frame_rgba8` scales 6-bit DAC components to 8 bits by replicating the high
bits (`v << 2 | v >> 4`), so `0x3F` maps to `0xFF`.

## Where the sources conflict

**Line Graphics Enable polarity.** IBM Figure 2-79 makes bit 2 set mean "the
ninth dot is identical to the eighth" for codes `C0h`–`DFh`. FreeVGA's
Attribute Mode Control page states the inverse: "If this field is set to 0,
then the 9th column of these characters is replicated from the 8th column.
Otherwise, if it is set to 1 then the 9th column is set to the background."
This model follows **IBM**, because the mode-03h reset default for Mode Control
is `0x0C` — the bit is *set* — and box-drawing characters are continuous in
mode 3 on real hardware, which only the IBM reading produces. FreeVGA's own
"VGA Text Mode Operation" page agrees with IBM in prose ("the Line Graphics
Enable field can be set to allow character codes C0h-DFh to have their ninth
column be identical to their eighth column").

**9/8 Dot Mode polarity.** FreeVGA's register page is explicit — Clocking Mode
bit 0 "0 - Selects 9 dots per character. 1 - Selects 8 dots per character" —
while its "VGA Text Mode Operation" page describes an 8-wide cell as "the 9/8
Dot Mode field is programmed to 0". The register page wins: it agrees with the
mode-03h default (`0x00`, 9-dot text) this device already asserts, and with the
existing `VGA_SEQ_CLOCKING_8DOT` constant.

**Underline Location scan line numbering.** FreeVGA says "The value programmed
is the scan line desired minus 1", which reads as a 1-based scan line. This
model treats the register value as the 0-based row index inside the cell,
because the classic monochrome programming (`0Dh` in a 14-scan-line cell) puts
the underline on the last drawn row only under that reading. The mode-03h
default `1Fh` is past a 16-line cell either way, so the underline is disabled
by default, which is what FreeVGA describes.

## Model choices, not hardware

- **The character grid is a fixed 80×25.** Horizontal Display End (`01h`) and
  Vertical Display End (`12h`) are stored but do not size the frame: this model
  has no CRTC timing and gives those registers no mode-03h reset default, so
  deriving the grid from them would produce a 1×1 screen. The renderer uses the
  same `VGA_TEXT_COLS`×`VGA_TEXT_ROWS` grid the rest of the device uses.
- **No font is installed at reset.** Plane 2 is zero, so a freshly reset device
  renders a uniform background. There is no built-in character ROM: a font is
  guest or host data, and none is vendored here.
- The display fetch wraps map offsets inside `plane_size_bytes()`, so clearing
  Extended Memory shrinks the addressable display region to 16 KiB per map, the
  same rule the CPU path uses.
- Character map selection is ignored (bank `0000h`) when Extended Memory is
  clear, per FreeVGA's Memory Mode `Ext. Mem` note.

## Still unsupported

- **No VBE, no host display, no timing-accurate raster.** Nothing drives the
  renderer on a schedule; a host calls it when it wants a frame.
- Graphics modes: `render_mode` reports `Unsupported` for any Graphics/
  Alphanumeric or `ATGE` programming, and `render_frame` returns `None`.
- Maximum Scan Line bit7 Scan Doubling, Preset Row Scan, Line Compare /
  split screen, Cursor Skew, Horizontal PEL Panning (`text_pel_pan` still
  reports the shift but the renderer does not apply it), Color Plane Enable,
  Overscan/border, Screen Disable, and the word-mode Address Wrap rotation.
- The blink rate (vertical sync ÷ 32) — the caller supplies the phase.
- The two display-memory backing stores are still separate in this slice: the
  renderer reads plane memory, while `0xB8000` CPU accesses still reach the
  legacy interleaved text buffer. Slice 2 retires that split.
