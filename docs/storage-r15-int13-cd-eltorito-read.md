# R15 INT 13h CD / El Torito sector-read deepen

Milestone 2, round 15, storage-int13 lane, slice 4.

## Scope

Deepen host CD INT 13h **AH=42h** (`DL=E0h`) against the attached ATAPI
Mode-1 medium for El Torito boot-media reads:

| Case | Behavior |
|---|---|
| Multi-block Mode-1 read | Consecutive 2048-byte LBAs; DAP count rewritten |
| Catalog LBA read | Validation keys + Initial/Default Entry RBA visible |
| Boot `load_rba` via AH=42h | Matches `Machine::load_eltorito_to_7c00` payload bytes |
| Past medium end | `CF` + `AH=04h`; DAP count `0`; BDA `0040:0074` mirrored |

Complements R10 CD path (`docs/storage-r10-int13-cd-eltorito.md`) with
bounded El Torito sector-read honesty used by firmware/guest boot probes.

## Honesty / unsupported

- Host subset only — not SeaBIOS CD stack / guest IVT body.
- AH=42h `count` is Mode-1 **2048-byte** blocks (El Torito `sector_count` is
  still in 512-byte units for AH=4Bh / `load_eltorito_to_7c00`).
- Floppy/HDD El Torito emulation (`media_type` 01h–04h) — out.
- AH=4Ah initiate remains rejected; no claim of Milestone 2 CD boot exit.

## Spec

IBM/Microsoft INT 13h Extensions AH=42h; "El Torito" Bootable CD-ROM Format
Specification 1.0; SFF-8020i Mode-1 2048-byte blocks;
`docs/storage-r10-int13-cd-eltorito.md`.
