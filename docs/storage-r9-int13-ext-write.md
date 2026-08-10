# R9 INT 13h Extended Write (AH=43h)

Milestone 2, round 9, boot-guest lane, slice 1.

## Scope

Host-side IBM/Microsoft INT 13h Extensions **extended write** against primary
IDE, mirroring the R8 AH=42h Disk Address Packet read path:

| AH | Function |
|---|---|
| `43h` | Extended write via classic 16-byte Disk Address Packet (`DS:SI`) |

Drive `DL = 80h` only. Packet form identical to AH=42h: size ≥ `10h`, real-mode
`seg:off` buffer, 64-bit LBA at offset 8. `AL` write flags are accepted;
verify-after-write (`AL` bit 0) is **ignored** (write-only; no post-write
readback).

## API

| Entry | Role |
|---|---|
| `Machine::service_int13_hd` | Dispatch includes AH=43h |
| `Machine::int13_hd_write_lba_from_phys` | Explicit LBA ← phys copy into IDE image |
| `setup_int13_hd_ext_write` | Harness DAP + register setup |

AH=41h still advertises `CX` bit 0 (packet access) only; that bit now covers
both AH=42h and AH=43h in this subset.

## Honesty / unsupported

- **Not** a guest IVT BIOS body (SeaBIOS still required for real `INT 13h`).
- Verify-after-write, flat 64-bit transfer buffers, floppy/CD extensions — out.
- Writes mutate the in-memory IDE image only (no host file persistence).

## Spec

IBM/Microsoft INT 13h Extensions / Ralf Brown's Interrupt List — INT 13h AH=43h
Extended Write; Disk Address Packet layout shared with AH=42h.
