# VBE PhysBasePtr honesty (R9 display-fw)

Milestone 2, Round 9, display-fw lane — slice 1.

## Goal

Keep guest-facing VBE mode info **truthful** about linear framebuffers: if this
model has no guest-mappable LFB aperture, ModeAttributes D7 stays clear and
`PhysBasePtr` stays zero. Do not invent a high BAR just to satisfy callers that
expect Bochs/QEMU VBE hardware.

## Decision

| Claim | Value | Why |
|---|---|---|
| `VgaText::guest_lfb_available` | `false` | No host-mapped guest LFB window exists |
| ModeAttributes D7 (`VBE_MODE_ATTR_LFB`) | clear | VBE 2.0 "linear framebuffer available" |
| `PhysBasePtr` (`VBE_PHYS_BASE_PTR_NONE`) | `0` | Mandatory VBE 2.0 field; zero = no aperture |
| Host linear view | `vbe_host_linear_framebuffer` / `capture_vga_host_frame` | Host-only RGBA/DAC-index path (R5/R7) |

R5 already advertised banked modes with D7 clear and `PhysBasePtr = 0`. R9
names the contract explicitly so later slices cannot reintroduce a silent lie.

## Spec refs

- **VESA BIOS Extension (VBE) Core Functions Standard, Version 2.0** —
  Function 01h `ModeInfoBlock`: ModeAttributes bit 7 (LFB), `PhysBasePtr` at
  offset 40.
- FreeVGA / IBM VGA — banked legacy apertures (`A0000` / `B8000`), not a VBE
  LFB BAR.

## Still unsupported

- Guest-mappable LFB / non-zero `PhysBasePtr`
- Bochs VBE DISPI / high-resolution VESA modes
- INT 10h AX=4F01h delivery of these blocks (host helpers only until a VGA BIOS
  or host INT 10h stub wires them)

## Files

- `crates/devices/src/vga.rs` — `guest_lfb_available`, `vbe_phys_base_ptr`,
  `VBE_PHYS_BASE_PTR_NONE`, `VBE_MODE_ATTR_LFB`
- `crates/devices/tests/vga_vbe_info_blocks.rs` — honesty assertions
- `docs/vga-r9-physbaseptr-honesty.md` — this note
