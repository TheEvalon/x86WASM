# PIIX BMIDE PRD walkers (both directions, primary channel)

Scope note for `devices::PciConfig::start_bm_read` / `start_bm_write` and their
`run_prd_read_stub` / `run_prd_write_stub` aliases.

## Sources

- Intel *Programming Interface for Bus Master IDE Controller* Rev 1.0
  (May 1994) §§1.1–1.2 — Bus Master IDE Command / Status registers and the
  Physical Region Descriptor format: physical base in bits \[31:1\], byte count
  in bits \[15:1\] with a zero field meaning 64 KiB, and EOT in bit 7 of the
  last descriptor byte.
- Intel 82371SB (PIIX3) §2.7 — BMICOM bit0 SSBM, BMICOM bit3 RWCON, BMISTA bit0
  Bus Master IDE Active, BMISTA bit1 DMA Error, BMIDTP descriptor table pointer.

## What both directions do

Starting at BMIDTP (bits \[31:2\]), consecutive 8-byte descriptors are fetched
until one carries EOT. Each descriptor's region is consumed against the caller's
device buffer:

- `start_bm_read` (RWCON cleared) copies the device buffer **into** the regions.
- `start_bm_write` (RWCON set) fills the device buffer **from** the regions.

Both set BMICOM.SSBM and BMISTA.Active while walking, clear both at the end, and
latch BMISTA.Error when the walk fails. Shared bounds:

| Condition | Result |
|---|---|
| Command.BusMaster clear | `BusMasterDisabled`, no register change |
| empty device buffer | `EmptyBuffer`, no register change |
| byte-count field 0 | region is 64 KiB |
| no EOT within 256 descriptors | `MissingEot`, BMISTA.Error |
| descriptor fetch would wrap 32-bit space | `PrdTableAddressOverflow`, BMISTA.Error |
| region would wrap 32-bit space | `GuestAddressOverflow`, BMISTA.Error, no partial copy |
| device buffer shorter than the regions | remaining PRDs are still walked to require EOT |

## Explicitly not implemented

- Any ATA command engine. Neither direction is started by an ATA READ DMA /
  WRITE DMA command or by a guest write to BMICOM.SSBM; both are host-called.
- The secondary channel. BMIDTP at BMIBA+`0x0C` remains store/readback only.
- BMIDE interrupt reporting (BMISTA bit2) and the PRD-exhausted / device-done
  interaction that drives it.
- PCI master/target abort modeling; BMISTA.Error is only latched by the bounds
  checks above.
