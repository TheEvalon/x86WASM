# Chain-4 256-color graphics display fetch (mode 13h)

Milestone 2, round 3, slice 3. The second — and only other — display fetch this
model has. Read `docs/vga-r3-character-generator.md` first; this slice reuses
its `VgaFrame` form, its addressing multiplier and its honesty rules.

## Approved sources used here

- **IBM PS/2 Hardware Interface Technical Reference — Video Subsystems**
  (42G2193) chapter 2: Figure 2-34 Map Selection, Chain 4; Figure 2-72
  Graphics Mode (256-Color Shift Mode); Figure 2-74 Miscellaneous
  (Graphics/Alphanumeric).
- **FreeVGA**: Attribute Controller Registers — Attribute Mode Control `ATGE`
  and `8BIT`, Color Select; Sequencer Memory Mode `Chain 4`; CRT Controller
  Offset, Start Address, Underline Location `DW`; Color Registers PEL Mask.

## What renders, exactly

`VgaText::render_mode` returns `VgaRenderMode::Graphics256Chain4` only when the
whole mode-13h signature is present:

| Register | Field | Required |
|---|---|---|
| Graphics Controller Miscellaneous `06h` | Graphics/Alphanumeric, bit 0 | set |
| Graphics Mode `05h` | `C256` / 256-Color Shift Mode, bit 6 | set |
| Sequencer Memory Mode `04h` | Chain 4, bit 3 | set |
| Attribute Mode Control `10h` | `ATGE`, bit 0 | set |
| Attribute Mode Control `10h` | `8BIT`, bit 6 | set |

Anything less reports `VgaRenderMode::Unsupported` and `render_frame` returns
`None`. Memory Map Select is deliberately *not* part of the test: the CRTC
addresses display memory directly, so where the CPU aperture sits does not
change what is displayed.

**Modes that do not render:** every planar 16-color mode (`0Dh`, `0Eh`, `10h`,
`12h`), CGA-compatible 4-color and 2-color modes (`04h`–`06h`), the 320×200
256-color unchained "mode X" variants, and every VBE mode. There is no VBE at
all. Text mode 03h renders through the character generator from slice 1.

## The fetch

Row *r* starts at CRTC address counter `StartAddress + r * Offset * 2`. The
addressing multiplier (4 under mode 13h's doubleword programming) turns that
into a display byte address; pixel *x* of the row is the next byte along, so
the row stride is `Offset * 2 * 4` = 320 bytes with the mode-13h Offset `0x28`.

Each display byte address *n* resolves the same way a CPU chain-4 access does
(IBM Figure 2-34, and `docs/vga-plane-memory-model.md` for the offset form):

```text
map    = n & 3
offset = n & !3
```

so a byte the guest wrote at `0xA0000 + n` is the pixel displayed at *n*.

That byte is the DAC index. With `8BIT` set the Internal Palette and Color
Select take no part — FreeVGA Color Select: "In mode 13 hex, the 8-bit
attribute is the digital color value to the video DAC" — and only the PEL Mask
is applied. `VgaText::frame_rgba8` converts the frame for a host exactly as it
does for text.

## Model choices, not hardware

- **The frame is a fixed 320×200.** Same reason as the 80×25 text grid:
  this model has no CRTC timing and gives Horizontal/Vertical Display End no
  reset defaults, so they cannot size the frame. A larger Offset therefore acts
  as a virtual resolution — the row stride grows while the visible window stays
  320 pixels wide.
- **Scan doubling is not applied.** Real mode 13h sets Maximum Scan Line bit 7
  so 200 rows fill 400 scan lines. The frame here is 200 rows; a host that
  wants square-ish pixels scales.
- Display byte addresses wrap inside `plane_size_bytes()`, matching the CPU
  path, so clearing Extended Memory shrinks the addressable region.

## Still unsupported

- No VBE, no host display, no timing-accurate raster, no double buffering or
  page flip beyond what Start Address already gives.
- No planar graphics renderer, so Color Plane Enable, the Shift Register
  Interleave mode, and the 4-bit attribute path have no display effect.
- Horizontal PEL Panning, Line Compare / split screen, Preset Row Scan, Screen
  Disable, and the overscan border are not applied to a graphics frame either.
