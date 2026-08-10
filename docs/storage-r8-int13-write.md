# R8 INT 13h hard-disk write (AH=03h)

Milestone 2, round 8, boot-guest lane, slice 1.

## Scope

Host-side IBM BIOS INT 13h **hard-disk write** against primary IDE, extending
the R7 AH=00/02/08 subset:

| AH | Function |
|---|---|
| `03h` | Write sectors from `ES:BX` (CHS) |

Drive `DL = 80h` only. Same fixed 16-head / 63-spt geometry as AH=02h.

## API

| Entry | Role |
|---|---|
| `Machine::service_int13_hd` | Dispatch includes AH=03h |
| `Machine::int13_hd_write_chs_from_phys` | Explicit CHS ← phys copy into IDE image |
| `setup_int13_hd_write` | Test / harness register setup |

## Honesty

- **Not** SeaBIOS INT 13h. Guest `INT 13h` still needs a real IVT handler.
- **Not** floppy write, IBM/MS extensions (`AH=43h`), verify (`AH=04h`), or
  format (`AH=05h`).
- Writes mutate the in-memory IDE image only (no host file persistence).

## Spec

IBM PC BIOS INT 13h Disk Services — AH=03h Write Disk Sectors; CHS packing
identical to AH=02h.
