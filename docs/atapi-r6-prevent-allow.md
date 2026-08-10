# ATAPI PREVENT/ALLOW MEDIUM REMOVAL

Milestone 2, round 6, slice 3. Locks soft eject from START/STOP UNIT so
firmware that issues PREVENT before media probes does not lose the tray.

## Approved sources used here

- **SFF-8020i** §10.8.11 PREVENT/ALLOW MEDIUM REMOVAL (`1Eh`) — Prevent bit in
  CDB byte 4; unlocked by default; cleared by hard reset / DEVICE RESET.
- **SFF-8020i** Table 84 — eject while locked → sense key `2h` NOT READY,
  ASC `53h` MEDIA REMOVAL PREVENTED.
- **SFF-8020i** §10.8.25 START/STOP UNIT — LoEj eject respects the lock.

## Behavior

| Action | Result |
|---|---|
| PREVENT (Prevent=1) | GOOD; `atapi_removal_prevented()` true |
| ALLOW (Prevent=0) | GOOD; lock cleared |
| START STOP eject while locked | CHECK CONDITION, ASC `53h`; medium stays |
| START STOP eject while unlocked | Unloads (slice 2) |
| DEVICE RESET / SRST | Clears lock |
| Minimal type `1Fh` | ASC `20h` unknown opcode |

There is no physical door motor; PREVENT only gates the soft-eject path.

## Still unsupported

- Persistent Prevent / changer slot selection
- Manual eject button / MCR ATA door-lock sequence
- Unit Attention on medium change
- El Torito / INT 13h CD boot (slice 4)
