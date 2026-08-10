# ATAPI MODE SENSE (6) / (10)

Milestone 2, round 6, slice 1. CD-ROM capable PACKET devices (`05h`) answer
firmware probes that ask for mode pages without inventing a full page database.

## Approved sources used here

- **SFF-8020i** §9.8.4 MODE SENSE (`5Ah`) — Page Control / Page Code /
  allocation length; unsupported page → sense key `5h` ILLEGAL REQUEST, ASC
  `24h` INVALID FIELD IN COMMAND PACKET.
- **SFF-8020i** §9.8.5 / Table 45 — mode parameter header; this model returns
  **no** block descriptors (block descriptor length `0`).
- **SFF-8020i** Table 52 — Read Error Recovery page (`01h`), page length `06h`.
- **SFF-8020i** Table 46 — medium type codes (`70h` no disc, `01h` 120 mm data).
- **SFF-8020i** Table 8 — MODE SENSE does **not** report NOT READY when empty.
- **MMC / SPC** MODE SENSE(6) (`1Ah`) — 4-byte header + the same page `01h`.

## Behavior

| Configuration | MODE SENSE(6)/(10) |
|---|---|
| Minimal PACKET (`1Fh`) | CHECK CONDITION, ASC `20h` (unknown opcode) |
| CD-ROM, page `01h` or `3Fh` | GOOD; header + Read Error Recovery page |
| CD-ROM, other page codes | CHECK CONDITION, ASC `24h` |
| CD-ROM, PC = saved (`11b`) | CHECK CONDITION, ASC `24h` (not implemented) |
| Empty / loaded medium | Succeeds; medium type `70h` / `01h` |

Default recovery parameters are zeros (maximum recovery, recovered errors not
reported, retry count `0`, PS clear). MODE SELECT is out of scope.

## Still unsupported

- Capabilities page `2Ah`, audio / CD-ROM parameter pages, block descriptors
- MODE SELECT, saved values, changeable-value writes
- `READ TOC`, `START STOP UNIT`, El Torito / INT 13h CD boot
