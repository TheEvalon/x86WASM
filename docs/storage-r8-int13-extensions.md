# R8 INT 13h Extensions (AH=41h / AH=42h)

Milestone 2, round 8, boot-guest lane, slice 2.

## Scope

Host-side IBM/Microsoft INT 13h Extensions subset against primary IDE:

| AH | Function |
|---|---|
| `41h` | Check extensions present (`BX=55AAh` → `BX=AA55h`, `CX` bit0) |
| `42h` | Extended read via classic 16-byte Disk Address Packet (`DS:SI`) |

Drive `DL = 80h` only. Packet form: size ≥ `10h`, real-mode `seg:off` buffer,
64-bit LBA at offset 8.

## API

| Entry | Role |
|---|---|
| `Machine::service_int13_hd` | Dispatch includes AH=41h/42h |
| `Machine::int13_hd_read_lba_to_phys` | Explicit LBA → phys copy |
| `setup_int13_hd_ext_read` | Harness DAP + register setup |

## Honesty / unsupported

- **Not** a guest IVT BIOS body (SeaBIOS still required for real `INT 13h`).
- **AH=43h** extended write — returns `AH=01h` / CF set (documented unsupported).
- **AH=47h/48h**, removable locking (`CX` bit1), EDD (`CX` bit2) — unsupported.
- Flat 64-bit transfer buffer (`FFFF:FFFF` + packet size ≥ `18h`) — rejected.
- Floppy / CD INT 13h extensions — out of scope.

`CX` advertises packet access bit 0 only so callers can discover AH=42h; that
does **not** imply AH=43h write is present.

## Spec

IBM/Microsoft INT 13h Extensions (Phoenix EDD / RBIL INT 13h AH=41h/42h);
Disk Address Packet layout per the Extensions specification.
