# ATAPI sense data, REQUEST SENSE, and DEVICE RESET

Milestone 2, round 4, slice 2. Slice 1 gave the PACKET protocol a CHECK
CONDITION path; this slice makes it *usable* by letting a host find out what
went wrong, and adds the one mandatory PACKET command round 3 recorded as
missing.

## Approved sources used here

- **SFF-8020i (ATA Packet Interface for CD-ROMs)** — §10.8.16 REQUEST SENSE and
  the fixed-format sense data layout; the Sense Key and ASC / ASCQ definition
  tables. **Needs a `docs/sources.md` entry** (see
  `docs/atapi-r4-packet-protocol.md`).
- **ATA/ATAPI-6 (T13/1410D revision 3a/3b)**, already listed:
  - §8.21.6 PACKET error outputs — the Error register on a PACKET device
    carries the Sense Key in bits (7:4), ABRT in bit 2
  - §8.7 DEVICE RESET — §8.7.2 "use prohibited" without the PACKET Command
    feature set; §8.7.5 normal outputs; the command does not assert INTRQ
  - §9.12 signature, §9.10 PACKET-device reset status
  - Table 29 — IDENTIFY PACKET DEVICE word 82 bit 9 and word 85 bit 9

## The sense-data model

Three bytes of state: sense key, additional sense code (ASC), and additional
sense code qualifier (ASCQ). A CHECK CONDITION sets them; `REQUEST SENSE`
reports and clears them. `IdePrimary::atapi_sense()` exposes the triple to a
host without running a packet command.

The 18 bytes `REQUEST SENSE` returns:

| Byte | Value | Reason |
|---|---|---|
| 0 | `70h` | current error; VALID (bit 7) clear — there is no information field |
| 1 | `00h` | segment number |
| 2 | sense key in bits (3:0) | FILEMARK / EOM / ILI all clear |
| 3:6 | zero | information field, not valid |
| 7 | `0Ah` | additional sense length: 8 + 10 = 18 bytes |
| 8:11 | zero | command-specific information |
| 12 | ASC | |
| 13 | ASCQ | |
| 14 | `00h` | field replaceable unit code |
| 15:17 | zero | sense-key specific |

Only two sense keys can occur:

| Condition | Sense key | ASC / ASCQ |
|---|---|---|
| unimplemented packet operation code | `5h` ILLEGAL REQUEST | `20h` / `00h` INVALID COMMAND OPERATION CODE |
| `INQUIRY` with EVPD set or a page code | `5h` ILLEGAL REQUEST | `24h` / `00h` INVALID FIELD IN COMMAND PACKET |
| nothing to report | `0h` NO SENSE | `00h` / `00h` |

**Clearing is tied to reporting, not to issuing the command.** SFF-8020i keeps
sense data valid until it is reported, so a `REQUEST SENSE` with an allocation
length of zero transfers nothing and leaves the sense data in place. A
*successful* packet command does not clear it either; only a report does. This
is what lets a host do `PACKET → sees ERR → PACKET REQUEST SENSE` and get the
reason for the first command.

## DEVICE RESET (`08h`)

| Output | Value | Spec |
|---|---|---|
| Error | `01h` diagnostic passed | §8.7.5 |
| Sector Count / LBA Low / Mid / High | `01h`/`01h`/`14h`/`EBh` | §8.7.5 → §9.12 |
| Status | `00h` | §9.10 — a PACKET device clears bits 6, 5, 4, 3, 2 and 0 |
| INTRQ | **not asserted** | §8.7 |
| Device Control (nIEN, HOB), Device register | untouched | this is a device reset, not a software reset |
| command in progress, pending sense | dropped | there is no longer a command to report on |

On a device that does not implement the PACKET Command feature set the command
is ERR + ABRT (§8.7.2).

Not asserting INTRQ is the one behavior easy to get wrong here: EXECUTE DEVICE
DIAGNOSTIC (`90h`), which this device also implements and which produces a very
similar task file, *does* interrupt.

With `08h` implemented, IDENTIFY PACKET DEVICE word 82 bit 9 and word 85 bit 9
are now set. Round 3 deliberately left them clear and said why; that claim is
now truthful rather than absent.

**Word 0 bits (12:8) are unchanged at `1Fh`.** Nothing in this slice gives the
device a medium or a medium-capable command packet set.

## Still unsupported

- **Unit Attention.** A real device queues a `6h` UNIT ATTENTION / `29h` POWER
  ON, RESET, OR BUS DEVICE RESET OCCURRED condition after a reset. This model
  clears the sense data instead, because there is no medium or configuration
  state for a host to have missed.
- Deferred errors (response code `71h`), descriptor-format sense data, the
  information and command-specific information fields, sense-key specific data,
  and every sense key other than NO SENSE and ILLEGAL REQUEST — including
  `2h` NOT READY, which needs a medium model first.
- `REQUEST SENSE` is a data-in command like any other, so it inherits the
  slice-1 limits: 12-byte packets, no DMA, no overlap.
