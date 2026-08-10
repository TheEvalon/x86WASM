# El Torito host-side detection and `0x7C00` handoff

Milestone 2, round 6, slice 4. Continues the CD-ROM path with host-side
catalog validation **and** a bounded no-emulation load toward phys `0x7C00`.
INT 13h CD emulation and SeaBIOS CD boot remain out of scope.

## Approved sources used here

- **"El Torito" Bootable CD-ROM Format Specification Version 1.0** — Boot Record
  Volume Descriptor (`EL TORITO SPECIFICATION`), Validation Entry (header `01h`,
  platform ID, word checksum 0, key bytes `55h`/`AAh`), Initial/Default Entry
  (boot indicator `88h`, media type, load segment, sector count, load RBA).
- ISO 9660 Volume Descriptor layout at LBA 16+ (`CD001`).

## API

| Entry | Role |
|---|---|
| `firmware_interface::parse_el_torito` | Pure parser over a raw 2048-byte-sector image |
| `IdePrimary::atapi_medium_image` | Borrow attached CD bytes for host helpers |
| `Machine::inspect_atapi_el_torito` | Parse the primary ATAPI medium |
| `Machine::load_eltorito_to_7c00` | No-emulation copy to load segment + `CS:IP` handoff |

`load_eltorito_to_7c00` accepts only media type `00h` (no emulation). A zero
load segment resolves to `07C0h` (phys `0x7C00`). Sector count is in 512-byte
virtual sectors; the bytes are taken from `load_rba` on the 2048-byte CD image.

## Still unsupported

- Floppy / hard-disk emulation media types
- INT 13h CD extensions / SeaBIOS CD boot path
- Multi-section catalogs / EFI platform `EFh` section headers (platform ID is
  reported; section walking is not)
- SeaVGABIOS option-ROM execution (map path already exists from earlier rounds)
