# ATAPI CD-ROM medium model

Milestone 2, round 5, slice 3. Round 4 left a detectable PACKET device with
three opcodes and peripheral type `1Fh`. This slice adds an optional **CD-ROM
capable** configuration that can attach a raw 2048-byte-sector image (ISO 9660
or any Mode-1 user-data image) and run `READ CAPACITY` / `READ (10)`.

## Approved sources used here

- **ATA/ATAPI-6** §8.16.9 — IDENTIFY PACKET DEVICE word 0 bits (12:8) peripheral
  device type; bit 7 RMB.
- **SFF-8020i** / **MMC** — `READ CAPACITY` (`25h`), `READ (10)` (`28h`),
  2048-byte logical block length for CD-ROM Mode 1 user data.
- **SCSI Primary Commands** — peripheral device type `05h` CD/DVD.

## Configuration

| API | Peripheral type | Medium | Packet set |
|---|---|---|---|
| `attach_atapi_device` (unchanged) | `1Fh` | none | TUR / REQUEST SENSE / INQUIRY only |
| `attach_atapi_cdrom` | `05h`, RMB=1 | empty | + READ CAPACITY, READ (10) |
| `attach_atapi_cdrom_image` / `load_atapi_medium` | `05h`, RMB=1 | loaded | same |

Non-configured channels and ATA disks are untouched. Attaching a disk image to
a packet channel still demotes it to an ATA disk.

IDENTIFY PACKET DEVICE word 0 reports `05h` **only** on the CD-ROM capable
path — the round-3/4 honesty rule: do not claim CD-ROM without READ.

INQUIRY byte 0 matches (`05h`), byte 1 sets RMB, and the product string names
a CD-ROM.

## Image layout

Raw 2048-byte logical blocks, contiguous. Length is truncated down to a whole
number of blocks. ISO 9660 is not parsed; El Torito is not implemented.

`READ CAPACITY` returns last LBA = `blocks − 1` and block length `2048`. An
empty drive completes `READ CAPACITY` / `READ (10)` with NOT READY /
`3Ah` MEDIUM NOT PRESENT (slice 4).

## Still unsupported

- CD-ROM boot / El Torito / INT 13h CD emulation
- `READ TOC`, `MODE SENSE`, audio, multi-session
- DMA PACKET, 2352-byte raw sectors, Device 1 ATAPI
