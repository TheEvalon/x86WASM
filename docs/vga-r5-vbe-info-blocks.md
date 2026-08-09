# VBE 2.0 information blocks (host-side)

Milestone 2, round 5, slice 1. This tree can render three programmings — text
mode `03h`, chain-4 mode `13h`, and planar 16-color modes `0Dh`/`0Eh`/`10h`/
`12h`. This slice builds the VBE 2.0 `VbeInfoBlock` / `ModeInfoBlock` wire
images a video BIOS would hand to INT 10h AX=4F00h / 4F01h for **exactly those
modes**, with truthful capability bits. There is still no INT 10h hook and no
VGA BIOS in the guest path.

## Approved sources used here

- **VESA BIOS Extension (VBE) Core Functions Standard, Version 2.0** — Function
  00h Return VBE Controller Information (`VbeInfoBlock`), Function 01h Return
  VBE Mode Information (`ModeInfoBlock`), the Capabilities field, ModeAttributes
  bits (including D7 linear framebuffer), MemoryModel codes, and the mandatory
  VBE 2.0 `PhysBasePtr` field.
- **IBM PS/2 Hardware Interface Technical Reference — Video Subsystems** and
  **FreeVGA** — already cited for the register programmings those mode numbers
  name (`docs/vga-r3-*.md`, `docs/vga-r4-planar16-renderer.md`).

**Needs a `docs/sources.md` entry** for VBE 2.0 (ownership of this slice does
not edit that file).

## What is built

| Helper | Wire size | Contents |
|---|---|---|
| `VgaText::vbe_info_block_bytes` | 512 | `VBE2` signature, version `0200h`, Capabilities all clear, mode list of the six VGA modes above, `TotalMemory` = 4 (256 KiB / 64 KiB) |
| `VgaText::vbe_mode_info_block_bytes(mode)` | 256 | Banked `ModeInfoBlock` for one supported mode, or `None` |

Capabilities are all zero on purpose:

| Bit | Claim if set | Why clear |
|---|---|---|
| 0 | 8-bit DAC switchable | This DAC is 6-bit store/readback |
| 1 | non-VGA controller | The controller *is* VGA-compatible |
| 2 | programmatic blanking of the RAMDAC | Not modeled |

ModeAttributes never set D7 (linear framebuffer available) and leave D6 clear
(windowing / banking is available). `PhysBasePtr` is zero. Claiming an LFB
would invent a high physical aperture this VGA model does not have.

## Still unsupported

- INT 10h / VGA BIOS delivery of these blocks to a guest
- Any VESA high-resolution mode (`101h`, `103h`, …)
- Guest-mappable linear framebuffer (`PhysBasePtr` ≠ 0)
- VBE protected-mode interface, dual windows, or a window function pointer
  (WinFuncPtr is zero; bank switching is not implemented as a callable)
