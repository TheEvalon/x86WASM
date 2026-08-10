# El Torito host-side detection

Milestone 2, round 6, slice 4. CD boot remains an open Milestone 2 exit item;
this slice adds **host-side** validation of an attached ATAPI/ISO image without
INT 13h CD emulation or executing a boot image.

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

## Still unsupported

- INT 13h CD emulation / booting the load RBA
- Multi-section catalogs / EFI platform `EFh` section headers (platform ID is
  reported; section walking is not)
- SeaVGABIOS option-ROM execution (map path already exists from earlier rounds)
