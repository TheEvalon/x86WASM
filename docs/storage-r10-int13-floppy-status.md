# R10 INT 13h Floppy Status / Media Sense

Milestone 2, round 10, boot-guest lane, slice 1.

## Scope

Deepen host-side IBM BIOS INT 13h **floppy** status beyond R9 AH=02h/03h:

| AH | Function |
|---|---|
| `08h` | Get drive parameters — 1.44MB max CHS + `BL=04h` |
| `15h` | Get disk type / media sense |

| Entry | Role |
|---|---|
| `Machine::service_int13_floppy` | Dispatch includes AH=08h / AH=15h |
| `setup_int13_floppy_get_params` | Harness setup for AH=08h |
| `setup_int13_floppy_get_disk_type` | Harness setup for AH=15h |

Geometry is fixed 80 cylinders / 2 heads / 18 SPT. AH=08h returns max
cylinder `79`, max head `1`, SPT `18`, drive count `DL=1`, type `BL=04h`.

## Honesty / error codes

- **Not** a guest IVT BIOS body; callers must invoke the host API.
- AH=08h with **no media** → `AH=80h` / CF set (timeout), matching reset/read.
- AH=15h with media → `AH=02h` (change-line capable; 82077AA DIR DSKCHG), CF clear.
- AH=15h with **no media** → `AH=00h` (no such drive), CF clear (RBIL type code).
- Diskette parameter table (`ES:DI` on AH=08h) — **out of scope**.
- Second floppy (`DL=01h`) — rejected as invalid.

## Spec

IBM PC BIOS INT 13h Disk Services — AH=08h / AH=15h; Ralf Brown's Interrupt
List; CMOS floppy type `04h` = 1.44 MB 3½″; Intel 82077AA change-line.
