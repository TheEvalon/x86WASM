# R15 INT 13h AH=02h multi-sector CHS deepen

Milestone 2, round 15, storage-int13 lane, slice 1.

## Scope

Deepen host INT 13h **AH=02h** multi-sector reads with media attached:

| Drive | Edge case | Behavior |
|---|---|---|
| HD `DL=80h` | Start at SPT, `AL≥2` | Consecutive LBA crosses to next head |
| HD `DL=80h` | Last head SPT, `AL≥2` | Consecutive LBA crosses to next cylinder |
| Floppy `DL=00h` | Sector SPT, `AL≥2` | CHS advance crosses head |
| Floppy `DL=00h` | Last head SPT, `AL≥2` | CHS advance crosses cylinder |
| Floppy `DL=00h` | Past last media sector | `CF` + `AH=04h`, `AL=0`, **no** partial buffer fill |

Floppy multi-sector now **preflights** the sector→head→cylinder walk so a
past-end request fails atomically (matching HD's all-or-nothing LBA bounds).

## Honesty / unsupported

- Host subset only — not a guest IVT BIOS body / SeaBIOS.
- HD track wrap is LBA-consecutive (IDENTIFY 16/63), not BIOS "same-track only".
- Partial-success `AL` (sectors transferred before error) remains out; failures
  clear `AL` to 0.
- Second floppy (`DL=01h`) still invalid.

## Spec

IBM PC BIOS INT 13h Disk Services AH=02h; Ralf Brown's Interrupt List INT 13h
AH=02h; `docs/storage-r7-int13-hd.md`, `docs/storage-r9-int13-floppy.md`.
