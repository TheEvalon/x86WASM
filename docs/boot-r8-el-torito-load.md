# R8 El Torito no-emulation load to 0x7C00

Milestone 2, round 8, boot-guest lane, slice 3.

## Scope

Restore host-side El Torito **no-emulation** boot-image handoff that R7 merge
kept inspect-only:

| Entry | Role |
|---|---|
| `Machine::inspect_atapi_el_torito` | Catalog validation (unchanged) |
| `Machine::load_eltorito_to_7c00` | Copy boot image → load segment; set `CS:IP` |

Default load segment `07C0h` → phys `0x7C00`. Sector count is El Torito's
512-byte virtual sectors; source bytes come from the attached ATAPI ISO image
at `load_rba` (2048-byte Mode-1 LBAs).

## Honesty

- **Not** a guest CD BIOS / INT 13h CD emulation.
- **Not** floppy or HDD emulation media types (`01h`–`04h`).
- **Not** multi-section catalogs / EFI platform section walking.
- Does **not** claim SeaBIOS CD boot or Milestone 2 exit.

## Spec

"El Torito" Bootable CD-ROM Format Specification Version 1.0 — Boot Record,
Validation Entry, Initial/Default Entry (media type `00h`, load RBA, sector
count, load segment).
