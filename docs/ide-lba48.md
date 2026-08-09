# IDE 48-bit Address feature set (READ/WRITE SECTOR(S) EXT)

Spec authority: **ATA/ATAPI-6, T13/1410D revision 3b** (see `docs/sources.md`,
"ATA / ATAPI specifications"). Section numbers below are from that draft.

`devices::IdePrimary` (and `devices::IdeSecondary`) implement the two commands
that make the feature set usable for PIO disk access:

| Command | Code | Protocol | Spec |
|---|---|---|---|
| READ SECTOR(S) EXT | `24h` | PIO data-in | §8.35 |
| WRITE SECTOR(S) EXT | `34h` | PIO data-out | §8.63 |

## Register model (§6.20, Table 11)

The Features, Sector Count, LBA Low, LBA Mid and LBA High registers are each a
**two-byte deep FIFO**. A write puts the byte in the "most recently written"
half and pushes the old value into "previous content". A 48-bit command is
therefore programmed high byte first, then low byte:

| Register | "most recently written" | "previous content" |
|---|---|---|
| Sector Count | Sector count (7:0) | Sector count (15:8) |
| LBA Low | LBA (7:0) | LBA (31:24) |
| LBA Mid | LBA (15:8) | LBA (39:32) |
| LBA High | LBA (23:16) | LBA (47:40) |
| Device | LBA bit set to one; DEV selects the device; **bits 3:0 reserved** | — |

The host reads the previous half by setting **HOB** (bit 7) of the Device
Control register (§7.8.6). "A write to any Command Block register shall cause
the device to clear the HOB bit to zero in the Device Control register" and
"the 'most recently written' content always gets written by a register write
regardless of the state of HOB".

Note the Device register difference from LBA28 (Table 12): for a 48-bit command
Device bits 3:0 are reserved and take no part in the address, whereas the LBA28
path uses them as LBA (27:24). This tree models both views separately.

## Transfer behavior

- Sector Count is 16 bits; `0000h` requests **65,536** sectors (§8.35.8 /
  §8.63.8).
- One DRQ block per sector, and "the device shall interrupt for each DRQ block
  transferred" — INTRQ is gated by nIEN exactly like the LBA28 path.
- The Device register LBA bit must be set: "The 48-bit Address feature set
  operates in LBA only" (§6.20). A CHS-style Device register aborts with
  ERR+ABRT.
- Addressing errors: §6.2.2 requires IDNF or ABRT when the requested LBA is at
  or beyond the user-addressable capacity. This tree validates the **whole**
  requested range before starting, sets ERR+IDNF, and presents no DRQ block, so
  an out-of-range WRITE SECTOR(S) EXT never touches media.
- On normal completion the Sector Count outputs are Reserved (§8.35.5 /
  §8.63.5); this tree reports zero in both FIFO halves, matching the LBA28 path.

## IDENTIFY DEVICE reporting

- Word 83 bit 10 and word 86 bit 10 = 1 (48-bit Address feature set supported).
  Word 83 bit 14 stays 1 as the spec requires.
- Words (103:100) = 48-bit user-addressable sector count (max LBA + 1), capped
  at `0000FFFFFFFFFFFFh` (§6.2.1).
- Words (61:60) = LBA28 sector count capped at 268,435,455 (§6.2.1 / §6.20:
  "words (61:60) shall describe the maximum capacity that can be addressed by
  28-bit commands").
- READ NATIVE MAX ADDRESS now clamps its answer to 268,435,454 as §6.20
  requires when the native maximum exceeds the 28-bit range.

The support bits are only set because the two EXT commands actually work; no
other member of the feature set is advertised.

## Explicitly not supported

- READ DMA EXT, WRITE DMA EXT, READ/WRITE DMA QUEUED EXT, READ MULTIPLE EXT,
  WRITE MULTIPLE EXT, READ VERIFY SECTOR(S) EXT, READ NATIVE MAX ADDRESS EXT,
  SET MAX ADDRESS EXT. FLUSH CACHE EXT (`EAh`) remains the pre-existing
  success stub with no cache behind it.
- Host Protected Area interaction between SET MAX ADDRESS and SET MAX ADDRESS
  EXT (§6.20) — SET MAX ADDRESS still aborts.
- Error-output addressing: §8.35.6 / §8.63.6 say the Command Block registers
  should hold the address of the first unrecoverable error (readable through
  HOB). This tree leaves the command address in the task file because the range
  is rejected before any sector is transferred.
- HOB clearing on a Data port write. §6.20 says "any Command Block register";
  this tree clears HOB only on Features / Sector Count / LBA Low / LBA Mid /
  LBA High / Device / Command writes so a PIO data-out block cannot clear HOB
  mid-transfer.
- Device Configuration Overlay word 7 bit 8 (disabling 48-bit addressing).
