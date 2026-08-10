# R8 INT 13h Extensions (AH=41h / AH=42h / AH=48h)

Milestone 2, round 8, boot-guest lane, slice 2 (companion to
`docs/storage-r8-int13-ext.md`).

## Scope

Host-side IBM/Microsoft INT 13h Extensions subset against primary IDE:

| AH | Function |
|---|---|
| `41h` | Check extensions present (`BX=55AAh` → `BX=AA55h`, `CX` bit0+bit2) |
| `42h` | Extended read via classic 16-byte Disk Address Packet (`DS:SI`) |
| `48h` | Extended get drive parameters (26-byte EDD v1.x buffer at `DS:SI`) |

Drive `DL = 80h` only. Packet form: size ≥ `10h`, real-mode `seg:off` buffer,
64-bit LBA at offset 8.

## API

| Entry | Role |
|---|---|
| `Machine::service_int13_hd` | Dispatch includes AH=41h/42h/48h |
| `Machine::int13_hd_read_lba_to_phys` | Explicit LBA → phys copy |
| `setup_int13_hd_ext_read` | Harness DAP + register setup |
| `setup_int13_hd_ext_get_params` | Harness AH=48h buffer + registers |

## Honesty / unsupported

- **Not** a guest IVT BIOS body (SeaBIOS still required for real `INT 13h`).
- **AH=43h** extended write — implemented in round 9; see
  `docs/storage-r9-int13-ext-write.md`.
- Removable locking (`CX` bit1), EDD 2.0/3.0 path/DPI — unsupported.
- Flat 64-bit transfer buffer (`FFFF:FFFF` + packet size ≥ `18h`) — rejected.
- Floppy / CD INT 13h extensions — out of scope for this R8 note (floppy CHS
  AH=02/03 added in R9).

`CX` advertises packet access bit 0 and EDD bit 2 so callers can discover
AH=42h/43h/48h. Removable locking remains unsupported.

## Spec

IBM/Microsoft INT 13h Extensions (Phoenix EDD / RBIL INT 13h AH=41h/42h/48h);
Disk Address Packet layout per the Extensions specification.
