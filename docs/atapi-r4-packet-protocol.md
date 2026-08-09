# The ATAPI PACKET protocol

Milestone 2, round 4, slice 1. Round 3 made a packet device *detectable*
(`docs/atapi-r3-identify-and-signature.md`); this slice makes `PACKET` (`A0h`)
actually run. It is still not a CD-ROM, and it still says so.

## Approved sources used here

- **ATA/ATAPI-6 (T13/1410D revision 3a/3b)**, already listed in
  `docs/sources.md`:
  - §8.21 PACKET — §8.21.2 "use prohibited" on non-PACKET devices; §8.21.4
    inputs (Features DMA bit 0 and OVL bit 1; the Byte Count Limit in Cylinder
    Low / Cylinder High); §8.21.5 normal outputs; §8.21.6 error outputs, where
    the Error register carries the Sense Key in bits (7:4) and ABRT in bit 2
  - §9.8 PACKET command protocol — the command-packet transfer, the data
    transfer in Byte Count Limit sized blocks, and command completion
  - §7.13 Interrupt Reason register (the Sector Count register on a PACKET
    device): bit 0 C/D, bit 1 I/O, bit 2 REL
  - §8.16.9 IDENTIFY PACKET DEVICE word 0 bits (6:5) DRQ response type and
    bits (1:0) command packet size
  - §5.2.9 INTRQ, §9.10 / §9.11 reset, §9.12 signature
- **SFF-8020i (ATA Packet Interface for CD-ROMs)** for the command packet set:
  §10.8.4 INQUIRY (EVPD, page code, allocation length, and the 36-byte standard
  INQUIRY data), §10.8.24 TEST UNIT READY, and the Sense Key / ASC / ASCQ
  tables. **This needs a new `docs/sources.md` entry** — round 3 explicitly did
  not use SFF-8020i because no command packet set existed.
- **SCSI Primary Commands** is still cited informationally only, for the
  peripheral device type value `1Fh`.

## The state machine

```text
Command register write A0h
  ├─ Features DMA or OVL set ............ ERR + ABRT, no packet phase
  ├─ Byte Count Limit rounds to zero .... ERR + ABRT, no packet phase
  └─ otherwise: command packet phase
        Interrupt Reason = C/D 1, I/O 0, REL 0
        Status = DRDY | DRQ,  no INTRQ
        host writes 12 bytes through the Data register
          └─ dispatch
               ├─ TEST UNIT READY (00h) ... completion, GOOD
               ├─ INQUIRY (12h) ........... data-in phase, then completion
               └─ anything else ........... completion, CHECK CONDITION

data-in block (repeats until the data is drained)
        byte count = min(remaining, Byte Count Limit) in Cylinder Low/High
        Interrupt Reason = C/D 0, I/O 1
        Status = DRDY | DRQ,  INTRQ asserted

completion
        Interrupt Reason = C/D 1, I/O 1
        Status = DRDY            (GOOD)
              or DRDY | ERR      (CHECK CONDITION), Error = SenseKey<<4 | ABRT
        INTRQ asserted
```

**No INTRQ for the command-packet DRQ.** IDENTIFY PACKET DEVICE word 0 bits
(6:5) report `00b`, "the device shall set DRQ to one within 3 ms of receiving
the PACKET command", not `01b` "INTRQ DRQ". §9.8 asserts INTRQ at that point
only in the interrupt-DRQ case, so this device must not.

**12-byte packets only.** Word 0 bits (1:0) report `00b`. A 16-byte command
packet is not accepted and is not claimed anywhere.

## The two implemented packet commands

| Command | Behavior | Spec |
|---|---|---|
| `TEST UNIT READY` (`00h`) | non-data, completes GOOD | SFF-8020i §10.8.24 |
| `INQUIRY` (`12h`) | 36-byte standard INQUIRY data, truncated to the allocation length in packet byte 4 | SFF-8020i §10.8.4 |
| everything else | CHECK CONDITION, sense key `5h` ILLEGAL REQUEST, ASC `20h` INVALID COMMAND OPERATION CODE | SFF-8020i |

