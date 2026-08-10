# ATAPI READ TOC and START STOP UNIT

Milestone 2, round 6, slice 2. Continues the CD-ROM capable PACKET path after
MODE SENSE with enough TOC and tray semantics for firmware probes.

## Approved sources used here

- **SFF-8020i** §9.8.20 READ TOC (`43h`) — format `00b` TOC (Table 112);
  format `01b` multi-session summary (Table 113) reported as a single session.
- **SFF-8020i** Table 118 — Control `04h` digital data; ADR `1h` → combined
  ADR/Control byte `14h` for Mode-1 data track descriptors.
- **SFF-8020i** §7.6 — MSF addresses include the 150-frame (2 s) pre-gap.
- **SFF-8020i** §9.8.26 START/STOP UNIT (`1Bh`) — LoEj/Start (Table 136).
- **SFF-8020i** Table 8 — both commands may return NOT READY when empty.

## Behavior

| Command | Loaded medium | Empty |
|---|---|---|
| READ TOC format 0 | Track 1 @ LBA 0 + lead-out @ `blocks` | NOT READY / `3Ah` |
| READ TOC format 1 | Single session; first track @ 0 | NOT READY / `3Ah` |
| READ TOC other formats | INVALID FIELD / `24h` | — |
| START STOP LoEj=1 Start=0 | Unloads image (soft eject) | GOOD |
| START STOP other | No-op GOOD | GOOD |

After eject, `TEST UNIT READY` reports NOT READY / ASC `3Ah` like an empty
tray. Load (LoEj=1 Start=1) does not invent media — the host must re-attach an
image. There is no tray motor.

Format field: MMC byte 2 bits (3:0) when non-zero, else SFF-8020i byte 9 bits
(7:6).

## Still unsupported

- Audio CD / multi-track / multi-session PhotoCD TOCs
- Raw lead-in Q (`format 2`), PMA/ATIP
- Physical tray / Unit Attention on medium change
- PREVENT/ALLOW interaction with eject — see `docs/atapi-r6-prevent-allow.md`
- El Torito / INT 13h CD boot
