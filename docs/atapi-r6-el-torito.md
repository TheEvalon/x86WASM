# El Torito host-side detection and no-emul handoff

Milestone 2, round 6 (inspect) + round 8 (load restore). CD boot remains an
open Milestone 2 exit item; this documents **host-side** catalog validation and
a bounded no-emulation load to the load segment (default phys `0x7C00`).

## Approved sources used here

- **"El Torito" Bootable CD-ROM Format Specification Version 1.0** — Boot Record
  Volume Descriptor (`EL TORITO SPECIFICATION`), Validation Entry (header `01h`,
  platform ID, word checksum 0, key bytes `55h`/`AAh`), Initial/Default Entry
  (boot indicator `88h`, media type, sector count, load RBA).
- ISO 9660 Volume Descriptor layout at LBA 16+ (`CD001`).

## API

| Entry | Role |
|---|---|
| `firmware_interface::parse_el_torito` | Pure parser over a raw 2048-byte-sector image |
| `IdePrimary::atapi_medium_image` | Borrow attached CD bytes for host helpers |
| `Machine::inspect_atapi_el_torito` | Parse the primary ATAPI medium |
| `Machine::load_eltorito_to_7c00` | No-emul image copy + `CS:IP` handoff |

## Still unsupported

- INT 13h CD emulation / guest CD BIOS services
- Floppy/HDD emulation media types
- Multi-section catalogs / EFI platform `EFh` section headers (platform ID is
  reported; section walking is not)
- SeaBIOS CD boot path
