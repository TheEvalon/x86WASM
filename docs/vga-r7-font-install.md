# VGA bring-up font install (R7 display/boot)

Milestone 2, campaign R7, display/boot lane — slice 3.

## Decision (unchanged from R4)

**Reset still installs no font.** Glyphs are guest / video-BIOS state
(`docs/vga-r4-font-provenance.md`). `VgaFrame::font_installed == Some(false)`
after reset remains the honest report.

## What this slice adds

A **host bring-up path** that installs a *procedural marker font* into map 2
bank `0000h` so text mode can light pixels and `font_installed` becomes
`Some(true)` without vendoring IBM/CP437 or GPL font data.

| API | Role |
|-----|------|
| `vga_bringup_font_glyphs()` | 256 × 16 packed scan lines (space blank; others boxed + identity row) |
| `VgaText::install_bringup_font()` | `install_font_bank(0, 16, …)` |
| `Machine::install_vga_bringup_font()` | machine wrapper |

## Spec refs (storage layout only)

- FreeVGA "VGA Text Mode Operation", Fonts — plane 2, `code * 32` stride,
  8 KiB banks, Character Map Select.
- IBM PS/2 Video Subsystems — character generator consumes map 2; real PCs
  load fonts from the video BIOS, not from the CRTC device model at reset.

## Licensing

- No `third_party/NOTICE` addition: bytes are generated in-tree, not copied.
- Not a substitute for SeaVGABIOS font load after option-ROM execute.
- Front ends that need real CP437 must supply an entitled font via
  `install_font_bank` and record provenance separately.

## Unsupported

- CRTC-derived glyph height / 9-dot mode changes to this buffer
- Automatic install on reset or on option-ROM return
- Claiming visual fidelity to IBM VGA ROM glyphs
