# Host INT 10h stub (R9 display-fw)

Milestone 2, Round 9, display-fw lane — slice 2.

## Goal

Provide a **minimal host-installed** INT 10h path for bring-up text output:
AH=00h set mode and AH=0Eh teletype. This is not SeaVGABIOS and not a full
VGA BIOS.

## API

| Helper | Role |
|--------|------|
| `Machine::install_int10_ivt_pointer(seg, off)` | Write IVT\[10h\] far pointer |
| `Machine::service_int10` | Dispatch AH from CPU registers |
| `setup_int10_set_mode` / `setup_int10_teletype` | Test/harness register loads |

### AH=00h SET VIDEO MODE (RBIL)

| AL | Behavior |
|----|----------|
| `03h` | `VgaText::reset` (mode-03h text) + BDA update |
| `13h` | `VgaText::program_bios_mode13h` + BDA update |
| other | No-op (unsupported) |

BDA fields written: `0040:0049` mode, `0040:004A` columns, `0040:0050`
cursor (0,0), `0040:0062` page 0.

### AH=0Eh TELETYPE OUTPUT (RBIL)

Text mode only, display page 0:

- Printable: write at BDA cursor with current cell attribute (default `07h`),
  advance; wrap to next row (no scroll).
- `0Dh` CR, `0Ah` LF, `08h` BS (move only), `07h` bell (no-op).

Graphics teletype, multi-page, scroll, and cursor shape are out of scope.

## Spec refs

- Ralf Brown's Interrupt List — INT 10h AH=00h, AH=0Eh; BIOS Data Area video
  bytes at `0040:0049` / `4A` / `50` / `62`.
- IBM VGA / FreeVGA — mode 03h reset defaults; mode 13h register signature
  (`program_bios_mode13h`).

## Still unsupported

- Full INT 10h surface (scroll, cursor, write string, VBE `4Fxx`)
- Guest-driven BIOS body at the IVT target (pointer only; host calls
  `service_int10`)
- SeaVGABIOS / option-ROM installed INT 10h

## Files

- `crates/machine-pc/src/int10.rs`
- `crates/devices/src/vga.rs` — `program_bios_mode13h` (shared with AH=00h)
- `docs/vga-r9-int10-stub.md` — this note
