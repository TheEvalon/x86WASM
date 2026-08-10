# R7 Floppy boot sector → `0x7C00`

Milestone 2, round 7, storage/guest lane, slice 2.

## Scope

Explicit host-side floppy VBR handoff that does **not** prefer IDE:

| Entry | Role |
|---|---|
| `Machine::load_mbr_to_7c00` | IDE LBA0 prefer, else floppy CHS `(0,0,1)` |
| `Machine::load_floppy_boot_to_7c00` | **Always** floppy CHS `(0,0,1)` → phys `0x7C00` |

Both require classic `0x55AA` and set `CS:IP = 0000:7C00`.

## Spec

IBM PC BIOS INT 19h floppy boot path / OSDev Boot Sequence.

## Still unsupported

- Guest INT 13h floppy AH=08h / format / extensions (AH=02/03 host path is R9 —
  see `docs/storage-r9-int13-floppy.md`)
- Multi-sector floppy loader / BPB parse beyond INT 13h AH=02h
- Boot-order policy beyond these two host helpers
