# VBE AX=4F02h set-mode stub without LFB (R14 display-fw)

Milestone 2, Round 14, display-fw lane — slice 3.

## Goal

Honest host INT 10h AX=4F02h for modes this model can already program, **without**
claiming a guest linear framebuffer.

## Honesty contract

| Condition | Result |
|-----------|--------|
| BX bit14 (LFB) set | AX=`014Fh` — no guest LFB / PhysBasePtr |
| BX mode = `03h` | Program text via AH=00h path; AX=`004Fh` |
| BX mode = `13h` | Program mode 13h via AH=00h path; AX=`004Fh` |
| Listed planar `0Dh`/`0Eh`/`10h`/`12h` | AX=`014Fh` (info-only today; no programmer) |
| Unknown / hi-res mode | AX=`014Fh` |

`VgaText::guest_lfb_available()` stays `false`. ModeAttributes D7 and
`PhysBasePtr` remain zero on subsequent 4F01h queries.

BX bit15 (don't clear) is accepted but clear behavior still follows the AH=00h
helpers (honest subset note).

## Spec refs

- VESA BIOS Extension (VBE) Core Functions Standard Version 3.0 — Function 02h
  Set VBE Mode (BX mode bits, bit14 LFB, bit15 don't-clear). Compatible with
  VBE 2.0 Function 02h register contract used by RBIL.
- Ralf Brown's Interrupt List — INT 10h AX=4F02h.
- Prior honesty: `docs/vga-r9-physbaseptr-honesty.md`,
  `docs/vga-r13-vbe-4f01-mode-info.md`.

## Still unsupported

- Guest-mappable LFB / non-zero PhysBasePtr
- Programming planar VBE-listed modes `0Dh`/`0Eh`/`10h`/`12h`
- VESA hi-res modes (`101h`, …)
- SeaVGABIOS binary execution of VBE

## Files

- `crates/machine-pc/src/int10.rs`
- `docs/vga-r14-vbe-4f02-set-mode.md` — this note
