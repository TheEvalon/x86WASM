# R15 INT 13h AH=02h CHS read deepen (FreeDOS path)

Milestone 2, round 15, boot-guest lane, slice 3.

## Scope

Solidify the host INT 13h **AH=02h** CHS read path used by FreeDOS-path
measure / FAT follow-on:

| Piece | Role |
|---|---|
| `int13_hd_ok_al` / `int13_hd_fail` | Mirror status to BDA `0040:0074` |
| AH=02h success | CF clear, `AH=00h`, `AL`=count, BDA=`00h` |
| AH=02h failure | CF set, `AH`=status, `AL=0`, BDA=status |
| FreeDOS FAT12 VBR CHS | LBA1 = CHS `(0,0,2)` under fixed 16H/63S |
| Multi-sector | Consecutive LBAs (VBR+FAT) for FreeDOS-style reads |

## Geometry note

Host HD AH=02h uses fixed **16 heads / 63 SPT** (IDENTIFY obsolete words).
Active partition at LBA1 is therefore CHS `(0,0,2)`, not `(0,1,1)`.

## Honesty

- Host `service_int13_hd` is **not** a guest IVT BIOS body (SeaBIOS still needed).
- Does not implement CHS translation modes or EDD for AH=02h.
- Track wrap beyond a single AH=02h call remains consecutive-LBA (BIOS classic).

## Spec

IBM PC BIOS INT 13h AH=02h; RBIL BDA `0040:0074`; ATA IDENTIFY geometry;
`docs/boot-r15-freedos-fat12.md`, `docs/storage-r9-int13-floppy.md` (floppy peer).
