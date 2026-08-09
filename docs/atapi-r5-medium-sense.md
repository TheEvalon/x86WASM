# TEST UNIT READY and medium sense honesty

Milestone 2, round 5, slice 4. With a medium model, `TEST UNIT READY` must
stop lying.

## Approved sources used here

- **SFF-8020i** §10.8.24 TEST UNIT READY; Sense Key `2h` NOT READY; ASC `3Ah`
  MEDIUM NOT PRESENT.
- Round-4 note in `docs/atapi-r4-packet-protocol.md` — TUR reported GOOD only
  because there was no medium model; that note is superseded for CD-ROM
  capable devices.

## Behavior

| Configuration | TUR |
|---|---|
| `attach_atapi_device` (type `1Fh`, no medium model) | GOOD (unchanged) |
| CD-ROM capable, no image loaded | CHECK CONDITION, sense key `2h`, ASC `3Ah` |
| CD-ROM capable, image loaded | GOOD |

`REQUEST SENSE` after a failed TUR reports and clears that sense data, same
fixed-format path as round 4.

`START/STOP UNIT` and `GET EVENT STATUS NOTIFICATION` are **not** implemented
in this slice — out of scope unless a later slice needs tray semantics.

## Still unsupported

- Unit Attention on medium change / reset
- Tray open/close, START/STOP UNIT, GESN
- CD-ROM boot path (machine / firmware)
