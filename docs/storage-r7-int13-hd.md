# R7 INT 13h hard-disk host service

Milestone 2, round 7, storage/guest lane, slice 1.

## Scope

Host-side IBM BIOS INT 13h **hard-disk** subset against primary IDE:

| AH | Function |
|---|---|
| `00h` | Reset disk system |
| `02h` | Read sectors into `ES:BX` (CHS) |
| `08h` | Get drive parameters (fixed 16 heads / 63 spt) |

Drive `DL = 80h` only. Geometry matches `IdePrimary` IDENTIFY obsolete CHS
(words 3/6).

## API

| Entry | Role |
|---|---|
| `Machine::service_int13_hd` | Dispatch from current CPU registers |
| `Machine::int13_hd_read_chs_to_phys` | Explicit CHS → phys copy |
| `Machine::install_int13_ivt_pointer` | IVT `[0x13]` far pointer only (no BIOS body) |
| `setup_int13_hd_read` / `pack_cx` / `chs_to_lba` | Test / harness helpers |

## Honesty

- **Not** SeaBIOS INT 13h. Guest `INT 13h` still needs a real IVT handler.
- **Not** floppy INT 13h, IBM/MS extensions (`AH=42h`), or CHS translation modes.
- Closest in-tree approach: host media helpers like `load_mbr_to_7c00`.

## Spec

IBM PC BIOS INT 13h Disk Services; ATA IDENTIFY obsolete geometry.
