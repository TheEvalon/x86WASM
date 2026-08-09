# ATAPI detection: the PACKET signature and IDENTIFY PACKET DEVICE

Milestone 2, round 3, slice 4. One bounded step toward ATAPI, done properly:
a *configured* packet device can now be **detected** by firmware. It is not a
CD-ROM and does not pretend to be one.

## Approved sources used here

- **ATA/ATAPI-6 (T13/1410D revision 3a/3b)**, already listed in
  `docs/sources.md`:
  - §6.8 PACKET Command feature set, §6.8.1 Identification of PACKET Command
    feature set devices
  - §8.15.5.2 IDENTIFY DEVICE — outputs for PACKET Command feature set devices
  - §8.16 IDENTIFY PACKET DEVICE, including §8.16.5 normal outputs, §8.16.7
    prerequisites, §8.16.9 word 0, §8.16.18 word 49, §8.16.19 word 50, and
    Table 29
  - §8.34.5.2 READ SECTOR(S) — outputs for PACKET Command feature set devices
  - §9.10 / §9.11 reset state machines (states D0ED3, D0HR3, D0SR2), §9.12
    Signature and persistence
  - §5.2.9 INTRQ, §8.11 / Table 26 EXECUTE DEVICE DIAGNOSTIC

The SFF-8020i / MMC command set is deliberately **not** used: no command packet
set is implemented, so there is nothing to cite it for.

## What a configured packet device does

`IdePrimary::attach_atapi_device()` / `with_atapi_device()` (and the same pair
on `IdeSecondary`) configure Device 0 as a PACKET Command feature set device
with no media. `is_packet_device()` reports it.

| Event | Behavior | Spec |
|---|---|---|
| Power-on / hardware / software reset | Command Block registers get `01h`/`01h`/`14h`/`EBh`; Status reads `00h` | §9.12, §9.10/§9.11 ("clear bits 6, 5, 4, 3, 2 and 0") |
| EXECUTE DEVICE DIAGNOSTIC (`0x90`) | Error `01h`, PACKET signature, Status `00h` | §8.11, §9.10 state D0ED3, §9.12 |
| IDENTIFY DEVICE (`0xEC`) | command aborted, full PACKET signature written | §6.8.1, §8.15.5.2 |
| READ SECTOR(S) (`0x20`) | command aborted, signature written to **LBA Mid and LBA High only** | §8.34.5.2 |
| IDENTIFY PACKET DEVICE (`0xA1`) | 256-word PIO data-in; accepted regardless of DRDY; completes DRDY=1, DRQ=0, ERR=0 | §8.16 |
| PACKET (`0xA0`) | command aborted | not implemented — see below |
| Everything else | command aborted | ATA/ATAPI-6 unimplemented-command response |

A host therefore sees a device whose Status is `00h` — the same as an empty
channel — and distinguishes it by the signature. That is exactly what §9.12
exists for, and it is why DRDY is *not* set after reset.

## What IDENTIFY PACKET DEVICE reports, and why

| Word | Value | Reason |
|---|---|---|
| 0 | `9F00h` | bits 15:14 = `10b` (PACKET device, §8.16.9); bits 12:8 = `1Fh`; bit 7 = 0 (non-removable); bits 6:5 = `00b` (3 ms DRQ); bits 1:0 = `00b` (12-byte packet) |
| 10–19 | zero | serial number is optional; "if not implemented, the content shall be zeros" |
| 23–26 | `"0001    "` | firmware revision |
| 27–46 | `"x86WASM ATAPI IDENTIFY-ONLY"` padded | model number, saying what this is |
| 49 | `0200h` | bit 9 "shall be set to one"; no DMA, IORDY, overlap or queuing claimed |
| 50 | `4000h` | bit 15 clear / bit 14 set |
| 53 | `0000h` | words (70:64) and word 88 reported **invalid** — there is no transfer-timing model to fill them |
| 63, 88 | zero | no multiword DMA, no Ultra DMA |
| 82 | `0010h` | bit 4 "shall be set to one indicating the PACKET Command feature set is supported"; bit 9 clear because DEVICE RESET is not implemented |
| 83, 84, 87 | `4000h` | bit 15 clear / bit 14 set |
| 85 | `0010h` | bit 4, as word 82 |
| 80 | zero | no ATA major version claimed |

**The important field is word 0 bits (12:8).** §8.16.9 says it names the
command packet set "following the peripheral device type value as defined in
SCSI Primary Commands". `05h` would mean CD-ROM. This device implements no
command packet set at all, so it reports the defined value `1Fh` — "Unknown or
no device type". Nothing in the identify block claims a capability the device
lacks.

## Model choices

- **Status bit 4.** On a PACKET device that bit is SERV, not DSC. This model
  never sets it, because nothing here ever has a service request. Command
  completion on a packet device is therefore `DRDY` alone rather than the
  `DRDY | DSC` an ATA disk reports.
- **Signature persistence.** §9.12 says the PACKET signature "shall not be
  changed by the device until the device receives a command that sets DRDY to
  one", which permits changing it after IDENTIFY PACKET DEVICE. §8.16.5 lists
  the Command Block registers as "na" on completion. This model simply never
  changes it, so a host that identifies and then re-reads the registers still
  sees `01h/01h/14h/EBh`. A host write to those registers still overwrites and
  invalidates the signature, as §9.12 requires.
- **Bits 6:5 and 1:0 of word 0** state a DRQ response time and a 12-byte
  command packet. Both are mandatory fields with no "not applicable" encoding,
  and neither is reachable because PACKET aborts.

## A channel with no packet device is untouched

Everything the tree did before is preserved exactly: `IdePrimary::new()` and
`with_image()` produce a non-PACKET device that reports `01h/01h/00h/00h`,
sets `DRDY | DSC`, and aborts both `0xA0` and `0xA1`. Attaching a disk image to
a channel that was configured as a packet device turns it back into an ATA
disk. `reset()` preserves the device type.

## Still unsupported

- **The PACKET command itself.** No command packet PIO, no SFF-8020i / MMC
  command set, no `TEST UNIT READY`, no `INQUIRY`, no `READ (10)`.
- **No CD-ROM and no media.** There is no ISO image path, no tray, no medium
  change notification, and no CD-ROM boot.
- DEVICE RESET (`0x08`), which §6.8 makes mandatory for a real PACKET device.
  Word 82 bit 9 stays clear rather than claiming it.
- Overlap, command queuing, SERVICE and release interrupts, DMA of any kind.
- A packet device on Device 1; the configuration applies to Device 0, and
  §9.16.1 "Device 0 only configurations" still governs Device 1 probes.
- PIO transfer-mode reporting (words 64–70) and Ultra DMA (word 88), which is
  why word 53 marks them invalid.
