# FDC READ ID track ID-field scan

Spec authority: **Intel 82077AA CHMOS Single-Chip Floppy Disk Controller**
datasheet (see `docs/sources.md`), READ ID (`0x0A`, Table 5-1 / §5.1.8) and
Status Register 1 (§6.2), plus the IBM PC / OSDev 1.44MB MFM geometry entry.

## Spec text used

> The READ ID command is used to find the present position of the recording
> heads. The 82077AA stores the values from the first ID Field it is able to
> read into its registers. If the 82077AA does not find an ID Address Mark on
> the diskette after the second occurrence of a pulse on the INDX# pin, then it
> sets the IC code in Status Register 0 to '01' (Abnormal termination), sets the
> MA bit in Status Register 1 to '1', and terminates the command.

Status Register 1 (§6.2): bit 0 = MA (Missing Address Mark), bit 2 = ND (No
Data — the requested/expected ID field was not read).

READ ID is a control command: one parameter byte (HD|US), no execution-phase
data transfer, a 7-byte result (ST0/ST1/ST2/C/H/R/N), and an interrupt on
completion (gated by DOR bit 3).

## Model

The previous implementation was a fixed stub that always answered `R = 1`,
`N = 2` whenever any media was attached — including when the head had been
seeked past the last formatted cylinder. It is replaced by a **deterministic
rotational scan**:

- Each drive (`US` 0–3) has its own zero-based position into the track's ID
  fields, `read_id_position[unit]`.
- A READ ID that can read an ID field reports `R = position + 1`, `C =
  pcn[unit]`, `H` from the HD parameter bit, `N = 2`, and then advances the
  position modulo 18 (`FDC_1440_SECTORS_PER_TRACK`). Successive READ ID
  commands therefore walk sector IDs 1, 2, … 18, 1, … exactly as the diskette
  turning under the head would present them.
- An ID field is readable when media is attached **and** the head is over a
  formatted cylinder (`pcn[unit] < 80`) and a valid side (`head < 2`).
- Otherwise the command terminates abnormally: ST0 IC=01 | H | US, ST1 = MA|ND,
  ST2 = 0, C/H/R/N = 0, and the position is left unchanged (nothing was read).
- The position is **not** reset by Seek/Recalibrate — the diskette keeps
  spinning while the head steps. Hardware reset (`Fdc82077::reset`) and DOR/DSR
  software reset restart it at the first ID field.

The raw 1.44MB image carries no explicit ID fields, so ID contents are derived
from the IBM MFM format: cylinder = present cylinder, head = selected side,
sector = 1..18, N = 2.

## Behavior change vs. the previous stub

| Case | Before | Now |
|---|---|---|
| Repeated READ ID on one track | always `R=1` | `R` = 1, 2, 3, … wrapping at 18 |
| Head seeked to cylinder ≥ 80 | success, `C` = that cylinder, `R=1` | IC=01, ST1 MA\|ND, zeros |
| No media / ejected | IC=01, ST1 ND | IC=01, ST1 **MA**\|ND (§5.1.8 names MA) |

## Explicitly not supported

- Real INDX# / index-pulse timing, rotational latency, or data-rate dependent
  behavior. The scan advances once per command, not on a clock.
- ID fields written by FORMAT TRACK: the format path fills sector data only, so
  a reformatted track still reports the standard IBM 1.44MB ID sequence
  (C/H/1..18/N=2) rather than the programmed C/H/R/N bytes.
- CRC errors in the ID field (ST1 DE, ST2 CRC), Wrong Cylinder (ST2 WC) or Bad
  Cylinder (ST2 BC) reporting.
- Media formats other than 1.44MB (80/2/18, N=2).
- Using head position to gate READ/WRITE DATA success — those commands still
  address by their `C` parameter and do not report ST2 WC.
