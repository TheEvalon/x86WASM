# R10 INT 13h CD / El Torito path

Milestone 2, round 10, boot-guest lane, slice 2.

## Scope

Host-side IBM/MS INT 13h Extensions + El Torito CD subset against the attached
ATAPI Mode-1 medium (`DL = E0h`):

| AH | Function |
|---|---|
| `41h` | Check extensions (`BX=55AAh` → `BX=AA55h`, packet+EDD bits) |
| `42h` | Extended read via Disk Address Packet — **2048-byte** Mode-1 LBAs |
| `48h` | Extended get drive parameters (sector size `2048`, linear geometry) |
| `4Bh` / `AL=00h` | El Torito get disk-emulation status (19-byte specification packet) |

Complements host `Machine::load_eltorito_to_7c00` (boot-image handoff) with a
guest-callable **host** INT 13h CD path for reading the ISO after attach.

## API

| Entry | Role |
|---|---|
| `Machine::service_int13_cd` | CD-only dispatch (`DL=E0h`) |
| `Machine::service_int13` | Routes `E0h` → CD |
| `Machine::int13_cd_read_lba_to_phys` | Explicit 2048-byte LBA → phys |
| `setup_int13_cd_*` | Harness register / DAP helpers |

## Honesty / unsupported

- **Not** SeaBIOS and **not** a guest IVT BIOS body.
- AH=42h `count` is Mode-1 **2048-byte** blocks (not 512).
- AH=4Bh `AL=01h` terminate-emulation — rejected (`AH=01h` / CF).
- Floppy/HDD El Torito emulation media types (`01h`–`04h`) — out.
- AH=43h CD write, AH=4A/4C/4D — out.
- Does **not** claim Milestone 2 CD boot exit.

## Spec

IBM/Microsoft INT 13h Extensions (AH=41h/42h/48h); "El Torito" Bootable CD-ROM
Format Specification 1.0 + RBIL INT 13h AH=4Bh; SFF-8020i Mode-1 2048-byte
blocks via existing ATAPI medium (`docs/atapi-r5-cdrom-medium.md`).
