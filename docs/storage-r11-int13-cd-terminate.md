# R11 INT 13h AH=4Ah / AH=4Bh El Torito terminate + status

Milestone 2, round 11, boot-guest lane, slice 1.

## Scope

Align host CD/El Torito INT 13h with El Torito 1.0 §§6.1–6.2 / RBIL:

| AH / AL | Function |
|---|---|
| `4Ah` / `AL=00h` | Initiate disk emulation — **unsupported** (`CF` + `AH=01h`) |
| `4Bh` / `AL=00h` | Terminate disk emulation + fill 19-byte specification packet |
| `4Bh` / `AL=01h` | Get status only (fill packet; do not terminate) |
| `DL=7Fh` | AH=4Bh “all emulations” accepted (same no-emul path) |

**R10 note:** R10 treated `AL=00h` as get-status and rejected terminate. R11
corrects AL per El Torito/RBIL; get-status is `AL=01h`.

## Honesty / unsupported

- Host dispatcher only — not SeaBIOS / not a guest IVT body.
- No floppy/HDD El Torito emulation state exists; terminate is a **no-op** that
  still returns the no-emul CD status packet (`CF` clear).
- Initiate (`AH=4Ah`) always fails — no emulation start.
- AH=4Ch / 4Dh remain out.

## Spec

"El Torito" Bootable CD-ROM Format Specification 1.0 §§6.1–6.2; Ralf Brown's
Interrupt List INT 13h AH=4Ah / AX=4B00h / AX=4B01h.
