# R8 INT 13h drive params / Extensions (AH=08h / 41h / 48h)

Milestone 2, round 8, boot-guest lane, slice 2.

## Scope

Deepen classic drive parameters and advertise an honest IBM/Phoenix Extensions
subset against primary IDE (`DL = 80h`):

| AH | Function |
|---|---|
| `08h` | Get drive parameters (16 heads / 63 spt; `AL=0`, `BL=0`) |
| `41h` | Check extensions (`BX=55AAh` → `BX=AA55h`; `CX` bits 0+2) |
| `48h` | Extended get drive parameters (Phoenix EDD v1.x 26-byte buffer) |

Also present (bonus from the same host dispatcher): `AH=42h` extended read via
classic Disk Address Packet — see `docs/storage-r8-int13-extensions.md`.

## AH=48h buffer (minimum `1Ah`)

| Offset | Field |
|---|---|
| `00h` | WORD size written back (`1Ah`) |
| `02h` | WORD info flags (geometry-valid bit 1) |
| `04h` | DWORD cylinder count |
| `08h` | DWORD heads (`16`) |
| `0Ch` | DWORD sectors/track (`63`) |
| `10h` | QWORD total sectors (from IDE image) |
| `18h` | WORD bytes/sector (`512`) |

## Honesty / unsupported

- **Not** a guest IVT BIOS body.
- Removable locking (`CX` bit 1), EDD 2.0/3.0 device-path / DPI fields —
  unsupported / not advertised.
- **AH=43h** extended write is present on the R9 tip (see
  `docs/storage-r9-int13-ext-write.md`); this R8 note originally deferred it.
- Geometry is the fixed IDENTIFY obsolete 16/63 mapping, not translation modes.

## Spec

IBM PC BIOS INT 13h AH=08h; IBM/Microsoft INT 13h Extensions / Phoenix EDD
AH=41h/48h (RBIL); ATA IDENTIFY obsolete CHS.
