# CMOS floppy, fixed-disk, and boot-option bytes

What `Machine::sync_firmware_configuration` writes into CMOS `10h`, `12h`,
`19h`–`2Ch`, and `2Dh`, and where each field comes from.

## Authority

Ralf Brown's Interrupt List, CMOS memory map:

| Index | RBIL entry |
|---|---|
| `10h` | IBM "FLOPPY DRIVE TYPE" — Table C0007 (bits 7-4 first drive, 3-0 second), Table C0008 (type codes) |
| `12h` | IBM "HARD DISK DATA" — Table C0014 (bits 7-4 first drive, 3-0 second; `0Fh` = type 16-255, actual type in `19h`/`1Ah`) |
| `19h` | IBM "FIRST EXTENDED HARD DISK DRIVE TYPE" — Table C0020 (`10h`-`FFh` = type 16-255) |
| `1Ah` | "SECOND EXTENDED HARD DISK DRIVE TYPE" |
| `1Bh`-`23h` | AMI "First Hard Disk (type 47) user defined": cylinders LSB, cylinders high, heads, WPC-low, WPC-high, control byte (Table C0025), landing zone low, landing zone high, sectors per track |
| `24h`-`2Ch` | AMI "Second Hard Disk user defined" — the same nine fields |
| `2Dh` | AMI Hi-Flex BIOS "CONFIGURATION OPTIONS" — Table C0032 (bit 5 boot order: 0 = C: then A:, 1 = A: then C:) |
| `2Eh`/`2Fh` | IBM standard checksum, "byte-wise additive sum of the values in locations 10h-2Dh only" |

Every byte above is inside the checksum range, which is why this slice depends
on `docs/cmos-r3-durability.md` landing first: before that, programming them
produced a checksum that went stale at the next reset.

## What the machine reports

`10h` — floppy. This machine's FDC is a 1.44 MB drive, so drive A: is
`04h` when media is attached and `00h` when it is not, and drive B: is always
`00h`. A machine with a floppy therefore reads `40h`, which is RBIL's own
worked example ("With a single 1.44 drive: 40h"). The rule matches the
equipment byte `14h`, which also counts a drive only when media is present: a
drive with nothing in it is the only thing this FDC model can be.

`12h`/`19h`/`1Bh`-`23h` — fixed disk 0. An image attached through
`Machine::attach_ide_image` is described as the user-defined type: nibble `0Fh`
in `12h`, type 47 in `19h`, and the geometry in the parameter block. Geometry is
16 heads and 63 sectors per track — the same values IDE IDENTIFY already reports
in its obsolete CHS words 3 and 6, so a guest cannot read two different
geometries for one disk — with cylinders derived from the image size.

`12h`/`1Ah`/`24h`-`2Ch` — fixed disk 1 is always absent. `12h` describes the
two drives of a single fixed-disk controller, and this machine models no
primary slave; its second IDE image lives on the secondary channel, which this
byte does not describe. Encoding the secondary master as CMOS drive 1 would
claim a topology the machine does not have.

`2Dh` — boot options. Only bit 5 is set, and only when a floppy is attached and
no fixed disk is: that is the one case where A: is the only bootable device.
Everything else in Table C0032 is a feature this machine does not have.

## Model choices (not spec)

- **Write precompensation cylinder** (`1Eh`/`1Fh`) is zero and the **landing
  zone** (`21h`/`22h`) is the cylinder count. RBIL documents where these fields
  live, not what they should contain for an emulated device, and neither has
  any meaning for a disk backed by a file: there is no MFM write channel to
  precompensate and no head to park.
- **Control byte** (`20h`/`29h`) follows Table C0025 and sets bit 3 ("more than
  8 heads") for the 16-head geometry. RBIL's note on `29h` describes the same
  condition as producing `80h` instead; the two readings conflict and this model
  follows the explicit bitfield table.
- **Cylinders** are the physical count, not an INT 13h-translated one. Nothing
  in this machine performs BIOS CHS translation yet, so no 1024-cylinder cap is
  applied. An image below one cylinder still reports one cylinder rather than
  zero, and an image beyond 65535 cylinders (about 31 GiB at this geometry)
  saturates rather than wrapping into a smaller, wrong disk.
- **Type 47** is the conventional user-defined type RBIL names for the `1Bh`
  block; the alternative on some vendors is 49. Nothing here reads the number
  back, so the choice only has to be self-consistent.

## Who reads these bytes

Nothing in this emulator does. `CmosRtc` stores them and `Machine` fills them
in; no device or machine behavior branches on them.

SeaBIOS does not read them either. It takes its drive configuration from
fw_cfg and ATA enumeration, and its boot order from fw_cfg rather than `2Dh`
(its own CMOS boot-order bytes are `38h`/`3Dh`, outside the checksum range).
These bytes are filled in for AMI-style POST compatibility and because a
configuration area that is battery backed but blank is a worse lie than one
that describes the machine.

## Unsupported (explicit)

- Predefined drive types 1-14 in the `12h` nibble. The API can read them back
  but the machine never selects one; every attached disk is the user-defined
  type.
- Primary slave and both secondary-channel drives in CMOS. `12h` has no field
  for them.
- CMOS `38h`/`3Dh` boot-order bytes (SeaBIOS/QEMU convention) — outside the
  checksum range and not written.
- `11h` (IBM fixed disk / setup options) and `13h` (AMI extended setup), which
  are inside the checksum range and left zero.
- INT 13h CHS translation modes (LBA-assist, bit-shift). The geometry reported
  is physical.
