# VBE AX=4F00h controller info deepen (R12 display-fw)

Milestone 2, Round 12, display-fw lane — slice 4.

## Goal

Deepen the host VBE 2.0 `VbeInfoBlock` and deliver it through host INT 10h
AX=4F00h **without** claiming a guest linear framebuffer.

## Honesty contract

| Field | Value |
|-------|-------|
| Capabilities | [`VBE_CAPABILITIES_NONE`] (`0`) — no 8-bit DAC, not non-VGA, no blanking |
| ModeAttributes D7 | clear on every mode (no LFB) |
| PhysBasePtr | [`VBE_PHYS_BASE_PTR_NONE`] (`0`) |
| VideoModePtr (host helper) | offset [`VBE_VIDEO_MODE_LIST_HOST_OFFSET`], segment `0` (embedded) |
| VideoModePtr (AX=4F00) | rewritten to `ES:(DI+mode_list_offset)` |
| OemStringPtr | `"x86WASM host VGA"` NUL string; guest delivery uses `ES:DI` too |

This is **not** SeaVGABIOS and **not** a claim that guests can mmap an LFB.

## API

| Surface | Behavior |
|---------|----------|
| `VgaText::vbe_info_block_bytes` | Host-embedded block (seg 0 offsets) |
| `VgaText::vbe_info_block_bytes_for_guest(es, di)` | Far pointers rewritten for `ES:DI` |
| `VgaText::vbe_capabilities` | Always `VBE_CAPABILITIES_NONE` |
| Host INT 10h AX=4F00h | Copy guest block to `ES:DI`; AX=`004Fh` |
| Other AX=4Fxxh | AX=`014Fh` (unsupported subfunction) |

## Spec refs

- VESA BIOS Extension (VBE) Core Functions Standard Version 2.0 — Function 00h
  `VbeInfoBlock`, Capabilities, VideoModePtr / OemStringPtr far pointers.
- Ralf Brown's Interrupt List — INT 10h AX=4F00h.
- Prior honesty notes: `docs/vga-r5-vbe-info-blocks.md`,
  `docs/vga-r9-physbaseptr-honesty.md`.

## Still unsupported

- AX=4F01h mode info delivery (device helper exists; no INT 10h yet)
- AX=4F02h+ mode set / bank / LFB
- Guest-mappable LFB / non-zero PhysBasePtr
- SeaVGABIOS binary execution of VBE

## Firmware note

SeaVGABIOS build scripts under `firmware/` are unchanged this slice. Host
AX=4F00h is a bring-up stub so guests can probe controller info before a real
option ROM is mapped and executed.

## Files

- `crates/devices/src/vga.rs`
- `crates/machine-pc/src/int10.rs`
- `crates/devices/tests/vga_vbe_info_blocks.rs`
- `docs/vga-r12-vbe-4f00-info.md` — this note
- `docs/firmware-r12-vbe-host-stub.md` — firmware-facing summary
