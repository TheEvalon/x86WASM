# R12 INT 13h AH=04h VERIFY

Milestone 2, round 12, boot-guest lane, slice 1.

## Scope

Host-side IBM BIOS / RBIL INT 13h **AH=04h verify sectors** against attached media:

| Drive | Path |
|---|---|
| `DL=80h` | Primary IDE image — CHS → LBA range must lie within the image |
| `DL=00h` | FDC 1.44MB — walks sectors via `Fdc82077::read_sector` |

Same CHS packing as AH=02h. Success clears `CF`, sets `AH=00h`, `AL` = sectors verified.
Errors use the existing subset: `AH=01h` invalid, `AH=04h` sector not found, `AH=80h` no media.

**No data transfer** to `ES:BX` (buffer unused).

## API

| Entry | Role |
|---|---|
| `Machine::service_int13_hd` / `service_int13_floppy` | Dispatch includes AH=04h |
| `Machine::int13_hd_verify_chs` | Explicit HD verify helper |
| `Machine::int13_floppy_verify_chs` | Explicit floppy verify helper |
| `setup_int13_hd_verify` / `setup_int13_floppy_verify` | Test harness register setup |

## Honesty / unsupported

- **Not** a guest IVT BIOS body (SeaBIOS still required for real `INT 13h`).
- Extended verify (**AH=44h**) remains unsupported.
- CD `DL=E0h` AH=04h not implemented (CD uses packet LBA AH=42h only).
- No ECC / soft-error retry modeling — presence in image ⇒ verify OK.

## Spec

IBM PC BIOS INT 13h Disk Services AH=04h; Ralf Brown's Interrupt List INT 13h AH=04h;
`docs/sources.md` INT 13h entry.
