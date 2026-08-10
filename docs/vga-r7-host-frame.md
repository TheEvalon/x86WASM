# Host VGA frame path (R7 display/boot)

Milestone 2, campaign R7, display/boot lane — slice 4.

## Choice: host frame, not guest LFB

This slice improves the **host** capture path for text and mode 13h. It does
**not** invent a guest linear framebuffer aperture or a non-zero VBE
`PhysBasePtr` — that would disagree with
`docs/vga-r5-vbe-banked-framebuffer.md` and the truthful mode-info blocks
(LFB bit clear, `PhysBasePtr = 0`).

## API

| Helper | Role |
|--------|------|
| `Machine::capture_vga_frame(blink)` | `Option<VgaFrame>` from `VgaText::render_frame` |
| `Machine::capture_vga_rgba8(blink)` | RGBA8 bytes via `frame_rgba8` |
| `Machine::capture_vga_host_frame(blink)` | [`HostVgaFrame`] { frame, rgba8 } |
| `HostVgaFrame::{mode,font_installed,width,height}` | Convenience accessors |

Unsupported programmings still return `None` (same as the device renderer).

## Spec refs

- FreeVGA text-mode display fetch; FreeVGA / IBM chain-4 256-color (mode 13h).
- VBE 2.0 ModeAttributes D7 / `PhysBasePtr` — remain honest (no LFB hardware).

## Still unsupported

- Guest-mappable LFB / high BAR decode
- Timing-accurate raster / dirty tracking / browser canvas (web crate)
- Automatic mode set / INT 10h
