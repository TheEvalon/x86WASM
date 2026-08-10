# VBE AX=4F01h mode-info honesty (R13 display-fw)

Milestone 2, Round 13, display-fw lane — slice 4.

## Goal

Deliver host INT 10h AX=4F01h from the existing truthful
`VgaText::vbe_mode_info_block_bytes` helper **without** claiming a guest linear
framebuffer.

## Honesty contract

| Field | Value |
|-------|-------|
| ModeAttributes D7 | clear (no LFB) |
| PhysBasePtr (offset 40) | `VBE_PHYS_BASE_PTR_NONE` (`0`) |
| OffScreenMemOffset (44) | `VBE_OFFSCREEN_MEM_OFFSET_NONE` (`0`) |
| OffScreenMemSize (48) | `VBE_OFFSCREEN_MEM_SIZE_NONE` (`0`) |
| WinFuncPtr | `0` (no bank-switch callable) |

## API

| Surface | Behavior |
|---------|----------|
| AX=4F01h | CX=mode → copy 256-byte `ModeInfoBlock` to `ES:DI`; AX=`004Fh` |
| Unknown mode | AX=`014Fh` (no memory write) |
| Other 4Fxx (e.g. 4F02h) | AX=`014Fh` |

## Spec refs

- VESA BIOS Extension (VBE) Core Functions Standard Version 2.0 — Function 01h
  `ModeInfoBlock`, ModeAttributes, PhysBasePtr, OffScreenMem*.
- Ralf Brown's Interrupt List — INT 10h AX=4F01h.
- Prior honesty: `docs/vga-r5-vbe-info-blocks.md`, `docs/vga-r9-physbaseptr-honesty.md`,
  `docs/vga-r12-vbe-4f00-info.md`.

## Still unsupported

- AX=4F02h+ mode set / bank / LFB
- Guest-mappable LFB / non-zero PhysBasePtr
- VESA hi-res modes (`101h`, …)
- SeaVGABIOS binary execution of VBE

## Files

- `crates/devices/src/vga.rs`
- `crates/machine-pc/src/int10.rs`
- `crates/devices/tests/vga_vbe_info_blocks.rs`
- `docs/vga-r13-vbe-4f01-mode-info.md` — this note
