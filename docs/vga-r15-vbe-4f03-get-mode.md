# VBE AX=4F03h get-mode / 4F0A honesty (R15 display-fw)

Milestone 2, Round 15, display-fw lane — slice 4.

## Goal

Honest host INT 10h AX=4F03h Return Current VBE Mode for modes programmed via
AH=00h or AX=4F02h — **without** claiming a guest linear framebuffer. Mode-list
honesty remains on AX=4F00h; AX=4F0Ah PMI fails explicitly.

## Honesty contract

| Condition | Result |
|-----------|--------|
| After AH=00h / 4F02h mode `03h` | AX=`004Fh`, BX=`0003h` |
| After AH=00h / 4F02h mode `13h` | AX=`004Fh`, BX=`0013h` |
| BX bit14 (LFB) | **Never** set — tracked mode strips LFB |
| Failed 4F02h (LFB / unprogrammable) | Current mode unchanged |
| AX=4F0Ah PMI | AX=`014Fh` — no protected-mode interface |
| Mode list | Still only via AX=4F00h `VideoModePtr` (`docs/vga-r12-vbe-4f00-info.md`) |

`VgaText::vbe_current_mode()` records successful AH=00h / 4F02h programs.
`guest_lfb_available()` stays `false`; `PhysBasePtr` stays zero.

## Spec refs

- VESA BIOS Extension (VBE) Core Functions Standard Version 2.0/3.0 —
  Function 03h Return Current VBE Mode; Function 0Ah PMI.
- Ralf Brown's Interrupt List — INT 10h AX=4F03h / AX=4F0Ah.
- Prior: `docs/vga-r14-vbe-4f02-set-mode.md`, `docs/vga-r9-physbaseptr-honesty.md`.

## Still unsupported

- Returning LFB / don't-clear sticky flags from a prior request
- VESA hi-res current modes / SeaVGABIOS VBE body
- Guest-callable PMI entry point

## Files

- `crates/devices/src/vga.rs` — `vbe_current_mode` / `set_vbe_current_mode`
- `crates/machine-pc/src/int10.rs`
- `docs/vga-r15-vbe-4f03-get-mode.md` — this note
