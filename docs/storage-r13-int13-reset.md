# R13 INT 13h AH=00h disk reset deepen

Milestone 2, round 13, boot-guest lane, slice 3.

## Scope

Deepen host INT 13h **AH=00h** for HD (`DL=80h`) and floppy (`DL=00h`):

| Drive | Behavior |
|---|---|
| HD | Require non-empty IDE image; `IdePrimary::reset` (preserve image, clear DRQ); clear BDA `0040:0074` |
| Floppy | Require media; `Fdc82077::reset` + release DOR reset; clear BDA `0040:0041` |

Empty / missing media → `CF` + `AH=80h` (same honesty as AH=08h).

## API

| Entry | Role |
|---|---|
| `BDA_HD_STATUS` / `BDA_FLOPPY_STATUS` | BDA offsets |
| `service_int13_hd` / `service_int13_floppy` | Existing AH=00h dispatch |

## Honesty

- Host stub only — not SeaBIOS INT 13h body.
- Does not model multi-drive `DL` broadcast reset beyond FD0/HD0.

## Spec

IBM PC BIOS INT 13h AH=00h; RBIL INT 13h AH=00h + BDA disk status bytes;
ATA/ATAPI-6 reset semantics via existing `IdePrimary::reset`.