`INQUIRY` with EVPD (byte 1 bit 0) set or a non-zero page code (byte 2) is
CHECK CONDITION with ASC `24h` INVALID FIELD IN COMMAND PACKET: no vital
product data pages exist, and answering with the standard data anyway would be
a wrong answer rather than an honest refusal.

### Why `TEST UNIT READY` reports GOOD, not MEDIUM NOT PRESENT

The obvious answer for an empty ATAPI drive is CHECK CONDITION with sense key
`2h` NOT READY and ASC `3Ah` MEDIUM NOT PRESENT. This device does not report
that, because it has **no medium model at all** — no tray, no disc, no
capacity, no medium-change notification. Reporting "medium not present" would
assert a medium state the device cannot have, which is the same class of claim
that the `1Fh` peripheral device type exists to avoid. The logical unit is
ready in the only sense this device implements: it accepts the packet commands
it advertises. When a medium model lands, `TEST UNIT READY` gains the NOT READY
path and this note goes away.

### What INQUIRY reports, and why

| Byte | Value | Reason |
|---|---|---|
| 0 | `1Fh` | peripheral qualifier `000b` (connected) + peripheral device type "unknown or no device type" — **the same value IDENTIFY PACKET DEVICE word 0 reports**, not `05h` CD-ROM |
| 1 | `00h` | RMB clear: non-removable, matching identify word 0 bit 7 |
| 2 | `00h` | no ANSI version claimed; this is not a conforming ATAPI CD-ROM |
| 3 | `02h` | response data format: the bytes below are the standard-defined layout |
| 4 | `1Fh` | additional length, 36 − 5 |
| 8:15 | `"x86WASM "` | vendor identification |
| 16:31 | `"ATAPI PACKET MIN"` | product identification, saying what this is |
| 32:35 | `"0001"` | product revision level |

The identify block's model number changed with it, from
`"x86WASM ATAPI IDENTIFY-ONLY"` to `"x86WASM ATAPI PACKET MINIMAL"`, because
the old one is no longer true. **Word 0 bits (12:8) still report `1Fh`**: two
non-medium commands are not a command packet set, and `05h` would claim a
CD-ROM that does not exist.

## Model choices, not hardware

- **An odd Byte Count Limit is rounded down to even.** §8.21.4 leaves an odd
  limit indeterminate for a PIO transfer; rounding down is the choice that
  cannot over-deliver.
- **A Byte Count Limit of zero (or one, which rounds to zero) aborts the
  command** with ERR + ABRT rather than transferring an indeterminate amount.
  §8.21.4 does not define the case.
- **DRDY is presented during the DRQ phases**, not only at completion. §8.21.5
  requires it at completion; this model reuses the `ready_status()` convention
  round 3 established, under which a packet device reports `DRDY` alone (bit 4
  is SERV on a PACKET device and there is never a service request).
- **Bytes past the end of a DRQ block read as zero.** An odd allocation length
  leaves half of the last 16-bit Data register cycle undefined; padding is
  safer than over-reading the buffer.
- **No BSY window.** There is no timing model, so the device transitions
  straight from the last command-packet byte to the next phase.

## Still unsupported

- **Every packet command except `TEST UNIT READY` and `INQUIRY`.** No
  `READ (10)`, no `READ TOC`, no `READ CAPACITY`, no `MODE SENSE`, no
  `START/STOP UNIT`. No medium of any kind, no ISO image path, no CD-ROM boot.
- **`REQUEST SENSE`.** A CHECK CONDITION stores its sense key, ASC and ASCQ
  (observable through `IdePrimary::atapi_sense`), but the packet command that
  retrieves them is slice 2.
- **`DEVICE RESET` (`08h`)**, still aborted; IDENTIFY PACKET DEVICE word 82
  bit 9 stays clear to say so. Slice 2.
- Packet data-out (host → device) transfers; only data-in exists.
- DMA (Features bit 0), overlap (bit 1), command queuing, `SERVICE`, release
  interrupts, and the REL bit of the Interrupt Reason register.
- A packet device on Device 1.
