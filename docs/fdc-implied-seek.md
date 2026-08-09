# FDC Configure EIS — implied seek

Spec authority: **Intel 82077AA CHMOS Single-Chip Floppy Disk Controller**
datasheet (see `docs/sources.md`), Configure (`0x13`, §5.2.7), Seek (§5.2.8),
Table 5-1 command parameter lists, and §5.3.2 LOCK.

## Spec text used

Configure parameter byte 1 is `0 | EIS | EFIFO | POLL | FIFOTHR(3:0)`. The
datasheet's Configure defaults are:

> EIS — No Implied Seeks
> EFIFO — FIFO Disabled
> POLL — Polling Enabled
> FIFOTHR — FIFO Threshold Set to 1 Byte
> PRETRK — Pre-Compensation Set to Track 0

and

> EIS — Enable Implied Seek. When set to "1", the 82077AA will perform a Seek
> operation before executing a Read or Write command.

The datasheet also spells out what the host must do without it:

> Note that if implied seek is not enabled, the read and write commands should
> be preceded by: 1) Seek command — Step to the proper track, 2) Sense Interrupt
> Status — Terminate the Seek command, 3) Read ID — Verify head is on proper
> track.

## Model

Configure EIS was previously stored and reported through DUMPREG but had no
runtime effect. It is now enforced:

- A command whose parameter list contains a cylinder (`C`) performs an implied
  Seek before execution: `pcn[unit] = C`, where `unit` comes from the US bits
  of the first parameter byte. Per Table 5-1 that is READ DATA (`0x06`), READ
  TRACK (`0x02`), READ DELETED DATA (`0x0C`), VERIFY (`0x16`), SCAN
  (`0x11`/`0x19`/`0x1D`), WRITE DATA (`0x05`) and WRITE DELETED DATA (`0x09`).
- **FORMAT TRACK (`0x0D`) never implies a seek** — its parameters are HD|US, N,
  SC, GPL and D, with no cylinder.
- The seek is mechanical, so it happens **before** the transfer is attempted and
  still moves the head when the transfer then terminates abnormally (no media,
  wrong N, out-of-range CHS).
- The implied seek is part of the command: it queues **no** Seek End ST0 latch
  for Sense Interrupt Status and generates **no** extra interrupt. The single
  IRQ6 remains the command's completion interrupt.
- With EIS clear (including the reset default) the head does not move, matching
  the datasheet's expectation that the host issues Seek / Sense Interrupt Status
  / READ ID itself.
- §5.3.2: an unlocked DOR/DSR software reset restores the Configure defaults, so
  EIS stops applying; LOCK protects only EFIFO / FIFOTHR / PRETRK, never EIS.

The new head position is observable through DUMPREG PCN0–PCN3 (§5.3.3), Sense
Drive Status ST3 T0 (§6.4 bit 4), and READ ID's `C` byte plus its MA|ND result
when the implied seek parked the head past the last formatted cylinder (see
`docs/fdc-read-id-scan.md`).

## Explicitly not supported

- Step-pulse and head-settle timing (SRT/HLT from Specify); the implied seek is
  instantaneous.
- Clearing the DIR DSKCHG latch. This tree still clears disk-change only on an
  explicit Recalibrate / Seek / Relative Seek with media.
- Gating transfer success on head position: READ/WRITE DATA still address by
  their own `C` parameter, and ST2 WC (Wrong Cylinder) / BC (Bad Cylinder) are
  never reported. With EIS set the implied seek makes `C` and `pcn[unit]` agree
  anyway; with EIS clear a mismatched head position is not detected.
- Configure POLL (drive polling disable) and EFIFO/FIFOTHR runtime effects,
  which remain stored-only.
