# Planar 16-color display fetch (modes 0Dh, 0Eh, 10h, 12h)

Milestone 2, round 4, slice 3. The third — and, for now, last — display fetch
this model has. Read `docs/vga-r3-character-generator.md` and
`docs/vga-r3-graphics256-mode13h.md` first; this slice reuses their `VgaFrame`
form, their addressing multiplier and their honesty rules, and **differs from
both on one point**: the frame geometry is derived rather than fixed. That
difference is the most important thing on this page.

## Approved sources used here

- **IBM PS/2 Hardware Interface Technical Reference — Video Subsystems**
  (42G2193) chapter 2: the four-map parallel serialization; Figure 2-72
  Graphics Mode (Shift Register Interleave bit 5, 256-Color Shift Mode bit 6);
  Figure 2-74 Miscellaneous (Graphics/Alphanumeric bit 0, Chain Odd/Even
  bit 1); Figure 2-34 Map Selection (Chain 4); Figure 2-79 Attribute Mode
  Control.
- **FreeVGA**: Attribute Controller Registers — Attribute Mode Control `ATGE`
  and `8BIT`, Color Plane Enable, Internal Palette, Color Select, `P54S`;
  Color Registers PEL Mask; CRT Controller — Start Address, Offset, End
  Horizontal Display, Vertical Display Enable End, Overflow, Maximum Scan Line
  and Scan Doubling, Mode Control byte/word addressing, Underline Location `DW`.

Both are already listed in `docs/sources.md`.

## Which mode numbers this covers

`VgaText::render_mode` returns `VgaRenderMode::Graphics16Planar` only when the
whole planar signature is present. Each condition that fails names a *different*
fetch, which is why all seven are required:

| Register | Field | Required | What a violation means |
|---|---|---|---|
| GC Miscellaneous `06h` bit 0 | Graphics/Alphanumeric | set | alphanumeric |
| ATC Mode Control `10h` bit 0 | `ATGE` | set | alphanumeric |
| ATC Mode Control `10h` bit 6 | `8BIT` | clear | mode 13h or "mode X" |
| GC Mode `05h` bit 6 | `C256` | clear | mode 13h or "mode X" |
| GC Mode `05h` bit 5 | `SRI` | clear | CGA 4-color `04h`/`05h` |
| Sequencer Memory Mode `04h` bit 3 | Chain 4 | clear | mode 13h |
| GC Miscellaneous `06h` bit 1 | Chain Odd/Even | clear | CGA `04h`–`06h` |

That signature is what BIOS modes **`0Dh` (320×200), `0Eh` (640×200), `10h`
(640×350) and `12h` (640×480)** share. Nothing distinguishes them in the
register fields above; they differ only in the geometry registers below, which
is exactly why this renderer cannot use a fixed frame size.

**Modes that still do not render:** CGA-compatible `04h`, `05h` and `06h`; the
unchained 256-color "mode X" variants; monochrome `0Fh` and `11h` are not
special-cased (they satisfy the planar signature and render as 16-color frames
whose upper planes happen to be unused, which is what the hardware fetch does
too); every VBE mode. There is still no VBE at all.

## The fetch

Row *r* starts at CRTC address counter `StartAddress + r * Offset * 2`, which
the addressing multiplier turns into a display byte address — the same
`graphics_row_stride_bytes()` mode 13h uses. Under the byte-addressed planar
programming the multiplier is 1, so mode 12h's Offset `28h` gives an 80-byte
row stride, which is 640 pixels.

Pixel *x* of a row is bit `x % 8` of byte `x / 8`, taken across all four maps
at the *same* offset:

```text
byte   = row_byte + x / 8
mask   = 0x80 >> (x % 8)          # most significant bit is leftmost
index  = (map0[byte] & mask ? 1 : 0)
       | (map1[byte] & mask ? 2 : 0)
       | (map2[byte] & mask ? 4 : 0)
       | (map3[byte] & mask ? 8 : 0)
```

That 4-bit index then walks the display path this device already had:

```text
index → Color Plane Enable (ATC 12h, AND) → Internal Palette (ATC 00h-0Fh)
      → Mode Control P54S + Color Select (ATC 14h) → PEL Mask (3C6h) → DAC
```

Color Plane Enable is new here. Round 3 recorded it as "no display effect"
because no planar renderer existed; it now forces disabled planes to zero
before the palette lookup, per FreeVGA: "setting a bit to 0 will force the
corresponding color plane to 0".

`VgaText::frame_rgba8` converts the frame for a host exactly as it does for the
other two fetches.

## Frame geometry: derived, not fixed — and what that does *not* claim

Round 3 fixed 80×25 for text and 320×200 for mode 13h and said why: this model
has no CRTC timing, and the display-end registers have no reset defaults. That
reasoning still holds, but a single fixed size cannot serve four modes at three
different resolutions, so this fetch reads the two display-end registers as
**geometry**:

| Dimension | Source | Spec |
|---|---|---|
| width | `(End Horizontal Display + 1) * 8` dots | FreeVGA CRTC `01h`: the displayed character count minus one; 8 dots per character clock in graphics |
| displayed scan lines | `Vertical Display Enable End + 1`, 10 bits across CRTC `12h` and Overflow bits 1 and 6 | FreeVGA CRTC `12h` / `07h` |
| height in rows | displayed scan lines ÷ `((Maximum Scan Line & 1Fh) + 1) × (Scan Doubling ? 2 : 1)` | FreeVGA CRTC `09h` |

Checked against what a BIOS programs: `0Dh` → HDE 39, 400 scan lines, doubling
→ 320×200. `0Eh` → HDE 79, 400, doubling → 640×200. `10h` → HDE 79, 350 →
640×350. `12h` → HDE 79, 480 → 640×480.

**This is not a timing model.** Those two registers are read for their pixel
counts and nothing else. There is no pixel clock, no raster, no blanking, no
horizontal or vertical total, and no retrace relationship. Reading them here
says only "the guest told us how many dots and how many scan lines it is
displaying"; it does not widen the claim that Round 3 narrowed.

The corollary is deliberate and tested: **a CRTC that has not been programmed
produces a degenerate 8×1 frame.** This model does not invent a default
resolution to cover an unprogrammed register file.

## Model choices, not hardware

- **One output row per row of display memory.** Scan Doubling and a non-zero
  Maximum Scan Line shrink the frame rather than repeating rows, so mode `0Dh`
  is a 320×200 frame and a host that wants square-ish pixels scales. This is
  the same choice mode 13h makes.
- **The visible window is the display-end geometry, and Offset is the stride.**
  A larger Offset therefore acts as a virtual resolution, as in mode 13h.
- Display byte addresses wrap inside `plane_size_bytes()`, matching the CPU
  path, so clearing Extended Memory shrinks the addressable region.
- The word-mode Address Wrap (`17h` bit 5) rotation of MA13/MA15 onto MA0 is
  not modeled; the address multiplier is a plain shift.

## Still unsupported

- **No VBE**, no host display, no timing-accurate raster, no damage tracking.
- No CGA-compatible fetch (`04h`–`06h`) and no unchained "mode X".
- Horizontal PEL Panning, Line Compare / split screen, Preset Row Scan, Screen
  Disable, and the overscan border are not applied to any frame.
- The Attribute Controller's `PAS` bit does not blank the display.
- Blink has no effect on a graphics frame (`blink_off_half` is ignored), which
  matches the ATC Mode Control `BLINK` bit's alphanumeric-only role here; the
  graphics-mode blink attribute path is not modeled.
