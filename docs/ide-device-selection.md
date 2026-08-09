# IDE device selection (DEV bit) with only Device 0 attached

Spec authority: **ATA/ATAPI-6, T13/1410D revision 3b** (see `docs/sources.md`,
"ATA / ATAPI specifications"). Section numbers below are from that draft.

`devices::IdePrimary` (and `devices::IdeSecondary`, which remaps the same
register file to `0x170`–`0x177` / `0x376`) models **Device 0 only**. There is
no second drive. The Device register DEV bit (bit 4, `ATA_DRIVE_SLAVE`) is now
honored so that firmware drive enumeration sees an honest "no Device 1".

## Spec text used

- **§7.7 Device register** — "When the DEV bit is cleared to zero, Device 0 is
  selected. When the DEV bit is set to one, Device 1 is selected."
- **§7.8.5 Device Control register** — "When the Device Control register is
  written, both devices respond to the write regardless of which device is
  selected."
- **§5.2.9 INTRQ** — "When the nIEN bit is set to one or the device is not
  selected, the INTRQ signal shall be released." Deselecting via a Device
  register write releases INTRQ while interrupt pending is still set;
  reselecting asserts INTRQ again.
- **§9.12 Signature and persistence** — a device that does not implement the
  PACKET command feature set places Sector Count `01h`, LBA Low `01h`,
  LBA Mid `00h`, LBA High `00h` in the Command Block registers on power-on
  reset, hardware reset, software reset, and EXECUTE DEVICE DIAGNOSTIC.
- **§9.16.1 Device 0 only configurations** — with Device 1 selected, Device 0
  shall:
  1. complete a Device Control register write as if Device 0 was selected;
  2. complete a Command Block register write **other than the Command
     register** as if Device 0 was selected;
  3. **ignore** a write to the Command register, "except for EXECUTE DEVICE
     DIAGNOSTIC";
  4. (non-PACKET device) complete Control/Command Block reads other than
     Status/Alternate Status as if Device 0 was selected, and return `00h`
     for a Status or Alternate Status read.
- **Table 18 "Device 1 is selected and Device 0 is responding for Device 1"** —
  a Device register read returns "the contents of the Device 0 Device register,
  with the DEV bit set to one"; a Command/Status register read places `00h` on
  the data bus.
- **§8.11 / Table 26 EXECUTE DEVICE DIAGNOSTIC** — diagnostic code `01h`;
  note 2: "If Device 1 is not present, the host may see the information from
  Device 0 even though Device 1 is selected."

## What this tree implements

| Host action with DEV=1 | Modeled response |
|---|---|
| Write Device Control (`0x3F6`) | Applies to Device 0 (SRST enters BSY, SRST release re-readies + writes the signature, nIEN gates INTRQ) |
| Write Features / Sector Count / LBA Low/Mid/High / Device | Lands in the Device 0 register file |
| Write Command register | Ignored — Device 0 status, Error, interrupt pending and any in-progress PIO are untouched |
| Write Command register = `0x90` | Executes on Device 0: Error = `01h`, signature written, DRDY\|DSC, INTRQ pending |
| Read Error / Sector Count / LBA Low/Mid/High | Device 0 content |
| Read Device register | Device 0 content with DEV set to one (the byte the host wrote) |
| Read Status (`0x1F7`) / Alternate Status (`0x3F6`) | `00h`; Device 0 interrupt pending is **not** cleared |
| `irq_line()` | Released (false) while DEV=1, reasserted when Device 0 is reselected |
| Read/write Data port | Ignored (see below) |

## Documented model choices

- **Data port with DEV=1.** Table 18 only enumerates responses for BSY=0 and
  DRQ=0, so it does not define a Data port cycle aimed at the absent Device 1
  while Device 0 has a DRQ block outstanding. This tree ignores the cycle
  (reads return all-ones, writes are dropped) so the Device 0 PIO stream is not
  corrupted by a probe.
- **Device register bits 7 and 5.** ATA/ATAPI-6 marks them obsolete and §9.12
  lists the signature Device value as `00h`; this tree keeps the classic
  `0xA0`-style value that PC firmware writes and reads back, and only the DEV
  bit is interpreted.
- **Reset clears DEV.** A software reset restores the Device register to the
  Device 0 default, matching §9.12's Device value modulo the obsolete bits.

## Explicitly not supported

- An actual Device 1 (no second task file, no second backing image).
- The Device 1 detection handshake (PDIAG-/DASP-), IDENTIFY word 93 hardware
  reset result, or diagnostic codes `8xh` reporting Device 1 failures.
- §9.16.2 "Device 1 only configurations".
- PACKET-device behavior for §9.16.1(5) (a PACKET device returns `00h` for
  *all* Control/Command Block reads); this channel is always a non-PACKET ATA
  disk.
- DEVICE RESET (`0x08`) and the Overlapped/Queued selection rules.
