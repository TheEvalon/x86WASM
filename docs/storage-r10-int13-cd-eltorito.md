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
| `4Bh` | El Torito status / terminate (see R11 for AL semantics) |

Complements host `Machine::load_eltorito_to_7c00` (boot-image handoff) with a
guest-callable **host** INT 13h CD path for reading the ISO after attach.

**Superseded AL note:** R10 treated `AH=4Bh AL=00h` as get-status. R11 aligns
with El Torito/RBIL (`AL=00h` terminate, `AL=01h` status-only) —
`docs/storage-r11-int13-cd-terminate.md`.

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
- Floppy/HDD El Torito emulation media types (`01h`–`04h`) — out.
- AH=43h CD write, AH=4C/4D — out (AH=4Ah initiate rejected in R11).
- Does **not** claim Milestone 2 CD boot exit.

## Spec

IBM/Microsoft INT 13h Extensions (AH=41h/42h/48h); "El Torito" Bootable CD-ROM
Format Specification 1.0 + RBIL INT 13h AH=4Bh; SFF-8020i Mode-1 2048-byte
blocks via existing ATAPI medium (`docs/atapi-r5-cdrom-medium.md`).
