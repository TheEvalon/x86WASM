# R9 INT 13h Floppy (AH=02h / AH=03h)

Milestone 2, round 9, boot-guest lane, slice 2.

## Scope

Host-side IBM BIOS INT 13h **floppy** read/write against attached FDC 1.44MB
media (`DL = 00h`), for FreeDOS floppy-boot measure prep:

| AH | Function |
|---|---|
| `00h` | Reset (media present → success) |
| `02h` | Read sectors into `ES:BX` (CHS) |
| `03h` | Write sectors from `ES:BX` (CHS) |

Geometry is fixed 80 cylinders / 2 heads / 18 SPT (IBM 1.44MB). Multi-sector
transfers advance sector → head → cylinder. Media write-protect returns
`AH=03h` / CF set.

## API

| Entry | Role |
|---|---|
| `Machine::service_int13_floppy` | Floppy-only dispatch |
| `Machine::service_int13` | Route by `DL` (`00h` / `80h`) |
| `Machine::int13_floppy_read_chs_to_phys` | Explicit CHS → phys |
| `Machine::int13_floppy_write_chs_from_phys` | Explicit phys → CHS |
| `setup_int13_floppy_read` / `setup_int13_floppy_write` | Harness setup |

Uses existing `Fdc82077::read_sector` / `write_sector` only (does not edit
`fdc.rs`).

## Honesty / unsupported

- **Not** a guest IVT BIOS body.
- AH=08h floppy get-params, format, verify, extensions — out.
- Second floppy (`DL=01h`) — rejected.
- Complements R7 host `load_floppy_boot_to_7c00` (boot-sector handoff only).

## Spec

IBM PC BIOS INT 13h Disk Services — floppy AH=02h/03h; Ralf Brown's Interrupt
List; 1.44MB CHS layout matching Intel 82077AA / IBM PC floppy media.
