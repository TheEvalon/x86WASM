# R11 INT 13h AH=08h HD CHS deepen

Milestone 2, round 11, boot-guest lane, slice 2.

## Scope

Deepen host INT 13h hard-disk get-drive-parameters (`AH=08h`, `DL=80h`):

| Case | Behavior |
|---|---|
| IDE image attached (non-empty) | `CF` clear; `CX`/`DH`/`DL` from fixed 16 heads / 63 SPT geometry derived from image size |
| No IDE / empty image | `CF` set, `AH=80h` (timeout / not ready) |

Max cylinder index = `(total_sectors - 1) / (16 * 63)` (same helper as AH=48h EDD).

## Honesty / unsupported

- Host subset only; not INT 13h translation modes / LBA-assist.
- Not a guest IVT BIOS body.
- Geometry words match IDE IDENTIFY obsolete CHS (16/63).

## Spec

IBM PC BIOS INT 13h AH=08h; ATA IDENTIFY obsolete geometry words 3/6;
`docs/storage-r7-int13-hd.md`.
