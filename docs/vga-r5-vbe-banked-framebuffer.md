# Banked VBE framebuffer view (no invented LFB hardware)

Milestone 2, round 5, slice 2. Slice 1 described the modes; this slice names
the one programming whose **host** linear pixel buffer the renderer already
drives, and records why a guest LFB is not invented.

## Approved sources used here

- **VBE 2.0** — ModeAttributes D6/D7 (windowing vs linear framebuffer),
  `WinASegment` / `WinGranularity` / `WinSize`, `PhysBasePtr`, MemoryModel
  packed-pixel (`04h`).
- **IBM / FreeVGA** — chain-4 256-color fetch already documented in
  `docs/vga-r3-graphics256-mode13h.md`.

## The honest "linear" surface

Mode `13h` (`VBE_MODE_13H_CHAIN4`) is packed-pixel banked VGA:

- Window A at segment `A000h`, 64 KiB granularity/size
- ModeAttributes: supported, color, graphics, VGA-compatible, **windowing
  available**, **LFB not available**
- `PhysBasePtr` = 0
- `render_mode` stays [`VgaRenderMode::Graphics256Chain4`] — no new enum
  variant (an exhaustive match in `emulator-cli` is outside this ownership
  area)

The host linear view is the existing `VgaFrame` from `render_frame`: one DAC
index per pixel, row-major. `VgaText::vbe_host_linear_framebuffer` is a named
alias that returns that frame only for the chain-4 programming, so callers
looking for a "VBE LFB-ish" buffer get the rendered pixels without a guest
aperture.

## Why not 640×480×8 or a PhysBasePtr LFB

- 640×480×8 needs 307,200 bytes; this model has 256 KiB of plane memory.
- A non-zero `PhysBasePtr` would invent a BAR / high-memory aperture the
  machine does not decode.

Banked mode-info for the renderable VGA modes is the truthful bound.

## Still unsupported

- Guest bank switching (AH=4Fh AL=05h) and WinFuncPtr
- Bochs/QEMU-style VBE LFB hardware
- Extending `Graphics256Chain4` geometry beyond the fixed 320×200 model choice
- INT 10h mode set
