//! Intel 82077AA floppy disk controller — port stub + Specify/Recalibrate/Seek/
//! Relative Seek/Sense Int/Sense Drive/Version/Configure/LOCK/PERPENDICULAR/DUMPREG/
//! READ DATA (media single-sector + no-media ND) / READ ID (media sector-ID
//! stub + no-media ND) / READ DELETED DATA / WRITE DATA / WRITE DELETED DATA /
//! VERIFY / FORMAT TRACK (no-media stubs) + IRQ6.
//!
//! Classic PC primary FDC at `0x3F0`–`0x3F7`, **excluding** `0x3F6` (owned by
//! primary IDE alternate status / device control on AT machines).
//!
//! # Spec refs
//!
//! - Intel 82077AA CHMOS Single-Chip Floppy Disk Controller — DOR, MSR, FIFO,
//!   DIR/CCR; Specify (`0x03`) two parameter bytes (SRT|HUT, HLT|ND), no result
//!   phase / no IRQ; Recalibrate (`0x07`) one unit-select parameter, Seek End
//!   ST0 + PCN=0 + IRQ; Seek (`0x0F`) HD|US + NCN, Seek End ST0 + PCN=NCN + IRQ;
//!   Relative Seek (`0x8F`/`0xCF`, §5.2.9) DIR in cmd bit6 + HD|US + RCN,
//!   PCN ±= RCN (clamp `0..=255`), Seek End ST0 + IRQ; Sense Interrupt Status
//!   (`0x08`) result ST0+PCN; Sense Drive Status
//!   (`0x04`, §5.2.5) HD|US parameter, no execution phase, result ST3 (§6.4:
//!   bit7 unused=0, bit6 WP, bit5 unused=1, bit4 T0, bit3 unused=1, bit2 HD,
//!   bits1:0 DS1/DS0), no IRQ; Version (`0x10`) no parameters, 1-byte result
//!   `0x90` (82077AA identification); Configure (`0x13`) three parameter bytes
//!   (unused, EIS|FIFO_DIS|POLL_DIS|FIFOTHR, PRETRK), no result/IRQ; LOCK
//!   (`0x14`/`0x94`, §5.3.2) no params, LOCK in command bit7, result
//!   `LOCK<<4`, no IRQ; PERPENDICULAR Mode (`0x12`, §5.2.11 / §5.3.1) one
//!   parameter byte `OW|0|D3–D0|GAP|WGATE`, no result/IRQ; DUMPREG (`0x0E`,
//!   §5.2.10 / §5.3.3) no params, 10-byte result (PCN0–3, SRT|HUT, HLT|ND,
//!   SC/EOT, LOCK|perp, Configure, PRETRK), no IRQ; READ DATA (`0x06` with
//!   optional MT/MFM/SK in bits 7:5, §5.1.1 / Table 5-1) eight parameter
//!   bytes then 7-byte result; with media + N=2 + valid CHS → ST0 IC=00 +
//!   ST1=0 + single-sector `last_sector` latch + IRQ6; else no-media/wrong-N/
//!   OOR → ST0 IC=01 + ST1 ND + C/H/R/N ENDaddress and IRQ6; READ DELETED DATA (`0x0C` with
//!   optional MT/MFM/SK in bits 7:5, §5.1.3 / Table 5-1) same 8-param /
//!   7-result shape as READ DATA; no-media stub completes with ST0 IC=01 +
//!   ST1 ND + C/H/R/N ENDaddress and IRQ6; WRITE DATA (`0x05` with optional
//!   MT/MFM in bits 7:6, §5.1.2 / Table 5-1) same 8-param / 7-result shape;
//!   with media + N=2 + valid CHS → ST0 IC=00 + ST1=0 + single-sector
//!   `last_write` / `write_sector` + IRQ6; else no-media/wrong-N/OOR → ST0
//!   IC=01 + ST1 NW + C/H/R/N ENDaddress and IRQ6; FORMAT TRACK (`0x0D` with optional MFM in bit6, §5.1.7 /
//!   Table 5-1) five parameter bytes then 7-byte result; no-media stub
//!   completes with ST0 IC=01 + ST1 NW + four undefined zeros and IRQ6;
//!   READ ID (`0x0A` with optional MFM in bit6, Table 5-1 / §5.1.8) one HD|US
//!   parameter then 7-byte result; with media → ST0 IC=00 + ST1=0 + sector-ID
//!   stub C=`pcn[unit]` / H from HD / R=1 / N=2 + IRQ6; no-media → ST0 IC=01
//!   with ST1 ND and C/H/R/N=0 + IRQ6; DOR bit3 DMA/IRQ enable; IRQ6 on
//!   command / reset completion.
//! - OSDev Wiki Floppy Disk Controller — port map; MSR RQM/DIO; Specify timing
//!   params; Recalibrate/Seek → IRQ then Sense Interrupt; Sense Interrupt clears
//!   IRQ; post-reset Sense Interrupt polling; Sense Drive Status ST3 fields;
//!   Version returns `0x90` for 82077AA-class controllers; Configure stores
//!   EIS/FIFO/POLL/FIFOTHR/PRETRK with no result bytes; Lock/Unlock via MT bit;
//!   Perpendicular Mode configures GAP/WGATE (and enhanced Dn bits);
//!   DUMPREG dumps internal registers; READ/READ ID/READ DELETED/WRITE DATA /
//!   FORMAT TRACK MT/MFM(/SK) forms → IRQ then 7-byte result (not Sense Interrupt).
//! - IBM PC/AT — floppy controller → IRQ6 (8259 master IR6).
//! - `docs/machine-model-pc-v1.md`, `plan.md` §21 Floppy boot (foundation stub).
//!
//! # Scope (this slice)
//!
//! - Accept R/W on SRA/SRB/DOR/TDR/MSR|DSR/FIFO/DIR|CCR with stored values
//! - Reset defaults: MSR = RQM (`0x80`), DOR = `0x00` (controller held in reset)
//! - IRQ6 stub: `assert_irq6` / `clear_irq6` + `irq_line` gated by DOR nRESET∧DMA/IRQ
//! - Specify (`0x03`): command byte → two parameter bytes (stored); no result;
//!   does not assert or clear IRQ; MSR RQM (!DIO) during parameter phase
//! - Recalibrate (`0x07`): command byte → one unit-select parameter (bits 1:0);
//!   sets `pcn[unit] = 0`, latches ST0 Seek End (`0x20 | unit`) for Sense
//!   Interrupt, asserts IRQ; MSR RQM (!DIO) during parameter phase; no result
//! - Seek (`0x0F`): command byte → HD|US + NCN; sets `pcn[unit] = NCN`, latches
//!   ST0 Seek End (`0x20 | unit`; H bit always 0 per 82077AA), asserts IRQ; no
//!   result
//! - Relative Seek (`0x8F` DIR=0 out / `0xCF` DIR=1 in, §5.2.9 / Table 5-1):
//!   command byte → HD|US + RCN; updates `pcn[unit]` by ±RCN (DIR selects
//!   sign; stub clamps to `0..=255`; ST0 EC beyond track 0 deferred), latches
//!   ST0 Seek End (`0x20 | unit`; H always 0), asserts IRQ; no result
//! - Sense Interrupt Status (`0x08`): command byte → 2-byte result (ST0, PCN);
//!   returns latched Recalibrate/Seek/Relative Seek ST0 when present, else post-reset/`assert_irq6`
//!   stub `0xC0 | DOR[1:0]`; PCN is Present Cylinder Number for ST0 US bits
//!   (`pcn[ST0[1:0]]`); clears latched IRQ; MSR RQM|DIO during result phase
//! - Sense Drive Status (`0x04`): command byte → one HD|US parameter (same
//!   packing as Seek param0) → 1-byte ST3 result; no execution phase, no IRQ
//!   assert/clear; T0 stub reflects `pcn[unit] == 0` for the selected unit;
//!   WP (ST3 bit6) set when media attached and [`Self::write_protected`] is
//!   true (via [`Self::set_write_protected`]; default clear); without media WP
//!   stays 0; reserved bits 3/5 always 1 per 82077AA §6.4; MSR RQM during
//!   parameter, RQM|DIO during result phase
//! - Version (`0x10`): command byte → 1-byte result `0x90` (82077AA id); no
//!   parameters, no IRQ assert/clear; MSR RQM|DIO during result phase
//! - Configure (`0x13`): command byte → three parameter bytes stored
//!   (`configure_byte0`, `configure_eis_fifo_poll_thr`, `configure_pretrk`);
//!   no result phase; no IRQ; MSR RQM (!DIO) during parameter phase.
//! - LOCK (`0x14` unlock / `0x94` lock): Spec Intel 82077AA §5.3.2 — LOCK is
//!   command-byte bit7 (no parameter bytes); one result byte `LOCK<<4` with
//!   MSR RQM|DIO; no IRQ. Soft DOR reset does **not** clear LOCK; when LOCK=0
//!   soft reset restores Configure EFIFO/FIFOTHR/PRETRK stub defaults (0);
//!   when LOCK=1 those Configure fields survive soft reset. Full `reset()`
//!   (hardware) clears LOCK and all Configure fields.
//! - PERPENDICULAR Mode (`0x12`): Spec Intel 82077AA §5.2.11 / Table 5-1 /
//!   §5.3.1 — command byte → one parameter `OW|0|D3 D2 D1 D0|GAP|WGATE`;
//!   always stores GAP|WGATE; updates D3–D0 only when OW=1; no result phase;
//!   no IRQ. Soft DOR reset clears GAP|WGATE only and **preserves** D3–D0
//!   (independent of LOCK). Hardware/`reset()` clears GAP|WGATE and D3–D0.
//!   Gap2/WGATE timing side effects are not enforced (no media engine).
//! - DUMPREG (`0x0E`): Spec Intel 82077AA §5.2.10 / Table 5-1 / §5.3.3 — no
//!   parameters; 10-byte result from stored state with MSR RQM|DIO; no IRQ.
//!   Result order: PCN0–3 (per-drive Present Cylinder Numbers), SRT|HUT, HLT|ND,
//!   SC/EOT, LOCK|0|D3–D0|GAP|WGATE, 0|EIS|EFIFO|POLL|FIFOTHR, PRETRK.
//!   `sc_eot` updated from READ/READ DELETED/WRITE DATA EOT or FORMAT TRACK SC;
//!   byte7 bits 5:0 reflect stored PERPENDICULAR D3–D0|GAP|WGATE (OW not returned).
//! - READ DATA (`0x06` | MT/MFM/SK): Spec Intel 82077AA §5.1.1 / Table 5-1 —
//!   command byte lower 5 bits `00110`; optional MT (`0x80`)/MFM (`0x40`)/
//!   SK (`0x20`); eight params (HD|US, C, H, R, N, EOT, GPL, DTL). With
//!   attached 1.44MB media, `N=2`, and C/H/R in geometry: transfer **one**
//!   sector only (MT ignored this slice), latch bytes in `last_sector` and
//!   arm `dma_read_pending` when DOR DMA/IRQ enable is set (Machine consumes
//!   via [`Self::take_pending_dma_sector`] → ISA DMA ch2 Write), result ST0
//!   (IC=00 normal | H | US), ST1=0, ST2=0, C/H/R/N = command ENDaddress. No
//!   media / wrong N / OOR CHS → ST0 IC=01 abnormal | H | US, ST1 ND, ST2=0.
//!   Asserts IRQ6 (cleared on first result byte); EOT→`sc_eot`; MSR RQM during
//!   params, RQM|DIO during result.
//! - READ DELETED DATA (`0x0C` | MT/MFM/SK): Spec Intel 82077AA §5.1.3 /
//!   Table 5-1 — command byte lower 5 bits `01100`; optional MT/MFM/SK; same
//!   eight params and 7-byte result as READ DATA; no media image → skip
//!   execution/DMA, immediate result ST0 (IC=01 abnormal | H | US), ST1 ND
//!   (same honest no-media ST1 as READ DATA stub), ST2=0, C/H/R/N = command
//!   ENDaddress; asserts IRQ6 (cleared on first result byte); EOT latched
//!   into `sc_eot`.
//! - WRITE DATA (`0x05` | MT/MFM): Spec Intel 82077AA §5.1.2 / Table 5-1 —
//!   command byte lower 5 bits `00101`; optional MT (`0x80`)/MFM (`0x40`);
//!   same eight params and 7-byte result as READ DATA. With attached 1.44MB
//!   media, `N=2`, and C/H/R in geometry: transfer **one** sector only (MT
//!   ignored this slice) from the `latch_write_sector` buffer (device-only
//!   stand-in) **or** via MachineBus ISA DMA ch2 Read (memory→device) when
//!   DOR DMA/IRQ enable arms [`Self::dma_write_pending`], then
//!   [`Self::commit_dma_write_sector`] / [`Self::write_sector`] into the image,
//!   latch bytes in `last_write`, result ST0 (IC=00 normal | H | US), ST1=0,
//!   ST2=0, C/H/R/N = command ENDaddress. No media / wrong N / OOR CHS → ST0
//!   IC=01 abnormal | H | US, ST1 NW, ST2=0; asserts IRQ6 (cleared on first
//!   result byte); EOT latched into `sc_eot`.
//! - FORMAT TRACK (`0x0D` | MFM): Spec Intel 82077AA §5.1.7 / Table 5-1 —
//!   command byte lower 5 bits `01101`; optional MFM (`0x40`); five params
//!   (HD|US, N, SC, GPL, D); no media image → skip execution/DMA and the
//!   per-sector C/H/R/N ID stream, immediate result ST0 (IC=01 abnormal | H |
//!   US), ST1 NW (Not Writable — §6.2 lists FORMAT TRACK), ST2=0, four
//!   undefined result bytes = 0; asserts IRQ6 (cleared on first result byte);
//!   SC latched into `sc_eot`.
//! - READ ID (`0x0A` | MFM): Spec Intel 82077AA Table 5-1 / §5.1.8 — command
//!   byte lower 5 bits `01010`; optional MFM (`0x40`); one HD|US param; 7-byte
//!   result. With media → ST0 IC=00 | H | US, ST1=ST2=0, sector-ID stub
//!   C=`pcn[unit]`, H from HD bit, R=1, N=2; no media → ST0 IC=01 | H | US,
//!   ST1 ND, C/H/R/N=0; asserts IRQ6 (cleared on first result byte). Full IDAM
//!   track scan deferred.
//! - 1.44MB media image attach/eject + CHS→offset/`read_sector`/
//!   `write_sector` helpers (PC MFM geometry); DIR bit7 DSKCHG stub on
//!   attach/eject; `write_protected` / [`Self::set_write_protected`] for ST3
//!   WP; `reset()` preserves media and write-protect flag like IDE. READ DATA
//!   success path latches one sector in `last_sector` and arms
//!   `dma_read_pending` when DOR bit3 is set for MachineBus auto DMA ch2 Write
//!   (device→memory). WRITE DATA success path accepts one sector via
//!   `latch_write_sector` → `last_write` / `write_sector`, or arms
//!   `dma_write_pending` for MachineBus ISA DMA ch2 Read (memory→device) when
//!   no pre-latch and DOR bit3 is set.
//! - `PortDevice` for MachineBus wiring
//!
//! # Unsupported (explicit)
//!
//! - WRITE DELETED DATA / READ TRACK / VERIFY and other transfer commands;
//!   READ ID full IDAM track scan (sector-ID stub only); READ DELETED DATA
//!   media/deleted-address-mark engine; FORMAT TRACK media write / per-sector
//!   ID DMA
//! - Multi-sector / full MT engine; DREQ/DACK cycle timing; FORMAT media
//! - Seek / Relative Seek step timing; Relative Seek ST0 EC when stepping out
//!   beyond track 0 (PCN clamp only this slice); real DIR disk-change edge
//!   timing (DSKCHG stub only)
//! - Implied seek from Configure EIS; multi-sector TC termination
//! - WRITE DATA / FORMAT ST1 NW from WP pin (Sense Drive ST3 WP only this slice)
//! - PERPENDICULAR Gap2/WGATE/VCO timing side effects on media commands
//! - Configure bit side effects beyond LOCK soft-reset protection (FIFO enable,
//!   implied seek, poll disable enforcement); DSR software-reset path

use crate::PortDevice;

/// Status Register A (read; PS/2 / enhanced). Spec: OSDev FDC.
pub const FDC_SRA: u16 = 0x3F0;
/// Status Register B (read).
pub const FDC_SRB: u16 = 0x3F1;
/// Digital Output Register.
pub const FDC_DOR: u16 = 0x3F2;
/// Tape Drive Register.
pub const FDC_TDR: u16 = 0x3F3;
/// Main Status Register (read) / Data Rate Select (write).
pub const FDC_MSR: u16 = 0x3F4;
/// Data FIFO (command / parameter / result / PIO data).
pub const FDC_FIFO: u16 = 0x3F5;
/// Digital Input Register (read) / Configuration Control Register (write).
pub const FDC_DIR_CCR: u16 = 0x3F7;

/// DIR bit7 — Disk Changed (DSKCHG). Spec: Intel 82077AA DIR; OSDev FDC.
///
/// Hardware latches a media-change edge and typically clears after drive
/// select / recalibrate / step sequencing. This emulator's stub only sets the
/// bit on [`Fdc82077::eject`] (and preserves it across `reset` when no media)
/// and clears it on successful [`Fdc82077::attach_image`] — not full DIR timing.
pub const FDC_DIR_DSKCHG: u8 = 0x80;

/// PC 1.44MB MFM floppy cylinders. Spec: IBM PC / OSDev Floppy Disk.
pub const FDC_1440_CYLINDERS: u8 = 80;
/// PC 1.44MB MFM floppy heads (sides). Spec: IBM PC / OSDev Floppy Disk.
pub const FDC_1440_HEADS: u8 = 2;
/// PC 1.44MB MFM sectors per track. Spec: IBM PC / OSDev Floppy Disk.
pub const FDC_1440_SECTORS_PER_TRACK: u8 = 18;
/// Sector size in bytes for N=2 (`128 << N`). Spec: Intel 82077AA / IBM PC.
pub const FDC_SECTOR_SIZE: usize = 512;
/// Sector-size code N for 512-byte sectors (`128 << 2`). Spec: 82077AA.
pub const FDC_SECTOR_N: u8 = 2;
/// Exact raw image size for a 1.44MB floppy (80×2×18×512). Spec: IBM PC MFM.
pub const FDC_1440_IMAGE_SIZE: usize = 1_474_560;

/// MSR bit7 RQM — FIFO ready for host byte exchange. Spec: Intel 82077AA / OSDev.
pub const FDC_MSR_RQM: u8 = 0x80;
/// MSR bit6 DIO — 1 = FDC→host (result), 0 = host→FDC (command). Spec: 82077AA.
pub const FDC_MSR_DIO: u8 = 0x40;
/// DOR bit2 — when clear, FDC held in reset. Spec: Intel 82077AA / OSDev.
pub const FDC_DOR_RESET_N: u8 = 0x04;
/// DOR bit3 — DMA and IRQ enable. Spec: Intel 82077AA / OSDev FDC.
pub const FDC_DOR_DMA_IRQ: u8 = 0x08;

/// Sense Drive Status command opcode. Spec: Intel 82077AA §5.2.5 — HD|US
/// parameter, no execution phase, 1-byte ST3 result.
pub const FDC_CMD_SENSE_DRIVE_STATUS: u8 = 0x04;
/// Specify command opcode. Spec: Intel 82077AA / OSDev FDC — 2 parameter bytes.
pub const FDC_CMD_SPECIFY: u8 = 0x03;
/// WRITE DATA base opcode (bits 4:0). Spec: Intel 82077AA §5.1.2 / Table 5-1 —
/// command byte is `MT|MFM|0|0 0 1 0 1`; match with [`FDC_CMD_OPCODE_MASK`].
pub const FDC_CMD_WRITE_DATA: u8 = 0x05;
/// WRITE DELETED DATA base opcode (bits 4:0). Spec: Intel 82077AA §5.1.4 / Table 5-1 —
/// command byte is \MT|MFM|0|0 1 0 0 1\; match with [\FDC_CMD_OPCODE_MASK\].
pub const FDC_CMD_WRITE_DELETED_DATA: u8 = 0x09;
/// READ DATA base opcode (bits 4:0). Spec: Intel 82077AA §5.1.1 / Table 5-1 —
/// command byte is `MT|MFM|SK|0 0 1 1 0`; match with [`FDC_CMD_OPCODE_MASK`].
pub const FDC_CMD_READ_DATA: u8 = 0x06;
/// READ DELETED DATA base opcode (bits 4:0). Spec: Intel 82077AA §5.1.3 /
/// Table 5-1 — command byte is `MT|MFM|SK|0 1 1 0 0`; match with
/// [`FDC_CMD_OPCODE_MASK`].
pub const FDC_CMD_READ_DELETED_DATA: u8 = 0x0C;
/// VERIFY base opcode (bits 4:0). Spec: Intel 82077AA Table 5-1 —
/// command byte is `MT|MFM|SK|1 0 1 1 0`; match with [`FDC_CMD_OPCODE_MASK`].
pub const FDC_CMD_VERIFY: u8 = 0x16;
/// READ ID base opcode (bits 4:0). Spec: Intel 82077AA Table 5-1 —
/// command byte is `0|MFM|0|0 0 0 1 0 1 0`; match with [`FDC_CMD_OPCODE_MASK`].
pub const FDC_CMD_READ_ID: u8 = 0x0A;
/// SCAN EQUAL base opcode (bits 4:0). Spec: Intel 82077AA Table 5-1 —
/// command byte is `MT|MFM|SK|1 0 0 0 1`; match with [`FDC_CMD_OPCODE_MASK`].
pub const FDC_CMD_SCAN_EQUAL: u8 = 0x11;
/// SCAN LOW OR EQUAL base opcode. Spec: Intel 82077AA Table 5-1 — `0x19`.
pub const FDC_CMD_SCAN_LOW_OR_EQUAL: u8 = 0x19;
/// SCAN HIGH OR EQUAL base opcode. Spec: Intel 82077AA Table 5-1 — `0x1D`.
pub const FDC_CMD_SCAN_HIGH_OR_EQUAL: u8 = 0x1D;
/// Documented READ ID form: MFM|0x0A. Spec: 82077AA Table 5-1.
pub const FDC_CMD_READ_ID_MFM: u8 = FDC_CMD_MFM | FDC_CMD_READ_ID;
/// READ ID result byte count (ST0, ST1, ST2, C, H, R, N). Spec: 82077AA.
pub const FDC_READ_ID_RESULT_LEN: u8 = 7;
/// Mask for FDC command opcode bits (excludes MT/MFM/SK). Spec: 82077AA Table 5-1.
pub const FDC_CMD_OPCODE_MASK: u8 = 0x1F;
/// Command bit7 Multi-Track (MT). Spec: Intel 82077AA Table 5-1 symbol MT.
pub const FDC_CMD_MT: u8 = 0x80;
/// Command bit6 MFM mode. Spec: Intel 82077AA Table 5-1 symbol MFM.
pub const FDC_CMD_MFM: u8 = 0x40;
/// Command bit5 Skip deleted (SK). Spec: Intel 82077AA Table 5-1 symbol SK.
pub const FDC_CMD_SK: u8 = 0x20;
/// SeaBIOS-common WRITE DATA form: MT|MFM|0x05. Spec: 82077AA Table 5-1.
pub const FDC_CMD_WRITE_DATA_MT_MFM: u8 = FDC_CMD_MT | FDC_CMD_MFM | FDC_CMD_WRITE_DATA;
/// SeaBIOS-common WRITE DELETED DATA form: MT|MFM|0x09. Spec: 82077AA Table 5-1.
pub const FDC_CMD_WRITE_DELETED_DATA_MT_MFM: u8 =
    FDC_CMD_MT | FDC_CMD_MFM | FDC_CMD_WRITE_DELETED_DATA;
/// SeaBIOS-common READ DATA form: MT|MFM|SK|0x06. Spec: 82077AA Table 5-1.
pub const FDC_CMD_READ_DATA_MT_MFM_SK: u8 =
    FDC_CMD_MT | FDC_CMD_MFM | FDC_CMD_SK | FDC_CMD_READ_DATA;
/// Documented READ DELETED DATA form: MT|MFM|SK|0x0C. Spec: 82077AA Table 5-1.
pub const FDC_CMD_READ_DELETED_DATA_MT_MFM_SK: u8 =
    FDC_CMD_MT | FDC_CMD_MFM | FDC_CMD_SK | FDC_CMD_READ_DELETED_DATA;
/// Documented VERIFY form: MT|MFM|SK|0x16. Spec: 82077AA Table 5-1.
pub const FDC_CMD_VERIFY_MT_MFM_SK: u8 = FDC_CMD_MT | FDC_CMD_MFM | FDC_CMD_SK | FDC_CMD_VERIFY;
/// WRITE DATA parameter count after the command byte. Spec: 82077AA §5.1.2.
pub const FDC_WRITE_DATA_PARAM_LEN: u8 = 8;
/// WRITE DATA result byte count (ST0, ST1, ST2, C, H, R, N). Spec: 82077AA §5.1.2.
pub const FDC_WRITE_DATA_RESULT_LEN: u8 = 7;
/// WRITE DELETED DATA parameter count after the command byte. Spec: 82077AA §5.1.4 —
/// same as WRITE DATA.
pub const FDC_WRITE_DELETED_DATA_PARAM_LEN: u8 = 8;
/// WRITE DELETED DATA result byte count (ST0, ST1, ST2, C, H, R, N). Spec: §5.1.4.
pub const FDC_WRITE_DELETED_DATA_RESULT_LEN: u8 = 7;
/// READ DATA parameter count after the command byte. Spec: 82077AA §5.1.1.
pub const FDC_READ_DATA_PARAM_LEN: u8 = 8;
/// READ DATA result byte count (ST0, ST1, ST2, C, H, R, N). Spec: 82077AA §5.1.1.
pub const FDC_READ_DATA_RESULT_LEN: u8 = 7;
/// READ DELETED DATA parameter count after the command byte. Spec: 82077AA §5.1.3 —
/// same as READ DATA.
pub const FDC_READ_DELETED_DATA_PARAM_LEN: u8 = 8;
/// READ DELETED DATA result byte count (ST0, ST1, ST2, C, H, R, N). Spec: §5.1.3.
pub const FDC_READ_DELETED_DATA_RESULT_LEN: u8 = 7;
/// VERIFY parameter count after the command byte. Spec: 82077AA — same as READ DATA.
pub const FDC_VERIFY_PARAM_LEN: u8 = 8;
/// VERIFY result byte count (ST0, ST1, ST2, C, H, R, N). Spec: 82077AA.
pub const FDC_VERIFY_RESULT_LEN: u8 = 7;
/// Recalibrate command opcode. Spec: Intel 82077AA / OSDev FDC — 1 unit parameter.
pub const FDC_CMD_RECALIBRATE: u8 = 0x07;
/// Sense Interrupt Status command opcode. Spec: Intel 82077AA / OSDev FDC.
pub const FDC_CMD_SENSE_INT: u8 = 0x08;
/// FORMAT TRACK base opcode (bits 4:0). Spec: Intel 82077AA §5.1.7 / Table 5-1 —
/// command byte is `0|MFM|0|0 1 1 0 1`; match with [`FDC_CMD_OPCODE_MASK`].
pub const FDC_CMD_FORMAT_TRACK: u8 = 0x0D;
/// SeaBIOS-common FORMAT TRACK form: MFM|0x0D. Spec: 82077AA Table 5-1.
pub const FDC_CMD_FORMAT_TRACK_MFM: u8 = FDC_CMD_MFM | FDC_CMD_FORMAT_TRACK;
/// FORMAT TRACK parameter count after the command byte. Spec: 82077AA §5.1.7 —
/// HD|US, N, SC, GPL, D (execution-phase per-sector C/H/R/N is separate; skipped
/// by the no-media stub).
pub const FDC_FORMAT_TRACK_PARAM_LEN: u8 = 5;
/// FORMAT TRACK result byte count (ST0, ST1, ST2, 4 undefined). Spec: §5.1.7.
pub const FDC_FORMAT_TRACK_RESULT_LEN: u8 = 7;
/// DUMPREG command opcode. Spec: Intel 82077AA §5.2.10 / Table 5-1 — no
/// parameters; 10-byte result dumping internal registers; no IRQ.
pub const FDC_CMD_DUMPREG: u8 = 0x0E;
/// Number of DUMPREG result bytes. Spec: Intel 82077AA Table 5-1 / §5.3.3.
pub const FDC_DUMPREG_RESULT_LEN: u8 = 10;
/// Seek command opcode. Spec: Intel 82077AA / OSDev FDC — HD|US + NCN.
pub const FDC_CMD_SEEK: u8 = 0x0F;
/// Relative Seek base opcode (DIR=0 / step out). Spec: Intel 82077AA Table 5-1 /
/// §5.2.9 — command byte is `1|DIR|0 0 1 1 1 1` (`0x8F` out, `0xCF` in);
/// match with [`Fdc82077::is_relative_seek_command`].
pub const FDC_CMD_RELATIVE_SEEK: u8 = 0x8F;
/// Relative Seek DIR bit in the command byte (bit6). Spec: 82077AA §5.2.9 —
/// DIR=0 step out (toward lower cylinders), DIR=1 step in (toward higher).
pub const FDC_CMD_RELATIVE_SEEK_DIR: u8 = 0x40;
/// Relative Seek with DIR=1 (step in). Spec: Intel 82077AA Table 5-1 / §5.2.9.
pub const FDC_CMD_RELATIVE_SEEK_IN: u8 = FDC_CMD_RELATIVE_SEEK | FDC_CMD_RELATIVE_SEEK_DIR;
/// Version command opcode. Spec: Intel 82077AA / OSDev FDC — no parameters,
/// 1-byte result identifying the controller class.
pub const FDC_CMD_VERSION: u8 = 0x10;
/// PERPENDICULAR Mode command opcode. Spec: Intel 82077AA §5.2.11 / Table 5-1
/// / §5.3.1 / OSDev — 1 parameter byte `OW|0|D3–D0|GAP|WGATE`, no result, no IRQ.
pub const FDC_CMD_PERPENDICULAR: u8 = 0x12;
/// Configure command opcode. Spec: Intel 82077AA / OSDev FDC — 3 parameter
/// bytes, no result phase, no IRQ.
pub const FDC_CMD_CONFIGURE: u8 = 0x13;
/// LOCK command base opcode (bits 6:0). Spec: Intel 82077AA §5.3.2 / OSDev —
/// command byte is `LOCK|0x14` where bit7 is the LOCK value (`0x14` unlock,
/// `0x94` lock); no parameter bytes; 1 result byte.
pub const FDC_CMD_LOCK: u8 = 0x14;
/// LOCK command with LOCK bit set (MT/LOCK position). Spec: 82077AA §5.3.2.
pub const FDC_CMD_LOCK_SET: u8 = 0x94;
/// LOCK result: LOCK value in bit4 (`lock << 4`). Spec: 82077AA §5.3.2 / OSDev.
pub const FDC_LOCK_RESULT_SHIFT: u8 = 4;
/// Version result byte for 82077AA-class controllers. Spec: Intel 82077AA /
/// OSDev FDC — `0x90` identifies enhanced/82077AA (vs `0x80` for older 8272A).
pub const FDC_VERSION_82077AA: u8 = 0x90;
/// ST0 Seek End (SE) bit. Spec: Intel 82077AA status register 0.
pub const FDC_ST0_SEEK_END: u8 = 0x20;
/// ST0 Interrupt Code = 00 (normal termination). Spec: Intel 82077AA §6.1.
pub const FDC_ST0_IC_NORMAL: u8 = 0x00;
/// ST0 Interrupt Code = 01 (abnormal termination). Spec: Intel 82077AA §6.1.
pub const FDC_ST0_IC_ABNORMAL: u8 = 0x40;
/// ST0 Interrupt Code = 11 (abnormal/ready-line-changed stub). Spec: 82077AA / OSDev.
pub const FDC_ST0_IC_READY_CHANGE: u8 = 0xC0;
/// ST0 bit2 Head Address (H). Spec: Intel 82077AA §6.1.
pub const FDC_ST0_HEAD: u8 = 0x04;
/// ST1 bit1 Not Writable (NW) — WP pin asserted / write not possible.
/// Spec: Intel 82077AA §6.2 Status Register 1.
pub const FDC_ST1_NW: u8 = 0x02;
/// ST1 bit2 No Data (ND) — specified sector not found. Spec: Intel 82077AA §6.2.
pub const FDC_ST1_ND: u8 = 0x04;

/// ST3 bits 1:0 — Drive Select (DS1, DS0), status of the DS1/DS0 pins.
/// Spec: Intel 82077AA §6.4 Status Register 3.
pub const FDC_ST3_UNIT_MASK: u8 = 0x03;
/// ST3 bit2 — Head Address (HD), status of the HDSEL pin. Spec: 82077AA §6.4.
pub const FDC_ST3_HEAD: u8 = 0x04;
/// ST3 bit3 — unused, always 1 per 82077AA §6.4 (some clones document as
/// Two-Side; not modeled here).
pub const FDC_ST3_RESERVED_BIT3: u8 = 0x08;
/// ST3 bit4 — Track 0 (T0), status of the TRK0 pin. Spec: 82077AA §6.4.
pub const FDC_ST3_TRACK0: u8 = 0x10;
/// ST3 bit5 — unused, always 1 per 82077AA §6.4 (hardwired high; some
/// software reads this as a Ready bit).
pub const FDC_ST3_RESERVED_BIT5: u8 = 0x20;
/// ST3 bit6 — Write Protected (WP), status of the WP pin. Spec: 82077AA §6.4.
pub const FDC_ST3_WRITE_PROTECT: u8 = 0x40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Idle / accept command byte when RQM && !DIO.
    Command,
    /// Specify parameters: byte0 = SRT|HUT, byte1 = HLT|ND.
    SpecifyParams { index: u8 },
    /// Recalibrate parameter: unit select bits 1:0.
    RecalibrateParams,
    /// Seek parameters: byte0 = HD|US, byte1 = NCN.
    SeekParams { index: u8 },
    /// Relative Seek parameters: byte0 = HD|US, byte1 = RCN.
    RelativeSeekParams { index: u8 },
    /// Sense Interrupt result: ST0 then PCN.
    SenseIntResult { index: u8 },
    /// Sense Drive Status parameter: byte0 = HD|US.
    SenseDriveStatusParam,
    /// Sense Drive Status result: ST3 (single byte).
    SenseDriveStatusResult,
    /// Version result: single identification byte (`0x90` for 82077AA).
    VersionResult,
    /// Configure parameters: byte0 unused, byte1 EIS|FIFO_DIS|POLL_DIS|FIFOTHR,
    /// byte2 PRETRK.
    ConfigureParams { index: u8 },
    /// PERPENDICULAR Mode parameter: `OW|0|D3–D0|GAP|WGATE`. Spec: 82077AA §5.3.1.
    PerpendicularParam,
    /// LOCK result: single status byte (`LOCK<<4`). Spec: 82077AA §5.3.2.
    LockResult,
    /// DUMPREG result: 10 bytes (index 0..9). Spec: 82077AA §5.2.10 / §5.3.3.
    DumpRegResult { index: u8 },
    /// READ DATA parameters: 8 bytes (HD|US, C, H, R, N, EOT, GPL, DTL).
    /// Spec: Intel 82077AA §5.1.1 / Table 5-1.
    ReadDataParams { index: u8 },
    /// READ DATA result: 7 bytes (ST0, ST1, ST2, C, H, R, N). Spec: §5.1.1.
    ReadDataResult { index: u8 },
    /// READ DELETED DATA parameters: 8 bytes (same as READ DATA).
    /// Spec: Intel 82077AA §5.1.3 / Table 5-1.
    ReadDeletedDataParams { index: u8 },
    /// READ DELETED DATA result: 7 bytes (ST0, ST1, ST2, C, H, R, N). Spec: §5.1.3.
    ReadDeletedDataResult { index: u8 },
    /// VERIFY parameters: 8 bytes (same as READ DATA).
    VerifyParams { index: u8 },
    /// VERIFY result: 7 bytes (ST0..N). Spec: 82077AA Table 5-1.
    VerifyResult { index: u8 },
    /// WRITE DATA parameters: 8 bytes (HD|US, C, H, R, N, EOT, GPL, DTL).
    /// Spec: Intel 82077AA §5.1.2 / Table 5-1.
    WriteDataParams { index: u8 },
    /// WRITE DATA result: 7 bytes (ST0, ST1, ST2, C, H, R, N). Spec: §5.1.2.
    WriteDataResult { index: u8 },
    /// WRITE DELETED DATA parameters: 8 bytes (same as WRITE DATA).
    WriteDeletedDataParams { index: u8 },
    /// WRITE DELETED DATA result: 7 bytes (ST0, ST1, ST2, C, H, R, N). Spec: §5.1.4.
    WriteDeletedDataResult { index: u8 },
    /// FORMAT TRACK parameters: 5 bytes (HD|US, N, SC, GPL, D).
    /// Spec: Intel 82077AA §5.1.7 / Table 5-1.
    FormatTrackParams { index: u8 },
    /// FORMAT TRACK result: 7 bytes (ST0, ST1, ST2, 4 undefined). Spec: §5.1.7.
    FormatTrackResult { index: u8 },
    /// READ ID parameter: HD|US (1 byte). Spec: Intel 82077AA Table 5-1.
    ReadIdParam,
    /// READ ID result: 7 bytes (ST0, ST1, ST2, C, H, R, N). Spec: 82077AA.
    ReadIdResult { index: u8 },
}

/// 82077AA-class FDC port stub with Specify/Recalibrate/Seek/Relative Seek/Sense/Version/
/// Configure/LOCK/PERPENDICULAR/DUMPREG/READ·READ DELETED·WRITE DATA/FORMAT
/// TRACK (no-media) + IRQ6.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fdc82077 {
    /// Digital Output Register (motors, select, nRESET, DMA/IRQ enable).
    pub dor: u8,
    /// Tape Drive Register (stored).
    pub tdr: u8,
    /// Data Rate Select (write side of `0x3F4`; stored).
    pub dsr: u8,
    /// Configuration Control Register (write side of `0x3F7`; stored).
    pub ccr: u8,
    /// Status A read value (fixed stub).
    pub sra: u8,
    /// Status B read value (fixed stub).
    pub srb: u8,
    /// Digital Input Register read value (disk-change stub; bit7 often media).
    pub dir: u8,
    /// Per-drive Present Cylinder Numbers (PCN0–PCN3). Spec: Intel 82077AA
    /// Recalibrate/Seek/Sense Interrupt / DUMPREG Table 5-1 — each unit keeps
    /// its own PCN; Recalibrate/Seek update the selected unit only; Sense
    /// Interrupt reports `pcn[ST0[1:0]]`; DUMPREG returns all four.
    pub pcn: [u8; 4],
    /// Specify parameter 1: SRT (bits 7–4) | HUT (bits 3–0). Spec: 82077AA.
    pub specify_srt_hut: u8,
    /// Specify parameter 2: HLT (bits 7–1) | ND (bit 0). Spec: 82077AA.
    pub specify_hlt_nd: u8,
    /// Configure parameter 0 (typically 0; stored). Spec: Intel 82077AA / OSDev.
    pub configure_byte0: u8,
    /// Configure parameter 1: EIS (bit6) | FIFO_DIS (bit5) | POLL_DIS (bit4) |
    /// FIFOTHR (bits 3:0 = threshold−1). Spec: Intel 82077AA / OSDev Configure.
    pub configure_eis_fifo_poll_thr: u8,
    /// Configure parameter 2: PRETRK (write precompensation start track).
    pub configure_pretrk: u8,
    /// LOCK bit from LOCK command (`0x14`/`0x94`). Spec: Intel 82077AA §5.3.2 —
    /// when set, soft DOR/DSR reset must not restore Configure EFIFO/FIFOTHR/
    /// PRETRK defaults; hardware/`reset()` clears LOCK. LOCK does **not**
    /// protect PERPENDICULAR D3–D0 (those survive soft reset independently).
    pub lock: bool,
    /// PERPENDICULAR Mode drive bits D3–D0 (nibble). Spec: Intel 82077AA
    /// §5.3.1 — updated only when OW=1; soft DOR reset preserves; hardware
    /// reset clears; appear in DUMPREG byte7 bits 5:2 (OW not returned).
    pub perp_d3_d0: u8,
    /// PERPENDICULAR Mode GAP (bit1) | WGATE (bit0). Spec: Intel 82077AA
    /// §5.2.11 / Table 5-11 / §5.3.1 — always updated by the command; soft DOR
    /// reset clears to 0; appear in DUMPREG byte7 bits 1:0.
    pub perp_gap_wgate: u8,
    /// Last SC (FORMAT TRACK) or EOT (READ/READ DELETED/WRITE/…) parameter.
    /// Spec: Intel 82077AA Table 5-1 note — DUMPREG result byte 6. Updated by
    /// READ/READ DELETED/WRITE DATA EOT or FORMAT TRACK SC.
    pub sc_eot: u8,
    /// Latched IRQ request (command-complete / reset stub). Spec: 82077AA → ISA IRQ6.
    irq_pending: bool,
    phase: Phase,
    /// Command-completion ST0 for Sense Interrupt (Recalibrate/Seek Seek End); consumed once.
    pending_sense_st0: Option<u8>,
    /// Seek / Relative Seek param0 (HD|US) latched between the two parameter bytes.
    seek_head_unit: u8,
    /// Relative Seek DIR from command bit6: true = step in (+RCN), false = step out (−RCN).
    relative_seek_dir_in: bool,
    /// Sense Interrupt ST0 result byte (set when entering result phase).
    sense_st0: u8,
    /// Sense Interrupt PCN result byte (PCN of unit in `sense_st0` US bits).
    sense_pcn: u8,
    /// Sense Drive Status ST3 result byte (set when entering result phase).
    sense_st3: u8,
    /// READ/READ DELETED/WRITE DATA / FORMAT TRACK parameter bytes. Spec:
    /// Intel 82077AA §5.1.1 / §5.1.3 / §5.1.2 / §5.1.7. Shared — transfer
    /// commands are mutually exclusive in the command stream (FORMAT uses the
    /// first 5 slots).
    read_params: [u8; FDC_READ_DATA_PARAM_LEN as usize],
    /// READ/READ DELETED/WRITE DATA / FORMAT TRACK result bytes (ST0..). Spec:
    /// §5.1.1 / §5.1.3 / §5.1.2 / §5.1.7.
    read_result: [u8; FDC_READ_DATA_RESULT_LEN as usize],
    /// Attached 1.44MB raw floppy image (`None` = no media). Preserved across
    /// [`Self::reset`] like [`crate::IdePrimary`]'s backing image.
    image: Option<Vec<u8>>,
    /// Write-protect pin / media flag for Sense Drive Status ST3 WP (bit6).
    ///
    /// Spec: Intel 82077AA §6.4 — ST3 bit6 reflects the WP pin. Default `false`.
    /// When media is attached and this is set, Sense Drive Status reports WP=1;
    /// without media WP stays 0. Preserved across [`Self::reset`]. Does not yet
    /// force WRITE DATA ST1 NW from the pin.
    pub write_protected: bool,
    /// Last sector bytes transferred by a successful READ DATA (single-sector
    /// slice). Cleared on ND/abnormal completion. Inspect via [`Self::last_sector`];
    /// Machine DMA consumes a one-shot pending arm via [`Self::take_pending_dma_sector`].
    last_sector: Option<[u8; FDC_SECTOR_SIZE]>,
    /// Armed when READ DATA media success latches `last_sector` and DOR DMA/IRQ
    /// enable (bit3) is set. Cleared by [`Self::take_pending_dma_sector`] or
    /// abnormal/reset paths. Prevents re-DMA of a stale latch on later port writes.
    dma_read_pending: bool,
    /// Armed when WRITE DATA media success needs ISA DMA ch2 Read (memory→device)
    /// and DOR DMA/IRQ enable is set with no pre-latched [`Self::last_write`].
    /// Cleared by [`Self::take_pending_dma_write`] or abnormal/reset paths.
    dma_write_pending: bool,
    /// Pending / last sector bytes for WRITE DATA (single-sector slice).
    ///
    /// Device-only stand-in for ISA DMA ch2 Read (memory→device): tests call
    /// [`Self::latch_write_sector`] before command completion;
    /// [`Self::finish_write_data`] writes into the image. MachineBus fills via
    /// DMA Read + [`Self::commit_dma_write_sector`]. Cleared on NW/abnormal
    /// completion.
    last_write: Option<[u8; FDC_SECTOR_SIZE]>,
}

impl Default for Fdc82077 {
    fn default() -> Self {
        Self::new()
    }
}

impl Fdc82077 {
    pub fn new() -> Self {
        Self {
            // Spec: Intel 82077AA — DOR reset bit cleared at pin RESET; host must
            // set bit2 to leave reset. Stub starts with DOR=0 (held in reset).
            dor: 0x00,
            tdr: 0x00,
            dsr: 0x00,
            ccr: 0x00,
            // Open-bus style defaults for largely unused status ports.
            sra: 0x00,
            srb: 0x00,
            dir: 0x00,
            pcn: [0x00; 4],
            specify_srt_hut: 0x00,
            specify_hlt_nd: 0x00,
            configure_byte0: 0x00,
            configure_eis_fifo_poll_thr: 0x00,
            configure_pretrk: 0x00,
            lock: false,
            perp_d3_d0: 0x00,
            perp_gap_wgate: 0x00,
            sc_eot: 0x00,
            irq_pending: false,
            phase: Phase::Command,
            pending_sense_st0: None,
            seek_head_unit: 0,
            relative_seek_dir_in: false,
            sense_st0: 0,
            sense_pcn: 0,
            sense_st3: 0,
            read_params: [0; FDC_READ_DATA_PARAM_LEN as usize],
            read_result: [0; FDC_READ_DATA_RESULT_LEN as usize],
            image: None,
            write_protected: false,
            last_sector: None,
            dma_read_pending: false,
            dma_write_pending: false,
            last_write: None,
        }
    }

    /// Attach a raw 1.44MB floppy image (exact [`FDC_1440_IMAGE_SIZE`] bytes).
    ///
    /// Rejects other sizes. Clears DIR DSKCHG (stub). Spec: IBM PC 1.44MB MFM;
    /// DIR bit7 semantics are stubbed (see [`FDC_DIR_DSKCHG`]). Does not change
    /// [`Self::write_protected`].
    pub fn attach_image(&mut self, image: Vec<u8>) -> Result<(), &'static str> {
        if image.len() != FDC_1440_IMAGE_SIZE {
            return Err("FDC attach_image requires exact 1.44MB (1_474_560) image");
        }
        self.image = Some(image);
        self.dir &= !FDC_DIR_DSKCHG;
        Ok(())
    }

    /// Set the media write-protect flag (Sense Drive Status ST3 WP pin).
    ///
    /// Spec: Intel 82077AA §6.4 — ST3 bit6 WP. With media attached and
    /// `protected == true`, Sense Drive Status returns WP=1; default and
    /// no-media cases report WP=0. WRITE DATA ST1 NW from this pin is deferred.
    pub fn set_write_protected(&mut self, protected: bool) {
        self.write_protected = protected;
    }

    /// Construct an FDC with a 1.44MB image already attached.
    ///
    /// # Panics
    ///
    /// Panics if `image.len() != FDC_1440_IMAGE_SIZE`.
    pub fn with_image(image: Vec<u8>) -> Self {
        let mut fdc = Self::new();
        fdc.attach_image(image)
            .expect("FDC with_image requires exact 1.44MB image");
        fdc
    }

    /// Eject media and set DIR DSKCHG (stub). Spec: OSDev FDC disk-change bit.
    pub fn eject(&mut self) {
        self.image = None;
        self.dir |= FDC_DIR_DSKCHG;
    }

    /// True when a 1.44MB image is attached.
    pub fn has_media(&self) -> bool {
        self.image.is_some()
    }

    /// Bytes of the last successful READ DATA sector transfer (512 bytes), if any.
    ///
    /// Inspection latch — not cleared by Machine DMA. See [`Self::take_pending_dma_sector`].
    pub fn last_sector(&self) -> Option<&[u8; FDC_SECTOR_SIZE]> {
        self.last_sector.as_ref()
    }

    /// Bytes of the last successful WRITE DATA sector transfer (512 bytes), if any.
    ///
    /// Also the pending buffer supplied by [`Self::latch_write_sector`] before
    /// command completion (device-only DMA Read stand-in). Cleared on NW/abnormal.
    pub fn last_write(&self) -> Option<&[u8; FDC_SECTOR_SIZE]> {
        self.last_write.as_ref()
    }

    /// Latch 512 sector bytes for the next WRITE DATA media success path.
    ///
    /// Spec: Intel 82077AA DMA mode — WRITE DATA execution receives sector bytes
    /// from the floppy DMA channel (ISA ch2 Read = memory→device). Device tests
    /// call this before the 8th WRITE DATA parameter completes (stand-in for
    /// MachineBus DMA). When present, [`Self::finish_write_data`] writes the
    /// latch immediately and does **not** arm [`Self::dma_write_pending`].
    pub fn latch_write_sector(&mut self, data: [u8; FDC_SECTOR_SIZE]) {
        self.last_write = Some(data);
    }

    /// Consume a one-shot pending DMA-read sector (copy of [`Self::last_sector`]).
    ///
    /// Spec: Intel 82077AA DMA mode — after READ DATA execution with DOR DMA/IRQ
    /// enable, the sector is presented on the floppy DMA channel (ISA ch2).
    /// MachineBus calls this after FIFO completion and feeds
    /// `dma_transfer(2, …)` Write (device→memory). Returns `None` when not armed.
    pub fn take_pending_dma_sector(&mut self) -> Option<[u8; FDC_SECTOR_SIZE]> {
        if !self.dma_read_pending {
            return None;
        }
        self.dma_read_pending = false;
        self.last_sector
    }

    /// Consume a one-shot pending WRITE DATA DMA arm (ISA ch2 Read = memory→device).
    ///
    /// Spec: Intel 82077AA DMA mode + 8237A Read transfer; OSDev ISA DMA floppy
    /// channel 2. MachineBus calls this after FIFO completion, fills a 512-byte
    /// buffer via `dma_transfer(2, …)` Read, then
    /// [`Self::commit_dma_write_sector`]. Returns `false` when not armed.
    pub fn take_pending_dma_write(&mut self) -> bool {
        if !self.dma_write_pending {
            return false;
        }
        self.dma_write_pending = false;
        true
    }

    /// Commit ISA DMA ch2 Read sector bytes into the image at the last WRITE
    /// DATA C/H/R (`read_params`) and latch [`Self::last_write`].
    ///
    /// Spec: Intel 82077AA §5.1.2 WRITE DATA execution + IBM 1.44MB geometry.
    /// Returns `false` if CHS/media reject the write.
    pub fn commit_dma_write_sector(&mut self, data: [u8; FDC_SECTOR_SIZE]) -> bool {
        let c = self.read_params[1];
        let h = self.read_params[2];
        let r = self.read_params[3];
        if self.write_sector(c, h, r, &data) {
            self.last_write = Some(data);
            true
        } else {
            false
        }
    }

    /// CHS → byte offset in a linear 1.44MB image. `r` is 1-based (sector ID).
    ///
    /// Spec: IBM PC / OSDev Floppy — offset =
    /// `((c * Heads + h) * Spt + (r - 1)) * 512`. Out-of-range → `None`.
    pub fn chs_byte_offset(c: u8, h: u8, r: u8) -> Option<usize> {
        if c >= FDC_1440_CYLINDERS
            || h >= FDC_1440_HEADS
            || r == 0
            || r > FDC_1440_SECTORS_PER_TRACK
        {
            return None;
        }
        let lba = (usize::from(c) * usize::from(FDC_1440_HEADS) + usize::from(h))
            * usize::from(FDC_1440_SECTORS_PER_TRACK)
            + (usize::from(r) - 1);
        Some(lba * FDC_SECTOR_SIZE)
    }

    /// Read one 512-byte sector from the attached image at CHS `(c,h,r)`.
    ///
    /// Returns `None` if no media or CHS is out of range.
    pub fn read_sector(&self, c: u8, h: u8, r: u8) -> Option<[u8; FDC_SECTOR_SIZE]> {
        let image = self.image.as_ref()?;
        let off = Self::chs_byte_offset(c, h, r)?;
        let end = off.checked_add(FDC_SECTOR_SIZE)?;
        if end > image.len() {
            return None;
        }
        let mut sector = [0u8; FDC_SECTOR_SIZE];
        sector.copy_from_slice(&image[off..end]);
        Some(sector)
    }

    /// Write one 512-byte sector into the attached image at CHS `(c,h,r)`.
    ///
    /// Spec: IBM PC / OSDev Floppy — same linear layout as [`Self::chs_byte_offset`].
    /// Returns `false` if no media or CHS is out of range.
    pub fn write_sector(&mut self, c: u8, h: u8, r: u8, data: &[u8; FDC_SECTOR_SIZE]) -> bool {
        let Some(off) = Self::chs_byte_offset(c, h, r) else {
            return false;
        };
        let Some(image) = self.image.as_mut() else {
            return false;
        };
        let Some(end) = off.checked_add(FDC_SECTOR_SIZE) else {
            return false;
        };
        if end > image.len() {
            return false;
        }
        image[off..end].copy_from_slice(data);
        true
    }

    /// Hardware reset: clears programmed controller state; preserves attached
    /// media, write-protect flag, and DSKCHG when ejected, matching
    /// [`crate::IdePrimary::reset`].
    pub fn reset(&mut self) {
        let image = self.image.take();
        let write_protected = self.write_protected;
        let dskchg = self.dir & FDC_DIR_DSKCHG;
        *self = Self::new();
        self.image = image;
        self.write_protected = write_protected;
        if self.has_media() {
            self.dir &= !FDC_DIR_DSKCHG;
        } else {
            // Preserve eject-history DSKCHG; never-attached stays 0.
            self.dir |= dskchg;
        }
    }

    /// True if this device owns the I/O port.
    ///
    /// Spec: OSDev FDC — `0x3F0`–`0x3F7` excluding `0x3F6` (IDE alt/control).
    pub fn owns_port(port: u16) -> bool {
        matches!(
            port,
            FDC_SRA | FDC_SRB | FDC_DOR | FDC_TDR | FDC_MSR | FDC_FIFO | FDC_DIR_CCR
        )
    }

    /// Main Status Register.
    ///
    /// Spec: Intel 82077AA / OSDev — RQM indicates FIFO may be touched; DIO
    /// distinguishes command (host→FDC) vs result (FDC→host) phases.
    pub fn msr(&self) -> u8 {
        if self.dor & FDC_DOR_RESET_N == 0 {
            0
        } else {
            match self.phase {
                // Spec: 82077AA — command/parameter phases are host→FDC (DIO=0).
                Phase::Command
                | Phase::SpecifyParams { .. }
                | Phase::RecalibrateParams
                | Phase::SeekParams { .. }
                | Phase::RelativeSeekParams { .. }
                | Phase::SenseDriveStatusParam
                | Phase::ConfigureParams { .. }
                | Phase::PerpendicularParam
                | Phase::ReadDataParams { .. }
                | Phase::ReadDeletedDataParams { .. }
                | Phase::VerifyParams { .. }
                | Phase::WriteDataParams { .. }
                | Phase::WriteDeletedDataParams { .. }
                | Phase::FormatTrackParams { .. }
                | Phase::ReadIdParam => FDC_MSR_RQM,
                Phase::SenseIntResult { .. }
                | Phase::SenseDriveStatusResult
                | Phase::VersionResult
                | Phase::LockResult
                | Phase::DumpRegResult { .. }
                | Phase::ReadDataResult { .. }
                | Phase::ReadDeletedDataResult { .. }
                | Phase::VerifyResult { .. }
                | Phase::WriteDataResult { .. }
                | Phase::WriteDeletedDataResult { .. }
                | Phase::FormatTrackResult { .. }
                | Phase::ReadIdResult { .. } => FDC_MSR_RQM | FDC_MSR_DIO,
            }
        }
    }

    /// ISA IRQ6 line level (pending ∧ nRESET ∧ DMA/IRQ enable).
    ///
    /// Spec: Intel 82077AA DOR bit3; OSDev FDC / IBM PC AT — floppy → IRQ6.
    pub fn irq_line(&self) -> bool {
        self.irq_pending && (self.dor & FDC_DOR_RESET_N != 0) && (self.dor & FDC_DOR_DMA_IRQ != 0)
    }

    /// Assert IRQ6 as if a command completed (stub API until full engine exists).
    ///
    /// Spec: 82077AA interrupts the host on completion when DOR DMA/IRQ is enabled.
    pub fn assert_irq6(&mut self) {
        self.irq_pending = true;
    }

    /// Clear the latched IRQ request (Sense Interrupt / EOI-side stub).
    pub fn clear_irq6(&mut self) {
        self.irq_pending = false;
    }

    fn enter_dor_reset(&mut self) {
        self.irq_pending = false;
        self.phase = Phase::Command;
        self.pending_sense_st0 = None;
        self.seek_head_unit = 0;
        self.relative_seek_dir_in = false;
        self.sense_st0 = 0;
        self.sense_st3 = 0;
        // Spec: Intel 82077AA §5.3.2 — soft DOR reset does not clear LOCK; when
        // LOCK=0, EFIFO/FIFOTHR/PRETRK return to defaults (stub zeros).
        if !self.lock {
            self.configure_eis_fifo_poll_thr = 0;
            self.configure_pretrk = 0;
        }
        // Spec: Intel 82077AA §5.3.1 — soft DOR/DSR reset clears GAP|WGATE only;
        // D3–D0 retain (independent of LOCK).
        self.perp_gap_wgate = 0;
    }

    /// Begin Specify parameter phase (2 bytes). Spec: Intel 82077AA Specify.
    fn start_specify(&mut self) {
        self.phase = Phase::SpecifyParams { index: 0 };
    }

    /// Begin Recalibrate parameter phase (1 byte). Spec: Intel 82077AA Recalibrate.
    fn start_recalibrate(&mut self) {
        self.phase = Phase::RecalibrateParams;
    }

    /// Complete Recalibrate after unit-select parameter.
    ///
    /// Spec: Intel 82077AA Recalibrate — retracts selected unit head to track 0;
    /// on completion that unit's PCN=0, ST0 SE|US (`0x20 | unit`), interrupt
    /// asserted; host uses Sense Interrupt Status (no Recalibrate result phase).
    fn finish_recalibrate(&mut self, param: u8) {
        let unit = (param & 0x03) as usize;
        self.pcn[unit] = 0;
        self.pending_sense_st0 = Some(FDC_ST0_SEEK_END | (unit as u8));
        self.irq_pending = true;
        self.phase = Phase::Command;
    }

    /// Begin Seek parameter phase (2 bytes). Spec: Intel 82077AA Seek.
    fn start_seek(&mut self) {
        self.seek_head_unit = 0;
        self.phase = Phase::SeekParams { index: 0 };
    }

    /// Complete Seek after NCN parameter.
    ///
    /// Spec: Intel 82077AA Seek — steps selected unit to NCN; on completion that
    /// unit's PCN=NCN, ST0 SE|US (`0x20 | unit`; H in ST0 always 0), interrupt
    /// asserted; host uses Sense Interrupt Status (no Seek result phase).
    /// OSDev: param0 = (HD<<2)|US.
    fn finish_seek(&mut self, ncn: u8) {
        let unit = (self.seek_head_unit & 0x03) as usize;
        self.pcn[unit] = ncn;
        self.pending_sense_st0 = Some(FDC_ST0_SEEK_END | (unit as u8));
        self.irq_pending = true;
        self.phase = Phase::Command;
    }

    /// True when `v` is Relative Seek (`1 DIR 0 0 1 1 1 1`). Spec: 82077AA Table 5-1.
    pub fn is_relative_seek_command(v: u8) -> bool {
        (v & !FDC_CMD_RELATIVE_SEEK_DIR) == FDC_CMD_RELATIVE_SEEK
    }

    /// Begin Relative Seek parameter phase (2 bytes). Spec: Intel 82077AA §5.2.9.
    fn start_relative_seek(&mut self, cmd: u8) {
        self.seek_head_unit = 0;
        self.relative_seek_dir_in = cmd & FDC_CMD_RELATIVE_SEEK_DIR != 0;
        self.phase = Phase::RelativeSeekParams { index: 0 };
    }

    /// Complete Relative Seek after RCN parameter.
    ///
    /// Spec: Intel 82077AA §5.2.9 / Table 5-1 — steps selected unit by ±RCN
    /// (DIR=1 in / +, DIR=0 out / −); this stub clamps PCN to `0..=255` (no ST0
    /// EC latch yet); on completion latches ST0 SE|US (`0x20 | unit`; H always 0)
    /// and asserts IRQ like Seek; host uses Sense Interrupt Status (no result phase).
    fn finish_relative_seek(&mut self, rcn: u8) {
        let unit = (self.seek_head_unit & 0x03) as usize;
        let cur = self.pcn[unit];
        self.pcn[unit] = if self.relative_seek_dir_in {
            cur.saturating_add(rcn)
        } else {
            cur.saturating_sub(rcn)
        };
        self.pending_sense_st0 = Some(FDC_ST0_SEEK_END | (unit as u8));
        self.irq_pending = true;
        self.phase = Phase::Command;
    }

    /// Begin Sense Interrupt Status result phase.
    ///
    /// Spec: Intel 82077AA Sense Interrupt Status — no parameters; result ST0,
    /// PCN; clears interrupt. When a seek-class command latched ST0 (Recalibrate
    /// / Seek / Relative Seek), return that value; otherwise ST0 IC=11 (`0xC0`)
    /// models post-reset “ready line changed” / `assert_irq6`-only status; unit
    /// select from DOR[1:0]. PCN is the Present Cylinder Number for the unit in
    /// ST0 bits 1:0.
    fn start_sense_interrupt(&mut self) {
        self.sense_st0 = self
            .pending_sense_st0
            .take()
            .unwrap_or(FDC_ST0_IC_READY_CHANGE | (self.dor & 0x03));
        self.sense_pcn = self.pcn[(self.sense_st0 & 0x03) as usize];
        self.irq_pending = false;
        self.phase = Phase::SenseIntResult { index: 0 };
    }

    /// Begin Sense Drive Status parameter phase (1 byte). Spec: 82077AA §5.2.5.
    fn start_sense_drive_status(&mut self) {
        self.phase = Phase::SenseDriveStatusParam;
    }

    /// Complete Sense Drive Status after the HD|US parameter.
    ///
    /// Spec: Intel 82077AA §5.2.5/§6.4 — no execution phase, goes directly to
    /// the result phase; ST3 bits 2:0 echo the HD|US parameter, T0 stub
    /// reflects `pcn[unit] == 0` for the selected unit, WP set when media is
    /// attached and [`Self::write_protected`] is true (else 0), reserved bits
    /// 3/5 always 1. No IRQ.
    fn finish_sense_drive_status(&mut self, param: u8) {
        let head_unit = param & (FDC_ST3_HEAD | FDC_ST3_UNIT_MASK);
        let unit = (param & FDC_ST3_UNIT_MASK) as usize;
        let mut st3 = head_unit | FDC_ST3_RESERVED_BIT3 | FDC_ST3_RESERVED_BIT5;
        if self.pcn[unit] == 0 {
            st3 |= FDC_ST3_TRACK0;
        }
        if self.has_media() && self.write_protected {
            st3 |= FDC_ST3_WRITE_PROTECT;
        }
        self.sense_st3 = st3;
        self.phase = Phase::SenseDriveStatusResult;
    }

    /// Begin Version result phase. Spec: Intel 82077AA / OSDev FDC Version.
    ///
    /// No parameters; one result byte `0x90` (82077AA identification). No IRQ.
    fn start_version(&mut self) {
        self.phase = Phase::VersionResult;
    }

    /// Begin Configure parameter phase (3 bytes). Spec: Intel 82077AA Configure.
    fn start_configure(&mut self) {
        self.phase = Phase::ConfigureParams { index: 0 };
    }

    /// Begin PERPENDICULAR Mode parameter phase (1 byte). Spec: 82077AA §5.2.11.
    fn start_perpendicular(&mut self) {
        self.phase = Phase::PerpendicularParam;
    }

    /// Complete PERPENDICULAR Mode after the parameter byte.
    ///
    /// Spec: Intel 82077AA §5.2.11 / Table 5-1 / §5.3.1 — param =
    /// `OW|0|D3 D2 D1 D0|GAP|WGATE`; always store GAP|WGATE; update D3–D0 only
    /// when OW=1; no result phase; no IRQ. OW is write-side only (not in DUMPREG).
    fn finish_perpendicular(&mut self, param: u8) {
        self.perp_gap_wgate = param & 0x03;
        if param & 0x80 != 0 {
            self.perp_d3_d0 = (param >> 2) & 0x0F;
        }
        self.phase = Phase::Command;
    }

    /// Begin LOCK result phase. Spec: Intel 82077AA §5.3.2 / OSDev Lock.
    ///
    /// Command byte encodes LOCK in bit7 (`0x14` unlock / `0x94` lock); no
    /// parameter bytes; one result byte `LOCK<<4`; no IRQ.
    fn start_lock(&mut self, cmd: u8) {
        self.lock = (cmd & 0x80) != 0;
        self.phase = Phase::LockResult;
    }

    /// Begin DUMPREG result phase. Spec: Intel 82077AA §5.2.10 / Table 5-1 /
    /// §5.3.3 / OSDev FDC.
    ///
    /// No parameters; 10 result bytes from stored registers; no IRQ.
    fn start_dumpreg(&mut self) {
        self.phase = Phase::DumpRegResult { index: 0 };
    }

    /// Begin READ DATA parameter phase (8 bytes). Spec: Intel 82077AA §5.1.1.
    fn start_read_data(&mut self) {
        self.read_params = [0; FDC_READ_DATA_PARAM_LEN as usize];
        self.phase = Phase::ReadDataParams { index: 0 };
    }

    /// Complete READ DATA after eight parameters.
    ///
    /// Spec: Intel 82077AA §5.1.1 / §6.1 / §6.2 / IBM 1.44MB geometry:
    ///
    /// - With media, `N == 2` (512-byte sectors), and C/H/R in range: transfer
    ///   **one** sector (MT ignored this slice), latch bytes in
    ///   [`Self::last_sector`], result ST0 IC=00 (normal) | H | US, ST1=0,
    ///   ST2=0, C/H/R/N = command ENDaddress.
    /// - Otherwise (no media / wrong N / out-of-range CHS): skip execution,
    ///   ST0 IC=01 (abnormal) | H | US, ST1 ND, ST2=0, C/H/R/N ENDaddress;
    ///   clear `last_sector`.
    ///
    /// Latches EOT into `sc_eot` (DUMPREG Table 5-1); asserts IRQ (cleared
    /// when the host reads the first result byte). No Sense Interrupt.
    /// On media success with DOR DMA/IRQ enable, arms [`Self::dma_read_pending`]
    /// for MachineBus ISA DMA ch2 Write.
    fn finish_read_data(&mut self) {
        let head_unit = self.read_params[0];
        let unit = head_unit & 0x03;
        let head = (head_unit >> 2) & 0x01;
        let c = self.read_params[1];
        let h = self.read_params[2];
        let r = self.read_params[3];
        let n = self.read_params[4];
        let eot = self.read_params[5];
        // GPL (params[6]) and DTL (params[7]) accepted; multi-sector MT ignored.

        self.sc_eot = eot;
        let st0_head = if head != 0 { FDC_ST0_HEAD } else { 0 };

        // Single-sector success: media + N=2 + valid CHS (R 1-based).
        if n == FDC_SECTOR_N {
            if let Some(sector) = self.read_sector(c, h, r) {
                self.last_sector = Some(sector);
                // Spec: 82077AA DOR bit3 — DMA/IRQ enable; arm one-shot for Machine.
                self.dma_read_pending = self.dor & FDC_DOR_DMA_IRQ != 0;
                self.read_result = [FDC_ST0_IC_NORMAL | st0_head | unit, 0x00, 0x00, c, h, r, n];
                self.irq_pending = true;
                self.phase = Phase::ReadDataResult { index: 0 };
                return;
            }
        }

        // No media / wrong N / OOR CHS → honest ND abnormal (existing stub).
        self.last_sector = None;
        self.dma_read_pending = false;
        self.read_result = [
            FDC_ST0_IC_ABNORMAL | st0_head | unit,
            FDC_ST1_ND,
            0x00,
            c,
            h,
            r,
            n,
        ];
        self.irq_pending = true;
        self.phase = Phase::ReadDataResult { index: 0 };
    }

    /// Begin READ DELETED DATA parameter phase (8 bytes). Spec: Intel 82077AA §5.1.3.
    fn start_read_deleted_data(&mut self) {
        self.read_params = [0; FDC_READ_DELETED_DATA_PARAM_LEN as usize];
        self.phase = Phase::ReadDeletedDataParams { index: 0 };
    }

    /// Complete READ DELETED DATA after eight parameters — no-media stub.
    ///
    /// Spec: Intel 82077AA §5.1.3 / Table 5-1 / §6.1 / §6.2 — same parameter and
    /// result shape as READ DATA. With no media image this stub skips the
    /// execution/DMA transfer phase and enters result immediately: ST0 IC=01
    /// (abnormal termination) | H | US; ST1 ND (No Data — honest no-media
    /// equivalent consistent with the READ DATA stub); ST2=0; C/H/R/N reflect
    /// command ENDaddress; latches EOT into `sc_eot`; asserts IRQ (cleared when
    /// the host reads the first result byte). No Sense Interrupt.
    fn finish_read_deleted_data(&mut self) {
        let head_unit = self.read_params[0];
        let unit = head_unit & 0x03;
        let head = (head_unit >> 2) & 0x01;
        let c = self.read_params[1];
        let h = self.read_params[2];
        let r = self.read_params[3];
        let n = self.read_params[4];
        let eot = self.read_params[5];
        // GPL (params[6]) and DTL (params[7]) accepted; unused without media.

        self.sc_eot = eot;
        let st0_head = if head != 0 { FDC_ST0_HEAD } else { 0 };
        self.read_result = [
            FDC_ST0_IC_ABNORMAL | st0_head | unit,
            FDC_ST1_ND,
            0x00,
            c,
            h,
            r,
            n,
        ];
        self.irq_pending = true;
        self.phase = Phase::ReadDeletedDataResult { index: 0 };
    }

    /// Begin WRITE DATA parameter phase (8 bytes). Spec: Intel 82077AA §5.1.2.
    fn start_write_data(&mut self) {
        self.read_params = [0; FDC_WRITE_DATA_PARAM_LEN as usize];
        self.phase = Phase::WriteDataParams { index: 0 };
    }

    /// Complete WRITE DATA after eight parameters.
    ///
    /// Spec: Intel 82077AA §5.1.2 / §6.1 / §6.2 / IBM 1.44MB geometry:
    ///
    /// - With media, `N == 2` (512-byte sectors), and C/H/R in range: transfer
    ///   **one** sector (MT ignored this slice):
    ///   - If [`Self::last_write`] was pre-latched via [`Self::latch_write_sector`]
    ///     (device-only DMA stand-in): write into the image immediately.
    ///   - Else if DOR DMA/IRQ enable: arm [`Self::dma_write_pending`] for
    ///     MachineBus ISA DMA ch2 Read (memory→device) +
    ///     [`Self::commit_dma_write_sector`]; do not write zeros yet.
    ///   - Else (no DMA, no latch): write a zero sector (device fallback).
    ///     Result ST0 IC=00 (normal) | H | US, ST1=0, ST2=0, C/H/R/N ENDaddress.
    /// - Otherwise (no media / wrong N / out-of-range CHS): skip execution,
    ///   ST0 IC=01 (abnormal) | H | US, ST1 NW, ST2=0, C/H/R/N ENDaddress;
    ///   clear `last_write` / `dma_write_pending`.
    ///
    /// Latches EOT into `sc_eot` (DUMPREG Table 5-1); asserts IRQ (cleared
    /// when the host reads the first result byte). No Sense Interrupt.
    fn finish_write_data(&mut self) {
        let head_unit = self.read_params[0];
        let unit = head_unit & 0x03;
        let head = (head_unit >> 2) & 0x01;
        let c = self.read_params[1];
        let h = self.read_params[2];
        let r = self.read_params[3];
        let n = self.read_params[4];
        let eot = self.read_params[5];
        // GPL (params[6]) and DTL (params[7]) accepted; multi-sector MT ignored.

        self.sc_eot = eot;
        let st0_head = if head != 0 { FDC_ST0_HEAD } else { 0 };
        let dma = self.dor & FDC_DOR_DMA_IRQ != 0;

        // Single-sector success: media + N=2 + valid CHS (R 1-based).
        if n == FDC_SECTOR_N {
            if let Some(data) = self.last_write {
                // Pre-latched device stand-in — write now; Machine DMA not armed.
                if self.write_sector(c, h, r, &data) {
                    self.last_write = Some(data);
                    self.dma_write_pending = false;
                    self.read_result =
                        [FDC_ST0_IC_NORMAL | st0_head | unit, 0x00, 0x00, c, h, r, n];
                    self.irq_pending = true;
                    self.phase = Phase::WriteDataResult { index: 0 };
                    return;
                }
            } else if dma && Self::chs_byte_offset(c, h, r).is_some() && self.has_media() {
                // Spec: 82077AA DOR bit3 — arm one-shot for MachineBus ch2 Read.
                self.dma_write_pending = true;
                self.last_write = None;
                self.read_result = [FDC_ST0_IC_NORMAL | st0_head | unit, 0x00, 0x00, c, h, r, n];
                self.irq_pending = true;
                self.phase = Phase::WriteDataResult { index: 0 };
                return;
            } else {
                let data = [0u8; FDC_SECTOR_SIZE];
                if self.write_sector(c, h, r, &data) {
                    self.last_write = Some(data);
                    self.dma_write_pending = false;
                    self.read_result =
                        [FDC_ST0_IC_NORMAL | st0_head | unit, 0x00, 0x00, c, h, r, n];
                    self.irq_pending = true;
                    self.phase = Phase::WriteDataResult { index: 0 };
                    return;
                }
            }
        }

        // No media / wrong N / OOR CHS → honest NW abnormal (existing stub).
        self.last_write = None;
        self.dma_write_pending = false;
        self.read_result = [
            FDC_ST0_IC_ABNORMAL | st0_head | unit,
            FDC_ST1_NW,
            0x00,
            c,
            h,
            r,
            n,
        ];
        self.irq_pending = true;
        self.phase = Phase::WriteDataResult { index: 0 };
    }

    /// True if `cmd` is READ DATA including optional MT/MFM/SK modifiers.
    ///
    /// Spec: Intel 82077AA §5.1.1 / Table 5-1 — opcode bits 4:0 = `00110`;
    /// bits 7:5 are MT/MFM/SK (`FDC_CMD_MT` / `FDC_CMD_MFM` / `FDC_CMD_SK`).
    #[inline]
    fn is_read_data_command(cmd: u8) -> bool {
        // Compare against opcode nibble of the documented MT|MFM|SK|READ form.
        cmd & FDC_CMD_OPCODE_MASK == (FDC_CMD_READ_DATA_MT_MFM_SK & FDC_CMD_OPCODE_MASK)
    }

    /// True if `cmd` is READ DELETED DATA including optional MT/MFM/SK modifiers.
    ///
    /// Spec: Intel 82077AA §5.1.3 / Table 5-1 — opcode bits 4:0 = `01100`;
    /// bits 7:5 are MT/MFM/SK (`FDC_CMD_MT` / `FDC_CMD_MFM` / `FDC_CMD_SK`).
    #[inline]
    fn is_read_deleted_data_command(cmd: u8) -> bool {
        cmd & FDC_CMD_OPCODE_MASK == (FDC_CMD_READ_DELETED_DATA_MT_MFM_SK & FDC_CMD_OPCODE_MASK)
    }

    /// Begin VERIFY parameter phase (8 bytes). Spec: Intel 82077AA Table 5-1.
    fn start_verify(&mut self) {
        self.read_params = [0; FDC_VERIFY_PARAM_LEN as usize];
        self.phase = Phase::VerifyParams { index: 0 };
    }

    /// Complete VERIFY after eight parameters — no-media stub (no DMA).
    ///
    /// Spec: Intel 82077AA VERIFY — same param/result shape as READ DATA; no media
    /// → abnormal ST0 IC=01|H|US, ST1 ND, ST2=0, C/H/R/N; EOT→`sc_eot`; IRQ6.
    fn finish_verify(&mut self) {
        let head_unit = self.read_params[0];
        let unit = head_unit & 0x03;
        let head = (head_unit >> 2) & 0x01;
        let c = self.read_params[1];
        let h = self.read_params[2];
        let r = self.read_params[3];
        let n = self.read_params[4];
        let eot = self.read_params[5];
        self.sc_eot = eot;
        let st0_head = if head != 0 { FDC_ST0_HEAD } else { 0 };
        self.read_result = [
            FDC_ST0_IC_ABNORMAL | st0_head | unit,
            FDC_ST1_ND,
            0x00,
            c,
            h,
            r,
            n,
        ];
        self.irq_pending = true;
        self.phase = Phase::VerifyResult { index: 0 };
    }

    /// True if `cmd` is VERIFY including optional MT/MFM/SK modifiers.
    #[inline]
    fn is_verify_command(cmd: u8) -> bool {
        cmd & FDC_CMD_OPCODE_MASK == (FDC_CMD_VERIFY_MT_MFM_SK & FDC_CMD_OPCODE_MASK)
    }

    /// True if `cmd` is SCAN EQUAL / LOW OR EQUAL / HIGH OR EQUAL (MT/MFM/SK).
    ///
    /// Spec: Intel 82077AA Table 5-1 — opcodes `0x11` / `0x19` / `0x1D`.
    #[inline]
    fn is_scan_command(cmd: u8) -> bool {
        matches!(
            cmd & FDC_CMD_OPCODE_MASK,
            FDC_CMD_SCAN_EQUAL | FDC_CMD_SCAN_LOW_OR_EQUAL | FDC_CMD_SCAN_HIGH_OR_EQUAL
        )
    }

    /// True if `cmd` is WRITE DATA including optional MT/MFM modifiers.
    ///
    /// Spec: Intel 82077AA §5.1.2 / Table 5-1 — opcode bits 4:0 = `00101`;
    /// bits 7:6 are MT/MFM (`FDC_CMD_MT` / `FDC_CMD_MFM`); bit5 is 0 in the
    /// documented form (SK not used for WRITE DATA).
    #[inline]
    fn is_write_data_command(cmd: u8) -> bool {
        cmd & FDC_CMD_OPCODE_MASK == (FDC_CMD_WRITE_DATA_MT_MFM & FDC_CMD_OPCODE_MASK)
    }

    /// Begin WRITE DELETED DATA parameter phase (8 bytes). Spec: Intel 82077AA §5.1.4.
    fn start_write_deleted_data(&mut self) {
        self.read_params = [0; FDC_WRITE_DELETED_DATA_PARAM_LEN as usize];
        self.phase = Phase::WriteDeletedDataParams { index: 0 };
    }

    /// Complete WRITE DELETED DATA after eight parameters — no-media stub.
    ///
    /// Spec: Intel 82077AA §5.1.4 / Table 5-1 — same param/result shape as WRITE
    /// DATA; no media → skip execution/DMA; ST0 IC=01|H|US; ST1 NW; ST2=0;
    /// C/H/R/N ENDaddress; EOT→`sc_eot`; IRQ6 when DOR enables.
    fn finish_write_deleted_data(&mut self) {
        let head_unit = self.read_params[0];
        let unit = head_unit & 0x03;
        let head = (head_unit >> 2) & 0x01;
        let c = self.read_params[1];
        let h = self.read_params[2];
        let r = self.read_params[3];
        let n = self.read_params[4];
        let eot = self.read_params[5];

        self.sc_eot = eot;
        let st0_head = if head != 0 { FDC_ST0_HEAD } else { 0 };
        self.read_result = [
            FDC_ST0_IC_ABNORMAL | st0_head | unit,
            FDC_ST1_NW,
            0x00,
            c,
            h,
            r,
            n,
        ];
        self.irq_pending = true;
        self.phase = Phase::WriteDeletedDataResult { index: 0 };
    }

    /// True if `cmd` is WRITE DELETED DATA including optional MT/MFM modifiers.
    ///
    /// Spec: Intel 82077AA §5.1.4 / Table 5-1 — opcode bits 4:0 = `01001`.
    #[inline]
    fn is_write_deleted_data_command(cmd: u8) -> bool {
        cmd & FDC_CMD_OPCODE_MASK == (FDC_CMD_WRITE_DELETED_DATA_MT_MFM & FDC_CMD_OPCODE_MASK)
    }

    /// Begin FORMAT TRACK parameter phase (5 bytes). Spec: Intel 82077AA §5.1.7.
    fn start_format_track(&mut self) {
        self.read_params = [0; FDC_READ_DATA_PARAM_LEN as usize];
        self.phase = Phase::FormatTrackParams { index: 0 };
    }

    /// Complete FORMAT TRACK after five parameters — no-media stub.
    ///
    /// Spec: Intel 82077AA §5.1.7 / Table 5-1 / §6.1 / §6.2 — with no media
    /// image this stub skips the execution phase (DMA and the per-sector
    /// C/H/R/N ID stream) and enters result immediately: ST0 IC=01 (abnormal
    /// termination) | H | US; ST1 NW (Not Writable — §6.2 lists FORMAT TRACK
    /// with WRITE DATA); ST2=0; four undefined result bytes = 0; latches SC
    /// into `sc_eot` (DUMPREG Table 5-1); asserts IRQ (cleared when the host
    /// reads the first result byte). No Sense Interrupt.
    fn finish_format_track(&mut self) {
        let head_unit = self.read_params[0];
        let unit = head_unit & 0x03;
        let head = (head_unit >> 2) & 0x01;
        // N (params[1]), GPL (params[3]), D (params[4]) accepted; unused without media.
        let sc = self.read_params[2];

        self.sc_eot = sc;
        let st0_head = if head != 0 { FDC_ST0_HEAD } else { 0 };
        self.read_result = [
            FDC_ST0_IC_ABNORMAL | st0_head | unit,
            FDC_ST1_NW,
            0x00,
            0x00, // undefined
            0x00,
            0x00,
            0x00,
        ];
        self.irq_pending = true;
        self.phase = Phase::FormatTrackResult { index: 0 };
    }

    /// True if `cmd` is FORMAT TRACK including optional MFM modifier.
    ///
    /// Spec: Intel 82077AA §5.1.7 / Table 5-1 — opcode bits 4:0 = `01101`;
    /// bit6 is MFM (`FDC_CMD_MFM`); bits 7 and 5 are 0 in the documented form
    /// (MT/SK not used for FORMAT TRACK).
    #[inline]
    fn is_format_track_command(cmd: u8) -> bool {
        cmd & FDC_CMD_OPCODE_MASK == (FDC_CMD_FORMAT_TRACK_MFM & FDC_CMD_OPCODE_MASK)
    }

    /// Begin READ ID parameter phase (1 byte HD|US). Spec: Intel 82077AA Table 5-1.
    fn start_read_id(&mut self) {
        self.phase = Phase::ReadIdParam;
    }

    /// Complete READ ID after HD|US parameter.
    ///
    /// Spec: Intel 82077AA Table 5-1 / §5.1.8 — one HD|US param; 7-byte result
    /// ST0/ST1/ST2/C/H/R/N; IRQ6 when DOR enables (cleared on first result byte).
    ///
    /// - With media: normal termination (ST0 IC=00 | H | US), ST1=ST2=0, and a
    ///   sector-ID stub: C = `pcn[unit]`, H from the HD bit of the param, R=1,
    ///   N=`FDC_SECTOR_N` (512-byte / IBM 1.44MB). Full IDAM track scan deferred.
    /// - No media: ST0 IC=01 | H | US, ST1 ND, ST2=0, C/H/R/N=0.
    fn finish_read_id(&mut self, head_unit: u8) {
        let unit = head_unit & 0x03;
        let head = (head_unit >> 2) & 0x01;
        let st0_head = if head != 0 { FDC_ST0_HEAD } else { 0 };
        if self.has_media() {
            let c = self.pcn[unit as usize];
            self.read_result = [
                FDC_ST0_IC_NORMAL | st0_head | unit,
                0x00,
                0x00,
                c,
                head,
                0x01,
                FDC_SECTOR_N,
            ];
        } else {
            self.read_result = [
                FDC_ST0_IC_ABNORMAL | st0_head | unit,
                FDC_ST1_ND,
                0x00,
                0x00,
                0x00,
                0x00,
                0x00,
            ];
        }
        self.irq_pending = true;
        self.phase = Phase::ReadIdResult { index: 0 };
    }

    /// True if `cmd` is READ ID including optional MFM modifier.
    ///
    /// Spec: Intel 82077AA Table 5-1 — opcode bits 4:0 = `01010`; bit6 = MFM.
    #[inline]
    fn is_read_id_command(cmd: u8) -> bool {
        cmd & FDC_CMD_OPCODE_MASK == (FDC_CMD_READ_ID_MFM & FDC_CMD_OPCODE_MASK)
    }

    /// One DUMPREG result byte by index. Spec: Intel 82077AA Table 5-1 / §5.3.3.
    fn dumpreg_byte(&self, index: u8) -> u8 {
        match index {
            0..=3 => self.pcn[index as usize], // PCN0–PCN3 per drive
            4 => self.specify_srt_hut,
            5 => self.specify_hlt_nd,
            6 => self.sc_eot,
            // LOCK | 0 | D3 D2 D1 D0 | GAP | WGATE. Spec: 82077AA §5.3.3.
            7 => {
                (u8::from(self.lock) << 7)
                    | ((self.perp_d3_d0 & 0x0F) << 2)
                    | (self.perp_gap_wgate & 0x03)
            }
            8 => self.configure_eis_fifo_poll_thr & 0x7F, // bit7 always 0
            9 => self.configure_pretrk,
            _ => 0xFF,
        }
    }

    fn fifo_read(&mut self) -> u8 {
        match self.phase {
            // Spec: Specify/Recalibrate/Seek/Relative Seek/Configure/PERPENDICULAR
            // have no result phase; open-bus when idle/params.
            Phase::Command
            | Phase::SpecifyParams { .. }
            | Phase::RecalibrateParams
            | Phase::SeekParams { .. }
            | Phase::RelativeSeekParams { .. }
            | Phase::SenseDriveStatusParam
            | Phase::ConfigureParams { .. }
            | Phase::PerpendicularParam
            | Phase::ReadDataParams { .. }
            | Phase::ReadDeletedDataParams { .. }
            | Phase::VerifyParams { .. }
            | Phase::WriteDataParams { .. }
            | Phase::WriteDeletedDataParams { .. }
            | Phase::FormatTrackParams { .. }
            | Phase::ReadIdParam => 0xFF,
            Phase::SenseIntResult { index } => {
                let v = match index {
                    0 => self.sense_st0,
                    _ => self.sense_pcn,
                };
                if index >= 1 {
                    self.phase = Phase::Command;
                } else {
                    self.phase = Phase::SenseIntResult { index: 1 };
                }
                v
            }
            Phase::SenseDriveStatusResult => {
                self.phase = Phase::Command;
                self.sense_st3
            }
            Phase::VersionResult => {
                self.phase = Phase::Command;
                FDC_VERSION_82077AA
            }
            Phase::LockResult => {
                self.phase = Phase::Command;
                u8::from(self.lock) << FDC_LOCK_RESULT_SHIFT
            }
            Phase::DumpRegResult { index } => {
                let v = self.dumpreg_byte(index);
                if index + 1 >= FDC_DUMPREG_RESULT_LEN {
                    self.phase = Phase::Command;
                } else {
                    self.phase = Phase::DumpRegResult { index: index + 1 };
                }
                v
            }
            Phase::ReadDataResult { index } => {
                // Spec: 82077AA / OSDev — IRQ for read/write cleared as the host
                // begins reading the result phase (first byte).
                if index == 0 {
                    self.irq_pending = false;
                }
                let v = self.read_result[index as usize];
                if index + 1 >= FDC_READ_DATA_RESULT_LEN {
                    self.phase = Phase::Command;
                } else {
                    self.phase = Phase::ReadDataResult { index: index + 1 };
                }
                v
            }
            Phase::ReadDeletedDataResult { index } => {
                // Spec: 82077AA / OSDev — IRQ for read/write cleared as the host
                // begins reading the result phase (first byte).
                if index == 0 {
                    self.irq_pending = false;
                }
                let v = self.read_result[index as usize];
                if index + 1 >= FDC_READ_DELETED_DATA_RESULT_LEN {
                    self.phase = Phase::Command;
                } else {
                    self.phase = Phase::ReadDeletedDataResult { index: index + 1 };
                }
                v
            }
            Phase::VerifyResult { index } => {
                // Spec: 82077AA / OSDev — IRQ for read/write cleared as the host
                // begins reading the result phase (first byte).
                if index == 0 {
                    self.irq_pending = false;
                }
                let v = self.read_result[index as usize];
                if index + 1 >= FDC_VERIFY_RESULT_LEN {
                    self.phase = Phase::Command;
                } else {
                    self.phase = Phase::VerifyResult { index: index + 1 };
                }
                v
            }
            Phase::WriteDataResult { index } => {
                // Spec: 82077AA / OSDev — IRQ for read/write cleared as the host
                // begins reading the result phase (first byte).
                if index == 0 {
                    self.irq_pending = false;
                }
                let v = self.read_result[index as usize];
                if index + 1 >= FDC_WRITE_DATA_RESULT_LEN {
                    self.phase = Phase::Command;
                } else {
                    self.phase = Phase::WriteDataResult { index: index + 1 };
                }
                v
            }
            Phase::WriteDeletedDataResult { index } => {
                // Spec: 82077AA / OSDev — IRQ for read/write cleared as the host
                // begins reading the result phase (first byte).
                if index == 0 {
                    self.irq_pending = false;
                }
                let v = self.read_result[index as usize];
                if index + 1 >= FDC_WRITE_DELETED_DATA_RESULT_LEN {
                    self.phase = Phase::Command;
                } else {
                    self.phase = Phase::WriteDeletedDataResult { index: index + 1 };
                }
                v
            }
            Phase::FormatTrackResult { index } => {
                // Spec: 82077AA / OSDev — IRQ for format cleared as the host
                // begins reading the result phase (first byte).
                if index == 0 {
                    self.irq_pending = false;
                }
                let v = self.read_result[index as usize];
                if index + 1 >= FDC_FORMAT_TRACK_RESULT_LEN {
                    self.phase = Phase::Command;
                } else {
                    self.phase = Phase::FormatTrackResult { index: index + 1 };
                }
                v
            }
            Phase::ReadIdResult { index } => {
                if index == 0 {
                    self.irq_pending = false;
                }
                let v = self.read_result[index as usize];
                if index + 1 >= FDC_READ_ID_RESULT_LEN {
                    self.phase = Phase::Command;
                } else {
                    self.phase = Phase::ReadIdResult { index: index + 1 };
                }
                v
            }
        }
    }

    fn fifo_write(&mut self, v: u8) {
        // Spec: Intel 82077AA — controller held in reset ignores command stream.
        if self.dor & FDC_DOR_RESET_N == 0 {
            return;
        }
        match self.phase {
            Phase::Command => {
                if v == FDC_CMD_SPECIFY {
                    // Spec: Intel 82077AA Specify — no IRQ; expect two params.
                    self.start_specify();
                } else if v == FDC_CMD_RECALIBRATE {
                    // Spec: Intel 82077AA Recalibrate — expect one unit-select param.
                    self.start_recalibrate();
                } else if v == FDC_CMD_SENSE_INT {
                    self.start_sense_interrupt();
                } else if v == FDC_CMD_SEEK {
                    // Spec: Intel 82077AA Seek — expect HD|US then NCN.
                    self.start_seek();
                } else if Self::is_relative_seek_command(v) {
                    // Spec: Intel 82077AA §5.2.9 — DIR in cmd bit6; HD|US then RCN.
                    self.start_relative_seek(v);
                } else if v == FDC_CMD_SENSE_DRIVE_STATUS {
                    // Spec: Intel 82077AA §5.2.5 — expect HD|US param; no IRQ.
                    self.start_sense_drive_status();
                } else if v == FDC_CMD_DUMPREG {
                    // Spec: Intel 82077AA §5.2.10 — no params; 10-byte result; no IRQ.
                    self.start_dumpreg();
                } else if v == FDC_CMD_VERSION {
                    // Spec: Intel 82077AA Version — no params; result 0x90; no IRQ.
                    self.start_version();
                } else if v == FDC_CMD_CONFIGURE {
                    // Spec: Intel 82077AA Configure — three params; no result/IRQ.
                    self.start_configure();
                } else if v == FDC_CMD_PERPENDICULAR {
                    // Spec: Intel 82077AA §5.2.11 / §5.3.1 — one param; no result/IRQ.
                    self.start_perpendicular();
                } else if v == FDC_CMD_LOCK || v == FDC_CMD_LOCK_SET {
                    // Spec: Intel 82077AA §5.3.2 — LOCK in bit7; no params; result LOCK<<4.
                    self.start_lock(v);
                } else if Self::is_read_data_command(v) {
                    // Spec: Intel 82077AA §5.1.1 — MT/MFM/SK | 00110; eight params.
                    self.start_read_data();
                } else if Self::is_read_deleted_data_command(v) {
                    // Spec: Intel 82077AA §5.1.3 — MT/MFM/SK | 01100; eight params.
                    self.start_read_deleted_data();
                } else if Self::is_verify_command(v) {
                    // Spec: Intel 82077AA Table 5-1 — MT/MFM/SK | 10110; eight params.
                    self.start_verify();
                } else if Self::is_scan_command(v) {
                    // Spec: Intel 82077AA Table 5-1 — SCAN *; reuse VERIFY no-media path.
                    self.start_verify();
                } else if Self::is_write_data_command(v) {
                    // Spec: Intel 82077AA §5.1.2 — MT/MFM | 00101; eight params.
                    self.start_write_data();
                } else if Self::is_write_deleted_data_command(v) {
                    // Spec: Intel 82077AA §5.1.4 — MT/MFM | 01001; eight params.
                    self.start_write_deleted_data();
                } else if Self::is_format_track_command(v) {
                    // Spec: Intel 82077AA §5.1.7 — MFM | 01101; five params.
                    self.start_format_track();
                } else if Self::is_read_id_command(v) {
                    // Spec: Intel 82077AA Table 5-1 — MFM | 01010; one HD|US param.
                    self.start_read_id();
                }
                // Other opcodes: accept/drop until a command engine exists.
            }
            Phase::SpecifyParams { index } => {
                // Spec: Intel 82077AA Specify — param0 = SRT|HUT, param1 = HLT|ND.
                match index {
                    0 => {
                        self.specify_srt_hut = v;
                        self.phase = Phase::SpecifyParams { index: 1 };
                    }
                    _ => {
                        self.specify_hlt_nd = v;
                        self.phase = Phase::Command;
                    }
                }
            }
            Phase::RecalibrateParams => {
                // Spec: Intel 82077AA Recalibrate — bits 1:0 = unit select.
                self.finish_recalibrate(v);
            }
            Phase::SeekParams { index } => {
                // Spec: Intel 82077AA / OSDev — param0 = (HD<<2)|US, param1 = NCN.
                match index {
                    0 => {
                        self.seek_head_unit = v;
                        self.phase = Phase::SeekParams { index: 1 };
                    }
                    _ => {
                        self.finish_seek(v);
                    }
                }
            }
            Phase::RelativeSeekParams { index } => {
                // Spec: Intel 82077AA §5.2.9 — param0 = HD|US, param1 = RCN.
                match index {
                    0 => {
                        self.seek_head_unit = v;
                        self.phase = Phase::RelativeSeekParams { index: 1 };
                    }
                    _ => {
                        self.finish_relative_seek(v);
                    }
                }
            }
            Phase::SenseDriveStatusParam => {
                // Spec: Intel 82077AA §5.2.5 — HD|US param, no execution phase.
                self.finish_sense_drive_status(v);
            }
            Phase::ConfigureParams { index } => {
                // Spec: Intel 82077AA / OSDev Configure — param0 unused, param1
                // EIS|FIFO_DIS|POLL_DIS|FIFOTHR, param2 PRETRK; no result/IRQ.
                match index {
                    0 => {
                        self.configure_byte0 = v;
                        self.phase = Phase::ConfigureParams { index: 1 };
                    }
                    1 => {
                        self.configure_eis_fifo_poll_thr = v;
                        self.phase = Phase::ConfigureParams { index: 2 };
                    }
                    _ => {
                        self.configure_pretrk = v;
                        self.phase = Phase::Command;
                    }
                }
            }
            Phase::PerpendicularParam => {
                // Spec: Intel 82077AA §5.2.11 / §5.3.1 — OW|0|D3–D0|GAP|WGATE.
                self.finish_perpendicular(v);
            }
            Phase::ReadDataParams { index } => {
                // Spec: Intel 82077AA §5.1.1 — HD|US, C, H, R, N, EOT, GPL, DTL.
                self.read_params[index as usize] = v;
                if index + 1 >= FDC_READ_DATA_PARAM_LEN {
                    self.finish_read_data();
                } else {
                    self.phase = Phase::ReadDataParams { index: index + 1 };
                }
            }
            Phase::ReadDeletedDataParams { index } => {
                // Spec: Intel 82077AA §5.1.3 — HD|US, C, H, R, N, EOT, GPL, DTL.
                self.read_params[index as usize] = v;
                if index + 1 >= FDC_READ_DELETED_DATA_PARAM_LEN {
                    self.finish_read_deleted_data();
                } else {
                    self.phase = Phase::ReadDeletedDataParams { index: index + 1 };
                }
            }
            Phase::VerifyParams { index } => {
                self.read_params[index as usize] = v;
                if index + 1 >= FDC_VERIFY_PARAM_LEN {
                    self.finish_verify();
                } else {
                    self.phase = Phase::VerifyParams { index: index + 1 };
                }
            }

            Phase::WriteDataParams { index } => {
                // Spec: Intel 82077AA §5.1.2 — HD|US, C, H, R, N, EOT, GPL, DTL.
                self.read_params[index as usize] = v;
                if index + 1 >= FDC_WRITE_DATA_PARAM_LEN {
                    self.finish_write_data();
                } else {
                    self.phase = Phase::WriteDataParams { index: index + 1 };
                }
            }
            Phase::WriteDeletedDataParams { index } => {
                self.read_params[index as usize] = v;
                if index + 1 >= FDC_WRITE_DELETED_DATA_PARAM_LEN {
                    self.finish_write_deleted_data();
                } else {
                    self.phase = Phase::WriteDeletedDataParams { index: index + 1 };
                }
            }

            Phase::FormatTrackParams { index } => {
                // Spec: Intel 82077AA §5.1.7 — HD|US, N, SC, GPL, D.
                self.read_params[index as usize] = v;
                if index + 1 >= FDC_FORMAT_TRACK_PARAM_LEN {
                    self.finish_format_track();
                } else {
                    self.phase = Phase::FormatTrackParams { index: index + 1 };
                }
            }
            Phase::ReadIdParam => {
                // Spec: Intel 82077AA — HD|US param; no-media result + IRQ6.
                self.finish_read_id(v);
            }
            Phase::SenseIntResult { .. }
            | Phase::SenseDriveStatusResult
            | Phase::VersionResult
            | Phase::LockResult
            | Phase::DumpRegResult { .. }
            | Phase::ReadDataResult { .. }
            | Phase::ReadDeletedDataResult { .. }
            | Phase::VerifyResult { .. }
            | Phase::WriteDataResult { .. }
            | Phase::WriteDeletedDataResult { .. }
            | Phase::FormatTrackResult { .. }
            | Phase::ReadIdResult { .. } => {
                // Host must not write during result phase (stub ignores).
            }
        }
    }
}

impl PortDevice for Fdc82077 {
    fn port_read(&mut self, port: u16, _size: u8) -> u32 {
        let v = match port {
            FDC_SRA => self.sra,
            FDC_SRB => self.srb,
            FDC_DOR => self.dor,
            FDC_TDR => self.tdr,
            FDC_MSR => self.msr(),
            FDC_FIFO => self.fifo_read(),
            FDC_DIR_CCR => self.dir,
            _ => 0xFF,
        };
        u32::from(v)
    }

    fn port_write(&mut self, port: u16, _size: u8, value: u32) {
        let v = value as u8;
        match port {
            FDC_SRA | FDC_SRB => {
                // Read-only status ports — ignore writes (stub).
            }
            FDC_DOR => {
                self.dor = v;
                // Spec: Intel 82077AA — DOR reset clears controller state including IRQ.
                if self.dor & FDC_DOR_RESET_N == 0 {
                    self.enter_dor_reset();
                }
            }
            FDC_TDR => self.tdr = v,
            FDC_MSR => self.dsr = v, // DSR write-only side
            FDC_FIFO => self.fifo_write(v),
            FDC_DIR_CCR => self.ccr = v, // CCR write-only side
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owns_fdc_ports_not_ide_3f6() {
        // Spec: OSDev FDC — 0x3F0–0x3F7 excluding 0x3F6 (IDE).
        assert!(Fdc82077::owns_port(FDC_SRA));
        assert!(Fdc82077::owns_port(FDC_SRB));
        assert!(Fdc82077::owns_port(FDC_DOR));
        assert!(Fdc82077::owns_port(FDC_TDR));
        assert!(Fdc82077::owns_port(FDC_MSR));
        assert!(Fdc82077::owns_port(FDC_FIFO));
        assert!(Fdc82077::owns_port(FDC_DIR_CCR));
        assert!(!Fdc82077::owns_port(0x3F6));
        assert!(!Fdc82077::owns_port(0x3F8));
    }

    #[test]
    fn reset_msr_zero_until_dor_release() {
        // Spec: Intel 82077AA — DOR bit2 must be set to leave reset.
        let mut f = Fdc82077::new();
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, 0);
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N));
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
    }

    #[test]
    fn dor_dsr_ccr_round_trip() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, 0x1C); // nRESET + DMA/IRQ + motor0 style
        assert_eq!(f.port_read(FDC_DOR, 1) as u8, 0x1C);
        f.port_write(FDC_MSR, 1, 0x02); // DSR
        assert_eq!(f.dsr, 0x02);
        // MSR read side is status, not DSR.
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        f.port_write(FDC_DIR_CCR, 1, 0x00);
        assert_eq!(f.ccr, 0x00);
    }

    #[test]
    fn tdr_round_trip() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_TDR, 1, 0x03);
        assert_eq!(f.port_read(FDC_TDR, 1) as u8, 0x03);
    }

    #[test]
    fn reset_clears_programmed_state() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, 0x1C);
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_INT));
        f.port_write(FDC_DIR_CCR, 1, 0x01);
        f.assert_irq6();
        f.reset();
        assert_eq!(f.dor, 0);
        assert_eq!(f.ccr, 0);
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, 0);
        assert!(!f.irq_line());
        assert_eq!(f.phase, Phase::Command);
    }

    /// Spec: Intel 82077AA DOR bit3 + IBM PC AT IRQ6 — assert gated by nRESET∧DMA/IRQ.
    #[test]
    fn assert_irq6_gated_by_dor_dma_irq_and_reset() {
        let mut f = Fdc82077::new();
        f.assert_irq6();
        assert!(!f.irq_line(), "held in DOR reset");

        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N)); // nRESET only
        f.assert_irq6();
        assert!(!f.irq_line(), "DMA/IRQ enable clear");

        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert!(f.irq_line());

        f.clear_irq6();
        assert!(!f.irq_line());
        f.assert_irq6();
        assert!(f.irq_line());

        // Entering DOR reset clears pending.
        f.port_write(FDC_DOR, 1, 0);
        assert!(!f.irq_line());
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert!(!f.irq_line(), "reset cleared pending");
    }

    /// Spec: Intel 82077AA Sense Interrupt Status — ST0+PCN result; clears IRQ.
    #[test]
    fn sense_interrupt_status_result_and_clears_irq() {
        let mut f = Fdc82077::new();
        f.port_write(
            FDC_DOR,
            1,
            u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ | 0x01),
        );
        f.pcn[1] = 0x12; // ST0 US will be DOR[1:0]=1
        f.assert_irq6();
        assert!(f.irq_line());

        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_INT));
        assert!(!f.irq_line(), "Sense Interrupt clears IRQ latch");
        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM | FDC_MSR_DIO,
            "result phase: RQM|DIO"
        );

        let st0 = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(st0, 0xC1, "ST0 = IC=11 | US=01 from DOR");
        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM | FDC_MSR_DIO,
            "still in result after ST0"
        );
        let pcn = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(pcn, 0x12, "PCN of unit 1");
        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM,
            "idle command phase after result"
        );
        assert!(!f.irq_line());
    }

    #[test]
    fn sense_interrupt_ignored_while_held_in_dor_reset() {
        let mut f = Fdc82077::new();
        // MSR=0 while reset; writing FIFO is still accepted by PortDevice but
        // phase must stay clear once nRESET is set without a prior command.
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_INT));
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N));
        // DOR write while leaving reset does not auto-run Sense Interrupt.
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
    }

    /// Spec: Intel 82077AA Specify — opcode `0x03`, two params, no result, no IRQ.
    #[test]
    fn specify_accepts_two_params_returns_to_command() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.assert_irq6();
        assert!(f.irq_line());

        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SPECIFY));
        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM,
            "param phase: RQM, DIO clear"
        );
        assert!(f.irq_line(), "Specify must not clear IRQ latch");

        // Typical BIOS values: SRT=0xC, HUT=0xF → 0xCF; HLT=0x01<<1 | ND=0 → 0x02.
        f.port_write(FDC_FIFO, 1, 0xCF);
        assert_eq!(f.specify_srt_hut, 0xCF);
        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM,
            "still param phase after first byte"
        );
        f.port_write(FDC_FIFO, 1, 0x02);
        assert_eq!(f.specify_hlt_nd, 0x02);
        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM,
            "idle command phase after Specify"
        );
        assert_eq!(f.phase, Phase::Command);
        assert!(f.irq_line(), "Specify must not assert/clear IRQ");

        // No result bytes — FIFO read stays open-bus style.
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0xFF);
    }

    #[test]
    fn specify_ignored_while_held_in_dor_reset() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SPECIFY));
        f.port_write(FDC_FIFO, 1, 0xCF);
        f.port_write(FDC_FIFO, 1, 0x02);
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N));
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        assert_eq!(f.specify_srt_hut, 0);
        assert_eq!(f.specify_hlt_nd, 0);
        assert_eq!(f.phase, Phase::Command);
    }

    #[test]
    fn dor_reset_aborts_specify_param_phase() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N));
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SPECIFY));
        f.port_write(FDC_FIFO, 1, 0xAB);
        assert_eq!(f.specify_srt_hut, 0xAB);
        f.port_write(FDC_DOR, 1, 0); // enter reset
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N));
        assert_eq!(f.phase, Phase::Command);
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        // Stored params survive soft DOR reset (full `reset()` clears); phase aborts.
        assert_eq!(f.specify_srt_hut, 0xAB);
    }

    #[test]
    fn reset_clears_specify_params() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N));
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SPECIFY));
        f.port_write(FDC_FIFO, 1, 0xCF);
        f.port_write(FDC_FIFO, 1, 0x02);
        f.reset();
        assert_eq!(f.specify_srt_hut, 0);
        assert_eq!(f.specify_hlt_nd, 0);
        assert_eq!(f.phase, Phase::Command);
    }

    /// Spec: Intel 82077AA Recalibrate — opcode `0x07`, unit param, PCN=0, SE ST0, IRQ.
    #[test]
    fn recalibrate_sets_pcn_zero_seek_end_st0_and_irq() {
        let mut f = Fdc82077::new();
        f.port_write(
            FDC_DOR,
            1,
            u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ | 0x01),
        );
        f.pcn = [0x2A; 4];
        assert!(!f.irq_line());

        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_RECALIBRATE));
        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM,
            "param phase: RQM, DIO clear"
        );
        assert!(!f.irq_line(), "IRQ only after parameter");
        assert_eq!(f.phase, Phase::RecalibrateParams);

        // Unit select = 2 (bits 1:0); upper bits ignored by stub.
        f.port_write(FDC_FIFO, 1, 0x12);
        assert_eq!(f.pcn[2], 0, "Recalibrate forces selected unit PCN=0");
        assert_eq!(f.pcn[0], 0x2A, "other units' PCN unchanged");
        assert_eq!(f.phase, Phase::Command);
        assert!(f.irq_line(), "Recalibrate asserts IRQ on completion");
        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM,
            "no result phase after Recalibrate"
        );
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0xFF);

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_INT));
        assert!(!f.irq_line(), "Sense Interrupt clears IRQ");
        let st0 = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(st0, FDC_ST0_SEEK_END | 0x02, "ST0 = SE | unit from param");
        let pcn = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(pcn, 0);
    }

    /// Spec: Sense Interrupt without command ST0 latch keeps post-reset / assert_irq6 ST0.
    #[test]
    fn sense_interrupt_uses_ready_change_st0_when_no_command_latch() {
        let mut f = Fdc82077::new();
        f.port_write(
            FDC_DOR,
            1,
            u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ | 0x01),
        );
        f.assert_irq6();
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_INT));
        assert_eq!(
            f.port_read(FDC_FIFO, 1) as u8,
            FDC_ST0_IC_READY_CHANGE | 0x01
        );
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0);
    }

    /// Spec: after Recalibrate ST0 is consumed, a later Sense Interrupt falls back to 0xC0|US.
    #[test]
    fn sense_interrupt_consumes_recalibrate_st0_latch() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_RECALIBRATE));
        f.port_write(FDC_FIFO, 1, 0x01);
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_INT));
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, FDC_ST0_SEEK_END | 0x01);
        let _pcn = f.port_read(FDC_FIFO, 1) as u8;

        f.assert_irq6();
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_INT));
        assert_eq!(
            f.port_read(FDC_FIFO, 1) as u8,
            FDC_ST0_IC_READY_CHANGE,
            "no pending command ST0 after first Sense"
        );
        let _ = f.port_read(FDC_FIFO, 1);
    }

    #[test]
    fn recalibrate_ignored_while_held_in_dor_reset() {
        let mut f = Fdc82077::new();
        f.pcn = [0x05; 4];
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_RECALIBRATE));
        f.port_write(FDC_FIFO, 1, 0x01);
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        assert_eq!(f.pcn, [0x05; 4], "ignored while reset");
        assert!(!f.irq_line());
        assert_eq!(f.phase, Phase::Command);
    }

    #[test]
    fn dor_reset_aborts_recalibrate_param_phase() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_RECALIBRATE));
        assert_eq!(f.phase, Phase::RecalibrateParams);
        f.port_write(FDC_DOR, 1, 0); // enter reset
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert_eq!(f.phase, Phase::Command);
        assert!(!f.irq_line());
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        // Aborted: no Seek End latch — Sense Interrupt uses ready-change stub.
        f.assert_irq6();
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_INT));
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, FDC_ST0_IC_READY_CHANGE);
        let _ = f.port_read(FDC_FIFO, 1);
    }

    /// Spec: Intel 82077AA Seek — opcode `0x0F`, HD|US + NCN, PCN=NCN, SE ST0, IRQ.
    #[test]
    fn seek_sets_pcn_to_ncn_seek_end_st0_and_irq() {
        let mut f = Fdc82077::new();
        f.port_write(
            FDC_DOR,
            1,
            u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ | 0x01),
        );
        f.pcn = [0x00; 4];
        assert!(!f.irq_line());

        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SEEK));
        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM,
            "param phase: RQM, DIO clear"
        );
        assert!(!f.irq_line(), "IRQ only after both parameters");
        assert_eq!(f.phase, Phase::SeekParams { index: 0 });

        // Param0: head=1 (bit2) | unit=2 (bits1:0) → 0x06; ST0 H always 0 per 82077AA.
        f.port_write(FDC_FIFO, 1, 0x06);
        assert_eq!(f.phase, Phase::SeekParams { index: 1 });
        assert_eq!(f.pcn[2], 0x00, "PCN unchanged until NCN");
        assert!(!f.irq_line());

        f.port_write(FDC_FIFO, 1, 0x28); // NCN
        assert_eq!(f.pcn[2], 0x28, "Seek sets selected unit PCN = NCN");
        assert_eq!(f.pcn[0], 0x00, "other units' PCN unchanged");
        assert_eq!(f.phase, Phase::Command);
        assert!(f.irq_line(), "Seek asserts IRQ on completion");
        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM,
            "no result phase after Seek"
        );
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0xFF);

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_INT));
        assert!(!f.irq_line(), "Sense Interrupt clears IRQ");
        let st0 = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(st0, FDC_ST0_SEEK_END | 0x02, "ST0 = SE | unit; H=0");
        let pcn = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(pcn, 0x28);
    }

    /// Spec: Intel 82077AA §5.2.9 Relative Seek — opcode `1|DIR|001111`
    /// (`0x8F` out / `0xCF` in), HD|US + RCN; PCN ±= RCN; Seek End ST0 + IRQ.
    #[test]
    fn relative_seek_out_decrements_pcn_seek_end_st0_and_irq() {
        let mut f = Fdc82077::new();
        f.port_write(
            FDC_DOR,
            1,
            u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ | 0x01),
        );
        f.pcn = [0x00, 0x00, 0x28, 0x00];
        assert!(!f.irq_line());

        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_RELATIVE_SEEK)); // DIR=0 out
        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM,
            "param phase: RQM, DIO clear"
        );
        assert!(!f.irq_line(), "IRQ only after both parameters");
        assert_eq!(f.phase, Phase::RelativeSeekParams { index: 0 });

        // Param0: head=1 (bit2) | unit=2 → 0x06; ST0 H always 0 per 82077AA.
        f.port_write(FDC_FIFO, 1, 0x06);
        assert_eq!(f.phase, Phase::RelativeSeekParams { index: 1 });
        assert_eq!(f.pcn[2], 0x28, "PCN unchanged until RCN");
        assert!(!f.irq_line());

        f.port_write(FDC_FIFO, 1, 0x08); // RCN
        assert_eq!(f.pcn[2], 0x20, "DIR=0 out: PCN -= RCN");
        assert_eq!(f.pcn[0], 0x00, "other units' PCN unchanged");
        assert_eq!(f.phase, Phase::Command);
        assert!(f.irq_line(), "Relative Seek asserts IRQ on completion");
        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM,
            "no result phase after Relative Seek"
        );

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_INT));
        assert!(!f.irq_line(), "Sense Interrupt clears IRQ");
        let st0 = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(st0, FDC_ST0_SEEK_END | 0x02, "ST0 = SE | unit; H=0");
        let pcn = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(pcn, 0x20);
    }

    /// Spec: Intel 82077AA §5.2.9 — DIR=1 (`0xCF`) steps in: PCN += RCN.
    #[test]
    fn relative_seek_in_increments_pcn_seek_end_st0_and_irq() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.pcn[1] = 0x10;

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_RELATIVE_SEEK_IN)); // DIR=1 in
        f.port_write(FDC_FIFO, 1, 0x01); // unit 1
        f.port_write(FDC_FIFO, 1, 0x05); // RCN
        assert_eq!(f.pcn[1], 0x15, "DIR=1 in: PCN += RCN");
        assert_eq!(f.pcn[0], 0x00);
        assert!(f.irq_line());

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_INT));
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, FDC_ST0_SEEK_END | 0x01);
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x15);
    }

    /// Spec: stub clamps PCN to `0..=255` (ST0 EC beyond track 0 deferred).
    #[test]
    fn relative_seek_clamps_pcn_to_0_255() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));

        f.pcn[0] = 0x05;
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_RELATIVE_SEEK)); // out
        f.port_write(FDC_FIFO, 1, 0x00);
        f.port_write(FDC_FIFO, 1, 0x10); // RCN > PCN
        assert_eq!(f.pcn[0], 0x00, "out clamp at 0");
        assert!(f.irq_line());
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_INT));
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, FDC_ST0_SEEK_END);
        let _ = f.port_read(FDC_FIFO, 1);

        f.pcn[0] = 0xF0;
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_RELATIVE_SEEK_IN)); // in
        f.port_write(FDC_FIFO, 1, 0x00);
        f.port_write(FDC_FIFO, 1, 0x20); // would exceed 255
        assert_eq!(f.pcn[0], 0xFF, "in clamp at 255");
        assert!(f.irq_line());
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_INT));
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, FDC_ST0_SEEK_END);
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0xFF);
    }

    /// Spec: Intel 82077AA Sense Drive Status — opcode `0x04`, HD|US param, ST3
    /// result (no execution phase, no IRQ). Track 0 reflects `pcn[unit]==0`.
    #[test]
    fn sense_drive_status_result_reflects_track0_head_and_unit() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.pcn = [0x00; 4];

        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_DRIVE_STATUS));
        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM,
            "param phase: RQM, DIO clear"
        );
        assert_eq!(f.phase, Phase::SenseDriveStatusParam);

        // Param: HD=1 (bit2) | US1,US0 = 2 (bits1:0) -> 0x06 (same packing as Seek).
        f.port_write(FDC_FIFO, 1, 0x06);
        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM | FDC_MSR_DIO,
            "result phase: RQM|DIO"
        );

        let st3 = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(
            st3,
            FDC_ST3_TRACK0 | FDC_ST3_RESERVED_BIT5 | FDC_ST3_RESERVED_BIT3 | 0x06,
            "T0 (pcn[unit]==0) | reserved bits | HD|US from param"
        );
        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM,
            "idle command phase after result byte read"
        );
        assert_eq!(f.phase, Phase::Command);
    }

    /// Spec: Intel 82077AA §6.4 — ST3 WP reflects the WP pin. No media → WP=0
    /// even if the host flag is set (honest empty-drive stub).
    #[test]
    fn sense_drive_status_write_protect_clear_without_media() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.set_write_protected(true);
        assert!(!f.has_media());
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_DRIVE_STATUS));
        f.port_write(FDC_FIFO, 1, 0x00);
        let st3 = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(
            st3 & FDC_ST3_WRITE_PROTECT,
            0,
            "no media: WP stays clear even when write_protected set"
        );
    }

    /// Spec: Intel 82077AA §6.4 — with media, WP defaults clear; set_write_protected
    /// asserts ST3 bit6; clearing the flag clears WP again.
    #[test]
    fn sense_drive_status_write_protect_reflects_media_flag() {
        let mut f = Fdc82077::with_image(vec![0u8; FDC_1440_IMAGE_SIZE]);
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert!(f.has_media());
        assert!(!f.write_protected, "default WP flag clear");

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_DRIVE_STATUS));
        f.port_write(FDC_FIFO, 1, 0x00);
        let st3_default = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(
            st3_default & FDC_ST3_WRITE_PROTECT,
            0,
            "media attached, default: WP clear"
        );

        f.set_write_protected(true);
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_DRIVE_STATUS));
        f.port_write(FDC_FIFO, 1, 0x00);
        let st3_wp = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(
            st3_wp & FDC_ST3_WRITE_PROTECT,
            FDC_ST3_WRITE_PROTECT,
            "media + write_protected: ST3 WP set"
        );

        f.set_write_protected(false);
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_DRIVE_STATUS));
        f.port_write(FDC_FIFO, 1, 0x00);
        let st3_clear = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(
            st3_clear & FDC_ST3_WRITE_PROTECT,
            0,
            "clearing write_protected clears ST3 WP"
        );
    }

    /// Spec: write-protect is a media property; hardware reset preserves it
    /// with the attached image (same policy as IDE backing media).
    #[test]
    fn reset_preserves_write_protected_with_media() {
        let mut f = Fdc82077::with_image(vec![0u8; FDC_1440_IMAGE_SIZE]);
        f.set_write_protected(true);
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.reset();
        assert!(f.has_media());
        assert!(f.write_protected);
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_DRIVE_STATUS));
        f.port_write(FDC_FIFO, 1, 0x00);
        let st3 = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(st3 & FDC_ST3_WRITE_PROTECT, FDC_ST3_WRITE_PROTECT);
    }

    /// Spec: T0 bit reflects TRK0 pin state (stub: `pcn[unit]==0`); clear when
    /// that unit's PCN is nonzero.
    #[test]
    fn sense_drive_status_track0_clear_when_pcn_nonzero() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.pcn[1] = 0x28;

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_DRIVE_STATUS));
        f.port_write(FDC_FIFO, 1, 0x01); // unit 1, head 0
        let st3 = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(
            st3 & FDC_ST3_TRACK0,
            0,
            "T0 clear when selected unit pcn!=0"
        );
        assert_eq!(st3 & 0x07, 0x01, "HD|US preserved from param");
    }

    /// Spec: Sense Drive Status has no execution phase and must not assert or
    /// clear IRQ (unlike Recalibrate/Seek).
    #[test]
    fn sense_drive_status_does_not_touch_irq() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.assert_irq6();
        assert!(f.irq_line());

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_DRIVE_STATUS));
        assert!(f.irq_line(), "command byte must not clear IRQ");
        f.port_write(FDC_FIFO, 1, 0x00);
        assert!(f.irq_line(), "param byte must not clear or assert IRQ");
        let _ = f.port_read(FDC_FIFO, 1);
        assert!(f.irq_line(), "result read must not clear IRQ");

        // Starting from no pending IRQ, Sense Drive Status must not assert one.
        f.clear_irq6();
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_DRIVE_STATUS));
        f.port_write(FDC_FIFO, 1, 0x00);
        let _ = f.port_read(FDC_FIFO, 1);
        assert!(!f.irq_line(), "Sense Drive Status never asserts IRQ");
    }

    #[test]
    fn sense_drive_status_ignored_while_held_in_dor_reset() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_DRIVE_STATUS));
        f.port_write(FDC_FIFO, 1, 0x00);
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N));
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        assert_eq!(f.phase, Phase::Command);
    }

    #[test]
    fn dor_reset_aborts_sense_drive_status_param_phase() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_DRIVE_STATUS));
        assert_eq!(f.phase, Phase::SenseDriveStatusParam);
        f.port_write(FDC_DOR, 1, 0); // enter reset
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert_eq!(f.phase, Phase::Command);
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
    }

    /// Spec: after Seek ST0 is consumed, a later Sense Interrupt falls back to 0xC0|US.
    #[test]
    fn sense_interrupt_consumes_seek_st0_latch() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SEEK));
        f.port_write(FDC_FIFO, 1, 0x01);
        f.port_write(FDC_FIFO, 1, 0x10);
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_INT));
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, FDC_ST0_SEEK_END | 0x01);
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x10);

        f.assert_irq6();
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_INT));
        assert_eq!(
            f.port_read(FDC_FIFO, 1) as u8,
            FDC_ST0_IC_READY_CHANGE,
            "no pending command ST0 after first Sense"
        );
        let _ = f.port_read(FDC_FIFO, 1);
    }

    #[test]
    fn seek_ignored_while_held_in_dor_reset() {
        let mut f = Fdc82077::new();
        f.pcn = [0x05; 4];
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SEEK));
        f.port_write(FDC_FIFO, 1, 0x01);
        f.port_write(FDC_FIFO, 1, 0x20);
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        assert_eq!(f.pcn, [0x05; 4], "ignored while reset");
        assert!(!f.irq_line());
        assert_eq!(f.phase, Phase::Command);
    }

    #[test]
    fn dor_reset_aborts_seek_param_phase() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SEEK));
        f.port_write(FDC_FIFO, 1, 0x01);
        assert_eq!(f.phase, Phase::SeekParams { index: 1 });
        f.port_write(FDC_DOR, 1, 0); // enter reset
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert_eq!(f.phase, Phase::Command);
        assert!(!f.irq_line());
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        f.assert_irq6();
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_INT));
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, FDC_ST0_IC_READY_CHANGE);
        let _ = f.port_read(FDC_FIFO, 1);
    }

    /// Spec: Intel 82077AA / OSDev FDC Version — opcode `0x10`, no params,
    /// result byte `0x90` (82077AA identification); no IRQ.
    #[test]
    fn version_returns_82077aa_id_byte() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));

        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_VERSION));
        assert_eq!(f.phase, Phase::VersionResult);
        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM | FDC_MSR_DIO,
            "result phase: RQM|DIO"
        );
        assert!(!f.irq_line(), "Version must not assert IRQ");

        let version = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(version, FDC_VERSION_82077AA, "82077AA identification byte");
        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM,
            "idle command phase after result byte read"
        );
        assert_eq!(f.phase, Phase::Command);
        assert!(!f.irq_line());
    }

    /// Spec: Version has no execution phase and must not assert or clear IRQ.
    #[test]
    fn version_does_not_touch_irq() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.assert_irq6();
        assert!(f.irq_line());

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_VERSION));
        assert!(f.irq_line(), "command byte must not clear IRQ");
        let _ = f.port_read(FDC_FIFO, 1);
        assert!(f.irq_line(), "result read must not clear IRQ");

        f.clear_irq6();
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_VERSION));
        let _ = f.port_read(FDC_FIFO, 1);
        assert!(!f.irq_line(), "Version never asserts IRQ");
    }

    #[test]
    fn version_ignored_while_held_in_dor_reset() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_VERSION));
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N));
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        assert_eq!(f.phase, Phase::Command);
        assert_eq!(
            f.port_read(FDC_FIFO, 1) as u8,
            0xFF,
            "no Version result latched while held in reset"
        );
    }

    #[test]
    fn dor_reset_aborts_version_result_phase() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_VERSION));
        assert_eq!(f.phase, Phase::VersionResult);
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM | FDC_MSR_DIO);
        f.port_write(FDC_DOR, 1, 0); // enter reset
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert_eq!(f.phase, Phase::Command);
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        assert_eq!(
            f.port_read(FDC_FIFO, 1) as u8,
            0xFF,
            "aborted Version result is discarded"
        );
    }

    /// Spec: Intel 82077AA / OSDev FDC Configure — opcode `0x13`, three params
    /// (unused, EIS|FIFO_DIS|POLL_DIS|FIFOTHR, PRETRK), no result, no IRQ.
    #[test]
    fn configure_stores_three_params_returns_to_command() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.assert_irq6();
        assert!(f.irq_line());

        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_CONFIGURE));
        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM,
            "param phase: RQM, DIO clear"
        );
        assert!(f.irq_line(), "Configure must not clear IRQ latch");

        // param0 typically 0 (ignored by hardware; stored by stub).
        f.port_write(FDC_FIFO, 1, 0x00);
        assert_eq!(f.configure_byte0, 0x00);
        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM,
            "still param phase after byte0"
        );

        // param1: EIS=1, FIFO_DIS=0, POLL_DIS=1, FIFOTHR=7 → 0x57.
        f.port_write(FDC_FIFO, 1, 0x57);
        assert_eq!(f.configure_eis_fifo_poll_thr, 0x57);
        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM,
            "still param phase after byte1"
        );

        // param2: PRETRK write precompensation start track.
        f.port_write(FDC_FIFO, 1, 0x00);
        assert_eq!(f.configure_pretrk, 0x00);
        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM,
            "idle command phase after Configure"
        );
        assert_eq!(f.phase, Phase::Command);
        assert!(f.irq_line(), "Configure must not assert/clear IRQ");

        // No result bytes — FIFO read stays open-bus style.
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0xFF);
    }

    /// Spec: Configure has no execution/result phase and must not assert or clear IRQ.
    #[test]
    fn configure_does_not_touch_irq() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.assert_irq6();
        assert!(f.irq_line());

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_CONFIGURE));
        assert!(f.irq_line(), "command byte must not clear IRQ");
        f.port_write(FDC_FIFO, 1, 0x00);
        f.port_write(FDC_FIFO, 1, 0x08);
        f.port_write(FDC_FIFO, 1, 0x00);
        assert!(f.irq_line(), "params must not clear IRQ");

        f.clear_irq6();
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_CONFIGURE));
        f.port_write(FDC_FIFO, 1, 0x00);
        f.port_write(FDC_FIFO, 1, 0x08);
        f.port_write(FDC_FIFO, 1, 0x00);
        assert!(!f.irq_line(), "Configure never asserts IRQ");
    }

    #[test]
    fn dor_reset_aborts_configure_param_phase() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N));
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_CONFIGURE));
        f.port_write(FDC_FIFO, 1, 0xAB);
        assert_eq!(f.configure_byte0, 0xAB);
        assert_eq!(f.phase, Phase::ConfigureParams { index: 1 });
        f.port_write(FDC_DOR, 1, 0); // enter reset — aborts mid-command
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N));
        assert_eq!(f.phase, Phase::Command);
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        // Soft DOR reset aborts phase; with LOCK=0, EFIFO/FIFOTHR/PRETRK return
        // to stub defaults (0). Unused configure_byte0 is not LOCK-protected.
        assert_eq!(f.configure_byte0, 0xAB);
        assert_eq!(f.configure_eis_fifo_poll_thr, 0);
        assert_eq!(f.configure_pretrk, 0);
    }

    /// After Configure, probe commands Version / Sense Interrupt still work.
    #[test]
    fn configure_then_version_and_sense_int_still_work() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_CONFIGURE));
        f.port_write(FDC_FIFO, 1, 0x00);
        f.port_write(FDC_FIFO, 1, 0x57);
        f.port_write(FDC_FIFO, 1, 0x00);
        assert_eq!(f.phase, Phase::Command);
        assert_eq!(f.configure_eis_fifo_poll_thr, 0x57);

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_VERSION));
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, FDC_VERSION_82077AA);
        assert_eq!(f.phase, Phase::Command);

        f.assert_irq6();
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_INT));
        assert_eq!(
            f.port_read(FDC_FIFO, 1) as u8,
            FDC_ST0_IC_READY_CHANGE,
            "Sense Int ST0 after Configure"
        );
        let _pcn = f.port_read(FDC_FIFO, 1);
        assert_eq!(f.phase, Phase::Command);
        assert!(!f.irq_line());
    }

    #[test]
    fn reset_clears_configure_params() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N));
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_CONFIGURE));
        f.port_write(FDC_FIFO, 1, 0x01);
        f.port_write(FDC_FIFO, 1, 0x57);
        f.port_write(FDC_FIFO, 1, 0x12);
        assert_eq!(f.configure_byte0, 0x01);
        assert_eq!(f.configure_eis_fifo_poll_thr, 0x57);
        assert_eq!(f.configure_pretrk, 0x12);
        f.reset();
        // Soft reset defaults: zeros (like Specify). Real 82077AA post-hardware-
        // reset often has FIFO disabled / thr=1; this stub stores 0 until programmed.
        assert_eq!(f.configure_byte0, 0);
        assert_eq!(f.configure_eis_fifo_poll_thr, 0);
        assert_eq!(f.configure_pretrk, 0);
        assert_eq!(f.phase, Phase::Command);
    }

    /// Spec: Intel 82077AA §5.3.2 / OSDev Lock — opcode `0x94` (LOCK=1 in bit7),
    /// no parameter bytes; result `LOCK<<4` = `0x10`; no IRQ.
    #[test]
    fn lock_set_stores_flag_and_returns_result_0x10() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));

        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_LOCK_SET));
        assert!(f.lock, "LOCK bit set from command byte bit7");
        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM | FDC_MSR_DIO,
            "result phase: RQM|DIO"
        );
        assert!(!f.irq_line(), "LOCK must not assert IRQ");

        let result = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(
            result,
            1u8 << FDC_LOCK_RESULT_SHIFT,
            "result reflects LOCK in bit4"
        );
        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM,
            "idle command phase after result"
        );
        assert_eq!(f.phase, Phase::Command);
        assert!(f.lock);
    }

    /// Spec: Intel 82077AA §5.3.2 / OSDev — unlock opcode `0x14` (LOCK=0);
    /// result `0x00`; no params; no IRQ.
    #[test]
    fn lock_clear_stores_flag_and_returns_result_0x00() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.lock = true;

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_LOCK));
        assert!(!f.lock, "LOCK cleared by 0x14");
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM | FDC_MSR_DIO);
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x00);
        assert_eq!(f.phase, Phase::Command);
        assert!(!f.lock);
        assert!(!f.irq_line());
    }

    /// Spec: 82077AA §5.3.2 — "No interrupts are generated at the end of this command."
    #[test]
    fn lock_does_not_touch_irq() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.assert_irq6();
        assert!(f.irq_line());

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_LOCK_SET));
        assert!(f.irq_line(), "command byte must not clear IRQ");
        let _ = f.port_read(FDC_FIFO, 1);
        assert!(f.irq_line(), "result read must not clear IRQ");

        f.clear_irq6();
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_LOCK));
        let _ = f.port_read(FDC_FIFO, 1);
        assert!(!f.irq_line(), "LOCK never asserts IRQ");
    }

    #[test]
    fn lock_ignored_while_held_in_dor_reset() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_LOCK_SET));
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N));
        assert!(!f.lock, "ignored while held in DOR reset");
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        assert_eq!(f.phase, Phase::Command);
        assert_eq!(
            f.port_read(FDC_FIFO, 1) as u8,
            0xFF,
            "no LOCK result latched while held in reset"
        );
    }

    #[test]
    fn dor_reset_aborts_lock_result_phase() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_LOCK_SET));
        assert!(f.lock);
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM | FDC_MSR_DIO);
        f.port_write(FDC_DOR, 1, 0); // enter reset — aborts result phase
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert_eq!(f.phase, Phase::Command);
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        // Spec: 82077AA §5.3.2 — soft DOR reset does not clear LOCK.
        assert!(f.lock, "soft reset must not clear LOCK");
        assert_eq!(
            f.port_read(FDC_FIFO, 1) as u8,
            0xFF,
            "aborted LOCK result is discarded"
        );
    }

    /// Spec: 82077AA §5.3.2 — when LOCK=1, soft DOR reset must not restore
    /// Configure EFIFO/FIFOTHR/PRETRK defaults.
    #[test]
    fn dor_soft_reset_preserves_configure_when_locked() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_CONFIGURE));
        f.port_write(FDC_FIFO, 1, 0x00);
        f.port_write(FDC_FIFO, 1, 0x57);
        f.port_write(FDC_FIFO, 1, 0x12);
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_LOCK_SET));
        let _ = f.port_read(FDC_FIFO, 1);
        assert!(f.lock);

        f.port_write(FDC_DOR, 1, 0); // soft DOR reset
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert!(f.lock, "LOCK survives soft reset");
        assert_eq!(f.configure_eis_fifo_poll_thr, 0x57);
        assert_eq!(f.configure_pretrk, 0x12);
        assert_eq!(f.phase, Phase::Command);
    }

    /// Spec: 82077AA §5.3.2 — when LOCK=0, soft DOR reset returns Configure
    /// EFIFO/FIFOTHR/PRETRK to defaults (stub zeros).
    #[test]
    fn dor_soft_reset_clears_configure_fifo_params_when_unlocked() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_CONFIGURE));
        f.port_write(FDC_FIFO, 1, 0x01);
        f.port_write(FDC_FIFO, 1, 0x57);
        f.port_write(FDC_FIFO, 1, 0x12);
        assert!(!f.lock);

        f.port_write(FDC_DOR, 1, 0); // soft DOR reset
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert!(!f.lock);
        assert_eq!(
            f.configure_eis_fifo_poll_thr, 0,
            "unlocked soft reset clears FIFOTHR/EIS/FIFO/POLL"
        );
        assert_eq!(f.configure_pretrk, 0, "unlocked soft reset clears PRETRK");
        // Spec protects only EFIFO/FIFOTHR/PRETRK; unused configure_byte0 policy
        // matches prior stub (survives soft reset until full `reset()`).
        assert_eq!(f.configure_byte0, 0x01);
    }

    /// Spec: 82077AA §5.3.2 — hardware reset (pin / full `reset()`) clears LOCK.
    #[test]
    fn hardware_reset_clears_lock() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_LOCK_SET));
        let _ = f.port_read(FDC_FIFO, 1);
        assert!(f.lock);
        f.reset();
        assert!(!f.lock);
        assert_eq!(f.phase, Phase::Command);
    }

    /// Spec: Intel 82077AA §5.2.10 / Table 5-1 / §5.3.3 — DUMPREG (`0x0E`) has
    /// no parameters; 10-byte result from stored registers; MSR RQM|DIO; no IRQ.
    #[test]
    fn dumpreg_returns_ten_bytes_from_stored_state() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));

        // Seed Specify / Configure / LOCK / PCN / SC-EOT stub state.
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SPECIFY));
        f.port_write(FDC_FIFO, 1, 0xDF); // SRT|HUT
        f.port_write(FDC_FIFO, 1, 0x02); // HLT|ND
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_CONFIGURE));
        f.port_write(FDC_FIFO, 1, 0x00);
        f.port_write(FDC_FIFO, 1, 0x57); // EIS|EFIFO|POLL|FIFOTHR
        f.port_write(FDC_FIFO, 1, 0x0A); // PRETRK
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_LOCK_SET));
        let _ = f.port_read(FDC_FIFO, 1);
        f.pcn = [0x2A; 4];
        f.sc_eot = 0x12;

        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_DUMPREG));
        assert_eq!(f.phase, Phase::DumpRegResult { index: 0 });
        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM | FDC_MSR_DIO,
            "result phase: RQM|DIO"
        );
        assert!(!f.irq_line(), "DUMPREG must not assert IRQ");

        let mut result = [0u8; FDC_DUMPREG_RESULT_LEN as usize];
        for byte in &mut result {
            assert_eq!(
                f.port_read(FDC_MSR, 1) as u8,
                FDC_MSR_RQM | FDC_MSR_DIO,
                "RQM|DIO until last result byte"
            );
            *byte = f.port_read(FDC_FIFO, 1) as u8;
        }

        assert_eq!(
            &result[0..4],
            &[0x2A, 0x2A, 0x2A, 0x2A],
            "PCN0–3 from per-drive array"
        );
        assert_eq!(result[4], 0xDF, "SRT|HUT");
        assert_eq!(result[5], 0x02, "HLT|ND");
        assert_eq!(result[6], 0x12, "SC/EOT stub");
        assert_eq!(result[7], 0x80, "LOCK<<7; perp bits default 0");
        assert_eq!(result[8], 0x57, "0|EIS|EFIFO|POLL|FIFOTHR");
        assert_eq!(result[9], 0x0A, "PRETRK");
        assert_eq!(f.phase, Phase::Command);
        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM,
            "idle command phase after 10th result byte"
        );
        assert!(!f.irq_line());
    }

    /// Spec: DUMPREG generates no interrupt (diagnostic dump only).
    #[test]
    fn dumpreg_does_not_touch_irq() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.assert_irq6();
        assert!(f.irq_line());

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_DUMPREG));
        assert!(f.irq_line(), "command byte must not clear IRQ");
        for _ in 0..FDC_DUMPREG_RESULT_LEN {
            let _ = f.port_read(FDC_FIFO, 1);
        }
        assert!(f.irq_line(), "result reads must not clear IRQ");

        f.clear_irq6();
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_DUMPREG));
        for _ in 0..FDC_DUMPREG_RESULT_LEN {
            let _ = f.port_read(FDC_FIFO, 1);
        }
        assert!(!f.irq_line(), "DUMPREG never asserts IRQ");
    }

    #[test]
    fn dumpreg_ignored_while_held_in_dor_reset() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_DUMPREG));
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N));
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        assert_eq!(f.phase, Phase::Command);
        assert_eq!(
            f.port_read(FDC_FIFO, 1) as u8,
            0xFF,
            "no DUMPREG result latched while held in reset"
        );
    }

    #[test]
    fn dor_reset_aborts_dumpreg_result_phase() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.pcn = [0x55; 4];
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_DUMPREG));
        assert_eq!(f.phase, Phase::DumpRegResult { index: 0 });
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM | FDC_MSR_DIO);
        // Consume one result byte so we are mid-result.
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x55);
        assert_eq!(f.phase, Phase::DumpRegResult { index: 1 });

        f.port_write(FDC_DOR, 1, 0); // enter reset
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert_eq!(f.phase, Phase::Command);
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        assert_eq!(
            f.port_read(FDC_FIFO, 1) as u8,
            0xFF,
            "aborted DUMPREG result is discarded"
        );
    }

    /// After DUMPREG, Version / LOCK still work (SeaBIOS probe sequencing).
    #[test]
    fn dumpreg_then_version_and_lock_still_work() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_DUMPREG));
        for _ in 0..FDC_DUMPREG_RESULT_LEN {
            let _ = f.port_read(FDC_FIFO, 1);
        }
        assert_eq!(f.phase, Phase::Command);

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_VERSION));
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, FDC_VERSION_82077AA);

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_LOCK_SET));
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 1u8 << FDC_LOCK_RESULT_SHIFT);
        assert!(f.lock);
        assert_eq!(f.phase, Phase::Command);
    }

    /// Spec: Intel 82077AA §5.2.11 / Table 5-1 / §5.3.1 — PERPENDICULAR Mode
    /// (`0x12`) takes one parameter `OW|0|D3–D0|GAP|WGATE`; no result; no IRQ.
    #[test]
    fn perpendicular_accepts_one_param_returns_to_command() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.assert_irq6();
        assert!(f.irq_line());

        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_PERPENDICULAR));
        assert_eq!(f.phase, Phase::PerpendicularParam);
        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM,
            "param phase: RQM, !DIO"
        );
        assert!(f.irq_line(), "PERPENDICULAR must not clear IRQ latch");

        // OW=1, D0+D1 set, GAP=1, WGATE=1 → 0x80 | (0b0011<<2) | 0x03 = 0x8F
        f.port_write(FDC_FIFO, 1, 0x8F);
        assert_eq!(f.perp_d3_d0, 0x03);
        assert_eq!(f.perp_gap_wgate, 0x03);
        assert_eq!(f.phase, Phase::Command);
        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM,
            "idle command phase after PERPENDICULAR"
        );
        assert!(f.irq_line(), "PERPENDICULAR must not assert/clear IRQ");
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0xFF, "no result phase");
    }

    /// Spec: 82077AA §5.3.1 — when OW=0, only GAP|WGATE are considered; D3–D0
    /// retain previously programmed values.
    #[test]
    fn perpendicular_ow_zero_preserves_drive_bits() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));

        // Seed Dn with OW=1.
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_PERPENDICULAR));
        f.port_write(FDC_FIFO, 1, 0x80 | (0x0A << 2)); // OW=1, D3+D1, GAP=WGATE=0
        assert_eq!(f.perp_d3_d0, 0x0A);
        assert_eq!(f.perp_gap_wgate, 0);

        // OW=0: update GAP|WGATE only; Dn unchanged even if param bits 5:2 differ.
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_PERPENDICULAR));
        f.port_write(FDC_FIFO, 1, (0x0F << 2) | 0x03); // OW=0, would-be Dn=0xF, GAP|WGATE=11
        assert_eq!(f.perp_d3_d0, 0x0A, "Dn must not change when OW=0");
        assert_eq!(f.perp_gap_wgate, 0x03);
        assert_eq!(f.phase, Phase::Command);
    }

    /// Spec: PERPENDICULAR has no execution/result phase and must not assert IRQ.
    #[test]
    fn perpendicular_does_not_touch_irq() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_PERPENDICULAR));
        f.port_write(FDC_FIFO, 1, 0x83); // OW=1, D0, GAP|WGATE=11
        assert!(!f.irq_line(), "PERPENDICULAR never asserts IRQ");
    }

    #[test]
    fn perpendicular_ignored_while_held_in_dor_reset() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_PERPENDICULAR));
        f.port_write(FDC_FIFO, 1, 0x8F);
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N));
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        assert_eq!(f.phase, Phase::Command);
        assert_eq!(f.perp_d3_d0, 0);
        assert_eq!(f.perp_gap_wgate, 0);
    }

    #[test]
    fn dor_reset_aborts_perpendicular_param_phase() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_PERPENDICULAR));
        assert_eq!(f.phase, Phase::PerpendicularParam);

        f.port_write(FDC_DOR, 1, 0); // enter reset
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert_eq!(f.phase, Phase::Command);
        assert_eq!(f.perp_d3_d0, 0);
        assert_eq!(f.perp_gap_wgate, 0);
        // Mid-command abort: a lone param write must not be treated as a command.
        f.port_write(FDC_FIFO, 1, 0x8F);
        assert_eq!(f.perp_d3_d0, 0);
        assert_eq!(f.perp_gap_wgate, 0);
    }

    /// Spec: 82077AA §5.3.1 — soft DOR reset clears GAP|WGATE only; D3–D0
    /// retain. LOCK does not gate this (LOCK protects Configure fields only).
    #[test]
    fn soft_reset_clears_gap_wgate_preserves_drive_bits() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_PERPENDICULAR));
        f.port_write(FDC_FIFO, 1, 0x80 | (0x05 << 2) | 0x03); // OW=1, D0+D2, GAP|WGATE=11
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_LOCK_SET));
        let _ = f.port_read(FDC_FIFO, 1);
        assert!(f.lock);

        f.port_write(FDC_DOR, 1, 0); // soft reset
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert!(f.lock, "LOCK survives soft reset");
        assert_eq!(f.perp_d3_d0, 0x05, "Dn survive soft reset");
        assert_eq!(f.perp_gap_wgate, 0, "GAP|WGATE cleared by soft reset");
    }

    /// Spec: 82077AA §5.3.1 — hardware reset clears GAP, WGATE, and D0–D3.
    #[test]
    fn hardware_reset_clears_all_perp_bits() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_PERPENDICULAR));
        f.port_write(FDC_FIFO, 1, 0x8F);
        assert_eq!(f.perp_d3_d0, 0x03);
        assert_eq!(f.perp_gap_wgate, 0x03);

        f.reset();
        assert_eq!(f.perp_d3_d0, 0);
        assert_eq!(f.perp_gap_wgate, 0);
        assert_eq!(f.phase, Phase::Command);
    }

    /// Spec: 82077AA §5.3.3 — DUMPREG eighth result byte =
    /// `LOCK|0|D3 D2 D1 D0|GAP|WGATE` from stored PERPENDICULAR state.
    #[test]
    fn dumpreg_reflects_perpendicular_bits() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_PERPENDICULAR));
        f.port_write(FDC_FIFO, 1, 0x80 | (0x09 << 2) | 0x02); // OW=1, D0+D3, GAP=1
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_LOCK_SET));
        let _ = f.port_read(FDC_FIFO, 1);

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_DUMPREG));
        let mut result = [0u8; FDC_DUMPREG_RESULT_LEN as usize];
        for byte in &mut result {
            *byte = f.port_read(FDC_FIFO, 1) as u8;
        }
        // LOCK<<7 | D3–D0<<2 | GAP|WGATE = 0x80 | (0x09<<2) | 0x02 = 0xA6
        assert_eq!(result[7], 0xA6, "LOCK|0|D3–D0|GAP|WGATE");
        assert_eq!(f.phase, Phase::Command);
    }

    /// Spec: Intel 82077AA — each drive has its own Present Cylinder Number;
    /// Recalibrate/Seek update only the selected unit (US bits).
    #[test]
    fn per_drive_pcn_seek_and_recalibrate_update_selected_unit_only() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));

        // Seek unit 0 → NCN 0x10.
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SEEK));
        f.port_write(FDC_FIFO, 1, 0x00); // HD=0 | US=0
        f.port_write(FDC_FIFO, 1, 0x10);
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_INT));
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, FDC_ST0_SEEK_END);
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x10);

        // Seek unit 1 → NCN 0x20.
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SEEK));
        f.port_write(FDC_FIFO, 1, 0x01); // US=1
        f.port_write(FDC_FIFO, 1, 0x20);
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_INT));
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, FDC_ST0_SEEK_END | 0x01);
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x20);

        assert_eq!(f.pcn[0], 0x10);
        assert_eq!(f.pcn[1], 0x20);
        assert_eq!(f.pcn[2], 0x00);
        assert_eq!(f.pcn[3], 0x00);

        // Recalibrate unit 0 only.
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_RECALIBRATE));
        f.port_write(FDC_FIFO, 1, 0x00);
        assert_eq!(f.pcn[0], 0);
        assert_eq!(
            f.pcn[1], 0x20,
            "unit 1 PCN preserved across unit-0 recalibrate"
        );
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_INT));
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, FDC_ST0_SEEK_END);
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0);
    }

    /// Spec: Intel 82077AA Sense Interrupt — result PCN is the Present Cylinder
    /// Number for the unit encoded in ST0 bits 1:0.
    #[test]
    fn sense_interrupt_reports_pcn_of_st0_unit() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.pcn = [0x11, 0x22, 0x33, 0x44];

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SEEK));
        f.port_write(FDC_FIFO, 1, 0x03); // US=3
        f.port_write(FDC_FIFO, 1, 0x55); // NCN overwrites pcn[3]
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_INT));
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, FDC_ST0_SEEK_END | 0x03);
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x55);
        assert_eq!(f.pcn, [0x11, 0x22, 0x33, 0x55]);

        // Post-reset / assert_irq6 Sense Interrupt uses DOR US for PCN index.
        f.port_write(
            FDC_DOR,
            1,
            u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ | 0x02),
        );
        f.assert_irq6();
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_INT));
        assert_eq!(
            f.port_read(FDC_FIFO, 1) as u8,
            FDC_ST0_IC_READY_CHANGE | 0x02
        );
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x33, "PCN of DOR unit 2");
    }

    /// Spec: Intel 82077AA §5.2.5/§6.4 — Sense Drive Status T0 reflects the
    /// selected unit's TRK0 / PCN, not a shared cylinder.
    #[test]
    fn sense_drive_status_t0_uses_selected_unit_pcn() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.pcn[0] = 0x05; // off track 0
        f.pcn[1] = 0x00; // at track 0

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_DRIVE_STATUS));
        f.port_write(FDC_FIFO, 1, 0x00); // unit 0
        let st3_u0 = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(st3_u0 & FDC_ST3_TRACK0, 0, "unit 0 off track 0");

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_DRIVE_STATUS));
        f.port_write(FDC_FIFO, 1, 0x01); // unit 1
        let st3_u1 = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(st3_u1 & FDC_ST3_TRACK0, FDC_ST3_TRACK0, "unit 1 at track 0");
    }

    /// Spec: Intel 82077AA Table 5-1 / §5.3.3 — DUMPREG result bytes 0–3 are
    /// distinct PCN0–PCN3.
    #[test]
    fn dumpreg_reports_distinct_per_drive_pcn() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.pcn = [0x01, 0x02, 0x03, 0x04];

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_DUMPREG));
        let mut result = [0u8; FDC_DUMPREG_RESULT_LEN as usize];
        for byte in &mut result {
            *byte = f.port_read(FDC_FIFO, 1) as u8;
        }
        assert_eq!(&result[0..4], &[0x01, 0x02, 0x03, 0x04], "PCN0–PCN3");
        assert_eq!(f.phase, Phase::Command);
    }

    /// Spec: Intel 82077AA §5.1.1 / Table 5-1 — READ DATA opcode `0x06` with
    /// MFM (`0x46`); eight parameter bytes; no media → immediate result phase
    /// ST0 IC=01 (abnormal) | H | US, ST1 ND, ST2=0, C/H/R/N = ENDaddress from
    /// command params; IRQ6 when DOR DMA/IRQ enable (not Sense Interrupt).
    #[test]
    fn read_data_mfm_no_media_abnormal_result_and_irq() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert!(!f.irq_line());

        // SeaBIOS-style MFM READ DATA: MT=0 MFM=1 SK=0 | 0x06.
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_MFM | FDC_CMD_READ_DATA));
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        assert_eq!(f.phase, Phase::ReadDataParams { index: 0 });

        // Params: HD|US, C, H, R, N, EOT, GPL, DTL.
        let params = [0x04u8, 0x12, 0x01, 0x01, 0x02, 0x12, 0x1B, 0xFF]; // head1/unit0
        for (i, &p) in params.iter().enumerate() {
            f.port_write(FDC_FIFO, 1, u32::from(p));
            if i + 1 < params.len() {
                assert_eq!(
                    f.phase,
                    Phase::ReadDataParams {
                        index: (i + 1) as u8
                    }
                );
                assert!(!f.irq_line(), "IRQ only after all 8 params");
            }
        }

        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM | FDC_MSR_DIO,
            "result phase: RQM|DIO"
        );
        assert!(
            f.irq_line(),
            "READ DATA asserts IRQ6 on no-media completion"
        );

        let st0 = f.port_read(FDC_FIFO, 1) as u8;
        assert!(!f.irq_line(), "first result byte clears IRQ");
        assert_eq!(
            st0,
            FDC_ST0_IC_ABNORMAL | FDC_ST0_HEAD,
            "ST0 = IC=01 | H | US"
        );
        let st1 = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(st1, FDC_ST1_ND, "ST1 ND — sector/media not found");
        let st2 = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(st2, 0);
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x12); // C
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x01); // H
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x01); // R
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x02); // N
        assert_eq!(f.phase, Phase::Command);
        assert_eq!(f.sc_eot, 0x12, "EOT latched for DUMPREG");
    }

    /// Spec: Intel 82077AA Table 5-1 — MT|MFM|SK|READ DATA (`0xE6`) uses the
    /// same 8-param / 7-result shape as plain READ DATA.
    #[test]
    fn read_data_mt_mfm_sk_opcode_form_accepted() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));

        assert_eq!(FDC_CMD_READ_DATA_MT_MFM_SK, 0xE6);
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_READ_DATA_MT_MFM_SK));
        for p in [0x01u8, 0x00, 0x00, 0x01, 0x02, 0x12, 0x1B, 0xFF] {
            f.port_write(FDC_FIFO, 1, u32::from(p));
        }
        assert!(f.irq_line());
        let st0 = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(st0 & FDC_ST0_IC_ABNORMAL, FDC_ST0_IC_ABNORMAL);
        assert_eq!(st0 & 0x03, 0x01, "US from param0");
        // Drain remaining result bytes.
        for _ in 0..6 {
            let _ = f.port_read(FDC_FIFO, 1);
        }
        assert_eq!(f.phase, Phase::Command);
    }

    /// Spec: Intel 82077AA DOR bit3 — IRQ line gated; pending still latched.
    #[test]
    fn read_data_irq_pending_gated_by_dor_dma_irq() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N)); // DMA/IRQ disabled

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_MFM | FDC_CMD_READ_DATA));
        for p in [0x00u8, 0x00, 0x00, 0x01, 0x02, 0x12, 0x1B, 0xFF] {
            f.port_write(FDC_FIFO, 1, u32::from(p));
        }
        assert!(!f.irq_line(), "DOR DMA/IRQ clear → line inactive");
        // Result still available (MSR DIO).
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM | FDC_MSR_DIO);
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert!(f.irq_line(), "enabling DMA/IRQ reveals latched pending");
    }

    /// Spec: Intel 82077AA — held in DOR reset ignores command stream.
    #[test]
    fn read_data_ignored_while_held_in_dor_reset() {
        let mut f = Fdc82077::new();
        assert_eq!(f.dor & FDC_DOR_RESET_N, 0);
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_MFM | FDC_CMD_READ_DATA));
        assert_eq!(f.phase, Phase::Command);
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, 0);
    }

    /// Spec: Intel 82077AA — DOR soft reset aborts in-progress READ DATA params.
    #[test]
    fn dor_reset_aborts_read_data_param_phase() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_MFM | FDC_CMD_READ_DATA));
        f.port_write(FDC_FIFO, 1, 0x00);
        assert_eq!(f.phase, Phase::ReadDataParams { index: 1 });

        f.port_write(FDC_DOR, 1, 0); // enter reset
        assert_eq!(f.phase, Phase::Command);
        assert!(!f.irq_line());

        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
    }

    /// Spec: Intel 82077AA §5.1.2 / Table 5-1 — WRITE DATA opcode `0x05` with
    /// MFM (`0x45`); eight parameter bytes; no media → immediate result phase
    /// ST0 IC=01 (abnormal) | H | US, ST1 NW (Not Writable), ST2=0, C/H/R/N =
    /// ENDaddress from command params; IRQ6 when DOR DMA/IRQ enable.
    #[test]
    fn write_data_mfm_no_media_abnormal_result_and_irq() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert!(!f.irq_line());

        // SeaBIOS-style MFM WRITE DATA: MT=0 MFM=1 | 0x05.
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_MFM | FDC_CMD_WRITE_DATA));
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        assert_eq!(f.phase, Phase::WriteDataParams { index: 0 });

        // Params: HD|US, C, H, R, N, EOT, GPL, DTL.
        let params = [0x04u8, 0x12, 0x01, 0x01, 0x02, 0x12, 0x1B, 0xFF]; // head1/unit0
        for (i, &p) in params.iter().enumerate() {
            f.port_write(FDC_FIFO, 1, u32::from(p));
            if i + 1 < params.len() {
                assert_eq!(
                    f.phase,
                    Phase::WriteDataParams {
                        index: (i + 1) as u8
                    }
                );
                assert!(!f.irq_line(), "IRQ only after all 8 params");
            }
        }

        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM | FDC_MSR_DIO,
            "result phase: RQM|DIO"
        );
        assert!(
            f.irq_line(),
            "WRITE DATA asserts IRQ6 on no-media completion"
        );

        let st0 = f.port_read(FDC_FIFO, 1) as u8;
        assert!(!f.irq_line(), "first result byte clears IRQ");
        assert_eq!(
            st0,
            FDC_ST0_IC_ABNORMAL | FDC_ST0_HEAD,
            "ST0 = IC=01 | H | US"
        );
        let st1 = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(st1, FDC_ST1_NW, "ST1 NW — write not possible without media");
        let st2 = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(st2, 0);
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x12); // C
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x01); // H
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x01); // R
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x02); // N
        assert_eq!(f.phase, Phase::Command);
        assert_eq!(f.sc_eot, 0x12, "EOT latched for DUMPREG");
    }

    /// Spec: Intel 82077AA Table 5-1 — MT|MFM|WRITE DATA (`0xC5`) uses the
    /// same 8-param / 7-result shape as plain WRITE DATA.
    #[test]
    fn write_data_mt_mfm_opcode_form_accepted() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));

        assert_eq!(FDC_CMD_WRITE_DATA_MT_MFM, 0xC5);
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_WRITE_DATA_MT_MFM));
        for p in [0x01u8, 0x00, 0x00, 0x01, 0x02, 0x12, 0x1B, 0xFF] {
            f.port_write(FDC_FIFO, 1, u32::from(p));
        }
        assert!(f.irq_line());
        let st0 = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(st0 & FDC_ST0_IC_ABNORMAL, FDC_ST0_IC_ABNORMAL);
        assert_eq!(st0 & 0x03, 0x01, "US from param0");
        // Drain remaining result bytes.
        for _ in 0..6 {
            let _ = f.port_read(FDC_FIFO, 1);
        }
        assert_eq!(f.phase, Phase::Command);
    }

    /// Spec: Intel 82077AA DOR bit3 — IRQ line gated; pending still latched.
    #[test]
    fn write_data_irq_pending_gated_by_dor_dma_irq() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N)); // DMA/IRQ disabled

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_MFM | FDC_CMD_WRITE_DATA));
        for p in [0x00u8, 0x00, 0x00, 0x01, 0x02, 0x12, 0x1B, 0xFF] {
            f.port_write(FDC_FIFO, 1, u32::from(p));
        }
        assert!(!f.irq_line(), "DOR DMA/IRQ clear → line inactive");
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM | FDC_MSR_DIO);
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert!(f.irq_line(), "enabling DMA/IRQ reveals latched pending");
    }

    /// Spec: Intel 82077AA — held in DOR reset ignores command stream.
    #[test]
    fn write_data_ignored_while_held_in_dor_reset() {
        let mut f = Fdc82077::new();
        assert_eq!(f.dor & FDC_DOR_RESET_N, 0);
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_MFM | FDC_CMD_WRITE_DATA));
        assert_eq!(f.phase, Phase::Command);
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, 0);
    }

    /// Spec: Intel 82077AA — DOR soft reset aborts in-progress WRITE DATA params.
    #[test]
    fn dor_reset_aborts_write_data_param_phase() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_MFM | FDC_CMD_WRITE_DATA));
        f.port_write(FDC_FIFO, 1, 0x00);
        assert_eq!(f.phase, Phase::WriteDataParams { index: 1 });

        f.port_write(FDC_DOR, 1, 0); // enter reset
        assert_eq!(f.phase, Phase::Command);
        assert!(!f.irq_line());

        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
    }

    /// Spec: Intel 82077AA §5.1.4 / Table 5-1 — WRITE DELETED DATA opcode `0x09`
    /// with MFM (`0x49`); eight parameter bytes; no media → immediate result
    /// ST0 IC=01|H|US, ST1 NW, ST2=0, C/H/R/N; IRQ6 when DOR DMA/IRQ enable.
    #[test]
    fn write_deleted_data_mfm_no_media_abnormal_result_and_irq() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert!(!f.irq_line());

        f.port_write(
            FDC_FIFO,
            1,
            u32::from(FDC_CMD_MFM | FDC_CMD_WRITE_DELETED_DATA),
        );
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        assert_eq!(f.phase, Phase::WriteDeletedDataParams { index: 0 });

        let params = [0x04u8, 0x12, 0x01, 0x01, 0x02, 0x12, 0x1B, 0xFF];
        for (i, &p) in params.iter().enumerate() {
            f.port_write(FDC_FIFO, 1, u32::from(p));
            if i + 1 < params.len() {
                assert_eq!(
                    f.phase,
                    Phase::WriteDeletedDataParams {
                        index: (i + 1) as u8
                    }
                );
                assert!(!f.irq_line(), "IRQ only after all 8 params");
            }
        }

        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM | FDC_MSR_DIO,
            "result phase: RQM|DIO"
        );
        assert!(
            f.irq_line(),
            "WRITE DELETED DATA asserts IRQ6 on no-media completion"
        );

        let st0 = f.port_read(FDC_FIFO, 1) as u8;
        assert!(!f.irq_line(), "first result byte clears IRQ");
        assert_eq!(
            st0,
            FDC_ST0_IC_ABNORMAL | FDC_ST0_HEAD,
            "ST0 = IC=01 | H | US"
        );
        let st1 = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(st1, FDC_ST1_NW, "ST1 NW — write not possible without media");
        let st2 = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(st2, 0);
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x12); // C
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x01); // H
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x01); // R
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x02); // N
        assert_eq!(f.phase, Phase::Command);
        assert_eq!(f.sc_eot, 0x12, "EOT latched for DUMPREG");
    }

    /// Spec: Intel 82077AA Table 5-1 — MT|MFM|WRITE DELETED DATA (`0xC9`).
    #[test]
    fn write_deleted_data_mt_mfm_opcode_form_accepted() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));

        assert_eq!(FDC_CMD_WRITE_DELETED_DATA_MT_MFM, 0xC9);
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_WRITE_DELETED_DATA_MT_MFM));
        for p in [0x01u8, 0x00, 0x00, 0x01, 0x02, 0x12, 0x1B, 0xFF] {
            f.port_write(FDC_FIFO, 1, u32::from(p));
        }
        assert!(f.irq_line());
        let st0 = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(st0 & FDC_ST0_IC_ABNORMAL, FDC_ST0_IC_ABNORMAL);
        assert_eq!(st0 & 0x03, 0x01, "US from param0");
        for _ in 0..6 {
            let _ = f.port_read(FDC_FIFO, 1);
        }
        assert_eq!(f.phase, Phase::Command);
    }

    /// Spec: Intel 82077AA DOR bit3 — IRQ line gated; pending still latched.
    #[test]
    fn write_deleted_data_irq_pending_gated_by_dor_dma_irq() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N));

        f.port_write(
            FDC_FIFO,
            1,
            u32::from(FDC_CMD_MFM | FDC_CMD_WRITE_DELETED_DATA),
        );
        for p in [0x00u8, 0x00, 0x00, 0x01, 0x02, 0x12, 0x1B, 0xFF] {
            f.port_write(FDC_FIFO, 1, u32::from(p));
        }
        assert!(!f.irq_line(), "DOR DMA/IRQ clear → line inactive");
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM | FDC_MSR_DIO);
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert!(f.irq_line(), "enabling DMA/IRQ reveals latched pending");
    }

    /// Spec: Intel 82077AA — held in DOR reset ignores command stream.
    #[test]
    fn write_deleted_data_ignored_while_held_in_dor_reset() {
        let mut f = Fdc82077::new();
        assert_eq!(f.dor & FDC_DOR_RESET_N, 0);
        f.port_write(
            FDC_FIFO,
            1,
            u32::from(FDC_CMD_MFM | FDC_CMD_WRITE_DELETED_DATA),
        );
        assert_eq!(f.phase, Phase::Command);
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, 0);
    }

    /// Spec: Intel 82077AA — DOR soft reset aborts in-progress WRITE DELETED params.
    #[test]
    fn dor_reset_aborts_write_deleted_data_param_phase() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.port_write(
            FDC_FIFO,
            1,
            u32::from(FDC_CMD_MFM | FDC_CMD_WRITE_DELETED_DATA),
        );
        f.port_write(FDC_FIFO, 1, 0x00);
        assert_eq!(f.phase, Phase::WriteDeletedDataParams { index: 1 });

        f.port_write(FDC_DOR, 1, 0);
        assert_eq!(f.phase, Phase::Command);
        assert!(!f.irq_line());

        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
    }

    /// Spec: Intel 82077AA Table 5-1 — SCAN EQUAL (`0x11`) no-media via VERIFY path.
    #[test]
    fn scan_equal_mfm_no_media_via_verify_path() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_MFM | FDC_CMD_SCAN_EQUAL));
        assert_eq!(f.phase, Phase::VerifyParams { index: 0 });
        for p in [0x00u8, 0x00, 0x00, 0x01, 0x02, 0x12, 0x1B, 0xFF] {
            f.port_write(FDC_FIFO, 1, u32::from(p));
        }
        assert!(f.irq_line());
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, FDC_ST0_IC_ABNORMAL);
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, FDC_ST1_ND);
        for _ in 0..5 {
            let _ = f.port_read(FDC_FIFO, 1);
        }
        assert_eq!(f.phase, Phase::Command);
    }

    /// Spec: Intel 82077AA — SCAN LOW/HIGH OR EQUAL (`0x19`/`0x1D`) via VERIFY path.
    #[test]
    fn scan_low_high_or_equal_no_media_via_verify_path() {
        for opcode in [FDC_CMD_SCAN_LOW_OR_EQUAL, FDC_CMD_SCAN_HIGH_OR_EQUAL] {
            let mut f = Fdc82077::new();
            f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
            f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_MFM | opcode));
            for p in [0x04u8, 0x00, 0x01, 0x01, 0x02, 0x12, 0x1B, 0xFF] {
                f.port_write(FDC_FIFO, 1, u32::from(p));
            }
            assert!(f.irq_line());
            assert_eq!(
                f.port_read(FDC_FIFO, 1) as u8,
                FDC_ST0_IC_ABNORMAL | FDC_ST0_HEAD
            );
            assert_eq!(f.port_read(FDC_FIFO, 1) as u8, FDC_ST1_ND);
            for _ in 0..5 {
                let _ = f.port_read(FDC_FIFO, 1);
            }
        }
    }

    /// Spec: Intel 82077AA Table 5-1 — READ ID (`0x0A` / MFM `0x4A`) no-media:
    /// 1 param HD|US → ST0 IC=01|H|US, ST1 ND, C/H/R/N=0 + IRQ6.
    #[test]
    fn read_id_mfm_no_media_abnormal_result_and_irq() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert_eq!(FDC_CMD_READ_ID_MFM, 0x4A);
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_READ_ID_MFM));
        assert_eq!(f.phase, Phase::ReadIdParam);
        assert!(!f.irq_line());
        f.port_write(FDC_FIFO, 1, 0x04); // head1 / unit0
        assert!(f.irq_line());
        let st0 = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(st0, FDC_ST0_IC_ABNORMAL | FDC_ST0_HEAD);
        assert!(!f.irq_line(), "first result byte clears IRQ6");
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, FDC_ST1_ND);
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0); // ST2
        for _ in 0..4 {
            assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0); // C/H/R/N
        }
        assert_eq!(f.phase, Phase::Command);
    }

    /// Spec: Intel 82077AA Table 5-1 / §5.1.8 — READ ID with media: normal ST0
    /// (IC=00 | H | US), ST1=ST2=0, sector-ID stub C=`pcn[unit]`, H from HD|US
    /// param, R=1, N=2; IRQ6. No full IDAM scan this slice.
    #[test]
    fn read_id_with_media_normal_sector_id_stub() {
        let mut f = Fdc82077::with_image(vec![0u8; FDC_1440_IMAGE_SIZE]);
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));

        // Seek unit0 to cylinder 12 so READ ID C reflects pcn[0].
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SEEK));
        f.port_write(FDC_FIFO, 1, 0x00); // HD|US unit0
        f.port_write(FDC_FIFO, 1, 12); // NCN
        assert!(f.irq_line());
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_INT));
        let _ = f.port_read(FDC_FIFO, 1);
        let _ = f.port_read(FDC_FIFO, 1);
        assert_eq!(f.pcn[0], 12);

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_READ_ID_MFM));
        assert_eq!(f.phase, Phase::ReadIdParam);
        f.port_write(FDC_FIFO, 1, 0x04); // head1 / unit0
        assert!(
            f.irq_line(),
            "READ ID asserts IRQ6 on media success completion"
        );
        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM | FDC_MSR_DIO,
            "result phase: RQM|DIO"
        );

        let st0 = f.port_read(FDC_FIFO, 1) as u8;
        assert!(!f.irq_line(), "first result byte clears IRQ6");
        assert_eq!(
            st0,
            FDC_ST0_IC_NORMAL | FDC_ST0_HEAD,
            "ST0 = IC=00 | H=1 | US=0"
        );
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x00, "ST1 clear");
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x00, "ST2 clear");
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 12, "C = pcn[unit]");
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x01, "H from HD param bit");
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x01, "R=1 sector-ID stub");
        assert_eq!(
            f.port_read(FDC_FIFO, 1) as u8,
            FDC_SECTOR_N,
            "N=2 (512-byte)"
        );
        assert_eq!(f.phase, Phase::Command);
    }

    /// Spec: Intel 82077AA — READ ID media path still uses ND abnormal when
    /// media is ejected (no-media stub preserved).
    #[test]
    fn read_id_after_eject_still_nd_abnormal() {
        let mut f = Fdc82077::with_image(vec![0u8; FDC_1440_IMAGE_SIZE]);
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.eject();
        assert!(!f.has_media());

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_READ_ID_MFM));
        f.port_write(FDC_FIFO, 1, 0x00);
        assert!(f.irq_line());
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, FDC_ST0_IC_ABNORMAL);
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, FDC_ST1_ND);
        for _ in 0..5 {
            assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0);
        }
    }

    /// Spec: Intel 82077AA Table 5-1 — VERIFY (`0x16`) no-media stub.
    #[test]
    fn verify_mfm_no_media_abnormal_result_and_irq() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_MFM | FDC_CMD_VERIFY));
        assert_eq!(f.phase, Phase::VerifyParams { index: 0 });
        for p in [0x04u8, 0x12, 0x01, 0x01, 0x02, 0x12, 0x1B, 0xFF] {
            f.port_write(FDC_FIFO, 1, u32::from(p));
        }
        assert!(f.irq_line());
        let st0 = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(st0, FDC_ST0_IC_ABNORMAL | FDC_ST0_HEAD);
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, FDC_ST1_ND);
        for _ in 0..5 {
            let _ = f.port_read(FDC_FIFO, 1);
        }
        assert_eq!(f.phase, Phase::Command);
        assert_eq!(f.sc_eot, 0x12);
    }

    #[test]
    fn verify_mt_mfm_sk_opcode_form_accepted() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert_eq!(FDC_CMD_VERIFY_MT_MFM_SK, 0xF6);
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_VERIFY_MT_MFM_SK));
        for p in [0x01u8, 0x00, 0x00, 0x01, 0x02, 0x12, 0x1B, 0xFF] {
            f.port_write(FDC_FIFO, 1, u32::from(p));
        }
        assert!(f.irq_line());
        let _ = f.port_read(FDC_FIFO, 1);
        for _ in 0..6 {
            let _ = f.port_read(FDC_FIFO, 1);
        }
    }

    #[test]
    fn dor_reset_aborts_verify_param_phase() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_MFM | FDC_CMD_VERIFY));
        f.port_write(FDC_FIFO, 1, 0x00);
        assert_eq!(f.phase, Phase::VerifyParams { index: 1 });
        f.port_write(FDC_DOR, 1, 0);
        assert_eq!(f.phase, Phase::Command);
    }

    /// Spec: Intel 82077AA §5.1.7 / Table 5-1 — FORMAT TRACK opcode `0x0D` with
    /// MFM (`0x4D`); five parameter bytes (HD|US, N, SC, GPL, D); no media →
    /// skip execution/DMA/per-sector ID stream; immediate result ST0 IC=01
    /// (abnormal) | H | US, ST1 NW (Not Writable — §6.2 applies to FORMAT),
    /// ST2=0, four undefined bytes = 0; IRQ6 when DOR DMA/IRQ enable; SC→`sc_eot`.
    #[test]
    fn format_track_mfm_no_media_abnormal_result_and_irq() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert!(!f.irq_line());

        // SeaBIOS-style MFM FORMAT TRACK: MT=0 MFM=1 | 0x0D → 0x4D.
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_MFM | FDC_CMD_FORMAT_TRACK));
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        assert_eq!(f.phase, Phase::FormatTrackParams { index: 0 });

        // Params: HD|US, N, SC, GPL, D. Spec: 82077AA Table 5-1.
        let params = [0x04u8, 0x02, 0x12, 0x54, 0xF6]; // head1/unit0, N=2, SC=18
        for (i, &p) in params.iter().enumerate() {
            f.port_write(FDC_FIFO, 1, u32::from(p));
            if i + 1 < params.len() {
                assert_eq!(
                    f.phase,
                    Phase::FormatTrackParams {
                        index: (i + 1) as u8
                    }
                );
                assert!(!f.irq_line(), "IRQ only after all 5 params");
            }
        }

        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM | FDC_MSR_DIO,
            "result phase: RQM|DIO"
        );
        assert!(
            f.irq_line(),
            "FORMAT TRACK asserts IRQ6 on no-media completion"
        );

        let st0 = f.port_read(FDC_FIFO, 1) as u8;
        assert!(!f.irq_line(), "first result byte clears IRQ");
        assert_eq!(
            st0,
            FDC_ST0_IC_ABNORMAL | FDC_ST0_HEAD,
            "ST0 = IC=01 | H | US"
        );
        let st1 = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(
            st1, FDC_ST1_NW,
            "ST1 NW — format not possible without media"
        );
        let st2 = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(st2, 0);
        // Spec: 82077AA Table 5-1 — remaining four result bytes undefined.
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x00);
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x00);
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x00);
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x00);
        assert_eq!(f.phase, Phase::Command);
        assert_eq!(f.sc_eot, 0x12, "SC latched for DUMPREG");
    }

    /// Spec: Intel 82077AA Table 5-1 — plain FORMAT TRACK (`0x0D`, FM) uses the
    /// same 5-param / 7-result shape as MFM `0x4D`.
    #[test]
    fn format_track_fm_opcode_form_accepted() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));

        assert_eq!(FDC_CMD_FORMAT_TRACK, 0x0D);
        assert_eq!(FDC_CMD_FORMAT_TRACK_MFM, 0x4D);
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_FORMAT_TRACK));
        for p in [0x01u8, 0x02, 0x09, 0x2A, 0xF6] {
            f.port_write(FDC_FIFO, 1, u32::from(p));
        }
        assert!(f.irq_line());
        let st0 = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(st0 & FDC_ST0_IC_ABNORMAL, FDC_ST0_IC_ABNORMAL);
        assert_eq!(st0 & 0x03, 0x01, "US from param0");
        for _ in 0..6 {
            let _ = f.port_read(FDC_FIFO, 1);
        }
        assert_eq!(f.phase, Phase::Command);
        assert_eq!(f.sc_eot, 0x09);
    }

    /// Spec: Intel 82077AA DOR bit3 — IRQ line gated; pending still latched.
    #[test]
    fn format_track_irq_pending_gated_by_dor_dma_irq() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N)); // DMA/IRQ disabled

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_MFM | FDC_CMD_FORMAT_TRACK));
        for p in [0x00u8, 0x02, 0x12, 0x54, 0xF6] {
            f.port_write(FDC_FIFO, 1, u32::from(p));
        }
        assert!(!f.irq_line(), "DOR DMA/IRQ clear → line inactive");
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM | FDC_MSR_DIO);
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert!(f.irq_line(), "enabling DMA/IRQ reveals latched pending");
    }

    /// Spec: Intel 82077AA — held in DOR reset ignores command stream.
    #[test]
    fn format_track_ignored_while_held_in_dor_reset() {
        let mut f = Fdc82077::new();
        assert_eq!(f.dor & FDC_DOR_RESET_N, 0);
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_MFM | FDC_CMD_FORMAT_TRACK));
        assert_eq!(f.phase, Phase::Command);
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, 0);
    }

    /// Spec: Intel 82077AA — DOR soft reset aborts in-progress FORMAT TRACK params.
    #[test]
    fn dor_reset_aborts_format_track_param_phase() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_MFM | FDC_CMD_FORMAT_TRACK));
        f.port_write(FDC_FIFO, 1, 0x00);
        assert_eq!(f.phase, Phase::FormatTrackParams { index: 1 });

        f.port_write(FDC_DOR, 1, 0); // enter reset
        assert_eq!(f.phase, Phase::Command);
        assert!(!f.irq_line());

        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
    }

    /// Spec: Intel 82077AA §5.1.3 / Table 5-1 — READ DELETED DATA opcode `0x0C`
    /// with MFM (`0x4C`); eight parameter bytes (same as READ DATA); no media →
    /// immediate result ST0 IC=01 (abnormal) | H | US, ST1 ND, ST2=0, C/H/R/N =
    /// ENDaddress; IRQ6 when DOR DMA/IRQ enable (not Sense Interrupt).
    #[test]
    fn read_deleted_data_mfm_no_media_abnormal_result_and_irq() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert!(!f.irq_line());

        // SeaBIOS-style MFM READ DELETED DATA: MT=0 MFM=1 SK=0 | 0x0C.
        f.port_write(
            FDC_FIFO,
            1,
            u32::from(FDC_CMD_MFM | FDC_CMD_READ_DELETED_DATA),
        );
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        assert_eq!(f.phase, Phase::ReadDeletedDataParams { index: 0 });

        // Params: HD|US, C, H, R, N, EOT, GPL, DTL.
        let params = [0x04u8, 0x12, 0x01, 0x01, 0x02, 0x12, 0x1B, 0xFF]; // head1/unit0
        for (i, &p) in params.iter().enumerate() {
            f.port_write(FDC_FIFO, 1, u32::from(p));
            if i + 1 < params.len() {
                assert_eq!(
                    f.phase,
                    Phase::ReadDeletedDataParams {
                        index: (i + 1) as u8
                    }
                );
                assert!(!f.irq_line(), "IRQ only after all 8 params");
            }
        }

        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM | FDC_MSR_DIO,
            "result phase: RQM|DIO"
        );
        assert!(
            f.irq_line(),
            "READ DELETED DATA asserts IRQ6 on no-media completion"
        );

        let st0 = f.port_read(FDC_FIFO, 1) as u8;
        assert!(!f.irq_line(), "first result byte clears IRQ");
        assert_eq!(
            st0,
            FDC_ST0_IC_ABNORMAL | FDC_ST0_HEAD,
            "ST0 = IC=01 | H | US"
        );
        let st1 = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(
            st1, FDC_ST1_ND,
            "ST1 ND — sector/media not found (same as READ DATA stub)"
        );
        let st2 = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(st2, 0);
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x12); // C
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x01); // H
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x01); // R
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x02); // N
        assert_eq!(f.phase, Phase::Command);
        assert_eq!(f.sc_eot, 0x12, "EOT latched for DUMPREG");
    }

    /// Spec: Intel 82077AA Table 5-1 — MT|MFM|SK|READ DELETED DATA (`0xEC`) uses
    /// the same 8-param / 7-result shape as plain READ DELETED DATA / READ DATA.
    #[test]
    fn read_deleted_data_mt_mfm_sk_opcode_form_accepted() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));

        assert_eq!(FDC_CMD_READ_DELETED_DATA_MT_MFM_SK, 0xEC);
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_READ_DELETED_DATA_MT_MFM_SK));
        for p in [0x01u8, 0x00, 0x00, 0x01, 0x02, 0x12, 0x1B, 0xFF] {
            f.port_write(FDC_FIFO, 1, u32::from(p));
        }
        assert!(f.irq_line());
        let st0 = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(st0 & FDC_ST0_IC_ABNORMAL, FDC_ST0_IC_ABNORMAL);
        assert_eq!(st0 & 0x03, 0x01, "US from param0");
        // Drain remaining result bytes.
        for _ in 0..6 {
            let _ = f.port_read(FDC_FIFO, 1);
        }
        assert_eq!(f.phase, Phase::Command);
    }

    /// Spec: Intel 82077AA DOR bit3 — IRQ line gated; pending still latched.
    #[test]
    fn read_deleted_data_irq_pending_gated_by_dor_dma_irq() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N)); // DMA/IRQ disabled

        f.port_write(
            FDC_FIFO,
            1,
            u32::from(FDC_CMD_MFM | FDC_CMD_READ_DELETED_DATA),
        );
        for p in [0x00u8, 0x00, 0x00, 0x01, 0x02, 0x12, 0x1B, 0xFF] {
            f.port_write(FDC_FIFO, 1, u32::from(p));
        }
        assert!(!f.irq_line(), "DOR DMA/IRQ clear → line inactive");
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM | FDC_MSR_DIO);
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert!(f.irq_line(), "enabling DMA/IRQ reveals latched pending");
    }

    /// Spec: Intel 82077AA — held in DOR reset ignores command stream.
    #[test]
    fn read_deleted_data_ignored_while_held_in_dor_reset() {
        let mut f = Fdc82077::new();
        assert_eq!(f.dor & FDC_DOR_RESET_N, 0);
        f.port_write(
            FDC_FIFO,
            1,
            u32::from(FDC_CMD_MFM | FDC_CMD_READ_DELETED_DATA),
        );
        assert_eq!(f.phase, Phase::Command);
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, 0);
    }

    /// Spec: Intel 82077AA — DOR soft reset aborts in-progress READ DELETED DATA params.
    #[test]
    fn dor_reset_aborts_read_deleted_data_param_phase() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.port_write(
            FDC_FIFO,
            1,
            u32::from(FDC_CMD_MFM | FDC_CMD_READ_DELETED_DATA),
        );
        f.port_write(FDC_FIFO, 1, 0x00);
        assert_eq!(f.phase, Phase::ReadDeletedDataParams { index: 1 });

        f.port_write(FDC_DOR, 1, 0); // enter reset
        assert_eq!(f.phase, Phase::Command);
        assert!(!f.irq_line());

        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
    }

    /// Unrelated opcodes (e.g. READ DATA `0x06`) remain distinct from READ DELETED DATA.
    #[test]
    fn read_deleted_data_unrelated_opcodes_unchanged() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_MFM | FDC_CMD_READ_DATA));
        assert_eq!(f.phase, Phase::ReadDataParams { index: 0 });
        // Soft-reset abort and start READ DELETED DATA instead.
        f.port_write(FDC_DOR, 1, 0);
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.port_write(
            FDC_FIFO,
            1,
            u32::from(FDC_CMD_MFM | FDC_CMD_READ_DELETED_DATA),
        );
        assert_eq!(f.phase, Phase::ReadDeletedDataParams { index: 0 });
        // Opcode masks are distinct: 0x06 vs 0x0C under FDC_CMD_OPCODE_MASK.
        assert_ne!(
            FDC_CMD_READ_DATA & FDC_CMD_OPCODE_MASK,
            FDC_CMD_READ_DELETED_DATA & FDC_CMD_OPCODE_MASK
        );
    }

    /// Spec: IBM PC 1.44MB MFM geometry — exact image size attaches; wrong size rejected.
    #[test]
    fn attach_1440k_image_accepts_exact_size_rejects_wrong() {
        let mut f = Fdc82077::new();
        assert!(!f.has_media());
        assert_eq!(FDC_1440_IMAGE_SIZE, 1_474_560);
        assert_eq!(
            FDC_1440_CYLINDERS as usize
                * FDC_1440_HEADS as usize
                * FDC_1440_SECTORS_PER_TRACK as usize
                * FDC_SECTOR_SIZE,
            FDC_1440_IMAGE_SIZE
        );
        assert_eq!(FDC_SECTOR_N, 2, "N=2 → 128<<2 = 512-byte sectors");

        assert!(f.attach_image(vec![0u8; 512]).is_err());
        assert!(!f.has_media());

        assert!(f.attach_image(vec![0u8; FDC_1440_IMAGE_SIZE]).is_ok());
        assert!(f.has_media());
        assert_eq!(
            f.port_read(FDC_DIR_CCR, 1) as u8 & FDC_DIR_DSKCHG,
            0,
            "DSKCHG cleared while media attached"
        );

        let f2 = Fdc82077::with_image(vec![0xABu8; FDC_1440_IMAGE_SIZE]);
        assert!(f2.has_media());
    }

    /// Spec: IBM PC 1.44MB CHS layout — R is 1-based; linear MFM image order C→H→R.
    #[test]
    fn chs_byte_offset_1440k_geometry() {
        assert_eq!(Fdc82077::chs_byte_offset(0, 0, 1), Some(0));
        assert_eq!(Fdc82077::chs_byte_offset(0, 0, 2), Some(512));
        assert_eq!(
            Fdc82077::chs_byte_offset(0, 1, 1),
            Some(18 * FDC_SECTOR_SIZE)
        );
        assert_eq!(
            Fdc82077::chs_byte_offset(1, 0, 1),
            Some(2 * 18 * FDC_SECTOR_SIZE)
        );
        assert_eq!(Fdc82077::chs_byte_offset(0, 0, 0), None, "R is 1-based");
        assert_eq!(Fdc82077::chs_byte_offset(0, 0, 19), None);
        assert_eq!(Fdc82077::chs_byte_offset(0, 2, 1), None);
        assert_eq!(Fdc82077::chs_byte_offset(80, 0, 1), None);
    }

    /// Spec: OSDev/IBM 1.44MB — `read_sector` returns image bytes at CHS; OOR → None.
    #[test]
    fn read_sector_returns_image_bytes() {
        let mut img = vec![0u8; FDC_1440_IMAGE_SIZE];
        // Mark (0,0,1), (0,0,2), (0,1,1), (1,0,1).
        img[0] = 0x11;
        img[511] = 0x22;
        img[512] = 0x33;
        img[18 * FDC_SECTOR_SIZE] = 0x44;
        img[2 * 18 * FDC_SECTOR_SIZE] = 0x55;

        let f = Fdc82077::with_image(img);
        let s001 = f.read_sector(0, 0, 1).expect("sector 0,0,1");
        assert_eq!(s001[0], 0x11);
        assert_eq!(s001[511], 0x22);
        assert_eq!(f.read_sector(0, 0, 2).unwrap()[0], 0x33);
        assert_eq!(f.read_sector(0, 1, 1).unwrap()[0], 0x44);
        assert_eq!(f.read_sector(1, 0, 1).unwrap()[0], 0x55);
        assert!(f.read_sector(0, 0, 19).is_none());
        assert!(f.read_sector(80, 0, 1).is_none());

        let empty = Fdc82077::new();
        assert!(empty.read_sector(0, 0, 1).is_none(), "no media → None");
    }

    /// Stub DIR bit7 DSKCHG: set on eject / no-media after attach history; clear on attach.
    /// Spec note: Intel 82077AA DIR DSKCHG is a latched change line cleared by
    /// drive-select/step sequencing — this stub only tracks attach/eject.
    #[test]
    fn eject_sets_dskchg_reset_preserves_media() {
        let mut f = Fdc82077::with_image(vec![0x7Eu8; FDC_1440_IMAGE_SIZE]);
        assert!(f.has_media());
        assert_eq!(f.port_read(FDC_DIR_CCR, 1) as u8 & FDC_DIR_DSKCHG, 0);

        f.eject();
        assert!(!f.has_media());
        assert_eq!(
            f.port_read(FDC_DIR_CCR, 1) as u8 & FDC_DIR_DSKCHG,
            FDC_DIR_DSKCHG,
            "eject sets DSKCHG"
        );

        // Re-attach clears DSKCHG.
        assert!(f.attach_image(vec![0x7Eu8; FDC_1440_IMAGE_SIZE]).is_ok());
        assert_eq!(f.port_read(FDC_DIR_CCR, 1) as u8 & FDC_DIR_DSKCHG, 0);

        // reset() preserves backing image like IdePrimary.
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.specify_srt_hut = 0xDF;
        f.reset();
        assert!(f.has_media(), "reset preserves attached image");
        assert_eq!(f.read_sector(0, 0, 1).unwrap()[0], 0x7E);
        assert_eq!(f.dor, 0, "reset clears programmed DOR");
        assert_eq!(f.specify_srt_hut, 0);
        assert_eq!(
            f.port_read(FDC_DIR_CCR, 1) as u8 & FDC_DIR_DSKCHG,
            0,
            "media still attached → DSKCHG clear"
        );

        f.eject();
        f.reset();
        assert!(!f.has_media());
        assert_eq!(
            f.port_read(FDC_DIR_CCR, 1) as u8 & FDC_DIR_DSKCHG,
            FDC_DIR_DSKCHG,
            "reset preserves DSKCHG after eject history"
        );
    }

    /// READ DATA path remains the no-media ND stub when no image is attached.
    #[test]
    fn finish_read_data_still_nd_without_media() {
        let mut f = Fdc82077::new();
        assert!(!f.has_media());
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_MFM | FDC_CMD_READ_DATA));
        for p in [0x00u8, 0x00, 0x00, 0x01, 0x02, 0x12, 0x1B, 0xFF] {
            f.port_write(FDC_FIFO, 1, u32::from(p));
        }
        assert!(f.irq_line());
        let st0 = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(st0 & FDC_ST0_IC_ABNORMAL, FDC_ST0_IC_ABNORMAL);
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, FDC_ST1_ND);
        assert!(
            f.last_sector().is_none(),
            "no-media ND path must not latch sector bytes"
        );
        for _ in 0..5 {
            let _ = f.port_read(FDC_FIFO, 1);
        }
        assert_eq!(f.phase, Phase::Command);
    }

    /// Spec: Intel 82077AA §5.1.1 READ DATA result — with media + N=2 + valid
    /// CHS, normal termination (IC=00 | H | US), ST1=ST2=0, C/H/R/N ENDaddress,
    /// EOT→`sc_eot`, IRQ6; latches one 512-byte sector and arms
    /// `take_pending_dma_sector` for MachineBus DMA ch2. Multi-sector MT
    /// ignored — one sector only. Spec: IBM 1.44MB geometry.
    #[test]
    fn read_data_with_media_normal_result_and_last_sector() {
        let mut img = vec![0u8; FDC_1440_IMAGE_SIZE];
        for (i, b) in img[..FDC_SECTOR_SIZE].iter_mut().enumerate() {
            *b = (i & 0xFF) as u8;
        }
        let mut f = Fdc82077::with_image(img);
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));

        // READ DATA MFM: C=0,H=0,R=1,N=2 — first sector of the image.
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_MFM | FDC_CMD_READ_DATA));
        for p in [0x00u8, 0x00, 0x00, 0x01, 0x02, 0x12, 0x1B, 0xFF] {
            f.port_write(FDC_FIFO, 1, u32::from(p));
        }

        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM | FDC_MSR_DIO,
            "result phase: RQM|DIO"
        );
        assert!(
            f.irq_line(),
            "READ DATA asserts IRQ6 on media success completion"
        );

        let st0 = f.port_read(FDC_FIFO, 1) as u8;
        assert!(!f.irq_line(), "first result byte clears IRQ");
        assert_eq!(
            st0, FDC_ST0_IC_NORMAL,
            "ST0 = IC=00 | H=0 | US=0 (normal termination)"
        );
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x00, "ST1 clear");
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x00, "ST2 clear");
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x00); // C
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x00); // H
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x01); // R
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x02); // N
        assert_eq!(f.phase, Phase::Command);
        assert_eq!(f.sc_eot, 0x12, "EOT latched for DUMPREG");

        let sector = *f.last_sector().expect("media READ DATA latches sector");
        assert_eq!(sector.len(), FDC_SECTOR_SIZE);
        for (i, &b) in sector.iter().enumerate() {
            assert_eq!(b, (i & 0xFF) as u8, "last_sector[{i}]");
        }
        let pending = f
            .take_pending_dma_sector()
            .expect("DOR DMA/IRQ arms one-shot DMA pending");
        assert_eq!(pending, sector);
        assert!(
            f.take_pending_dma_sector().is_none(),
            "pending arm is one-shot"
        );
        assert!(
            f.last_sector().is_some(),
            "inspection latch survives take_pending_dma_sector"
        );
    }

    /// Spec: Intel 82077AA §5.1.1 / §6.2 — out-of-range sector ID (R>18 on
    /// 1.44MB) with media attached still completes as ND abnormal (no sector
    /// latch). IBM 1.44MB SPT=18.
    #[test]
    fn read_data_with_media_oor_r_nd_abnormal() {
        let mut f = Fdc82077::with_image(vec![0xAAu8; FDC_1440_IMAGE_SIZE]);
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_MFM | FDC_CMD_READ_DATA));
        // C=0,H=0,R=19 (OOR), N=2
        for p in [0x00u8, 0x00, 0x00, 0x13, 0x02, 0x12, 0x1B, 0xFF] {
            f.port_write(FDC_FIFO, 1, u32::from(p));
        }
        assert!(f.irq_line());
        let st0 = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(st0 & FDC_ST0_IC_ABNORMAL, FDC_ST0_IC_ABNORMAL);
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, FDC_ST1_ND);
        assert!(
            f.last_sector().is_none(),
            "OOR CHS must not latch sector bytes"
        );
        for _ in 0..5 {
            let _ = f.port_read(FDC_FIFO, 1);
        }
        assert_eq!(f.phase, Phase::Command);
    }

    /// Spec: IBM PC 1.44MB geometry — `write_sector` stores 512 bytes at CHS.
    #[test]
    fn write_sector_writes_image_at_chs() {
        let mut f = Fdc82077::with_image(vec![0u8; FDC_1440_IMAGE_SIZE]);
        let mut data = [0u8; FDC_SECTOR_SIZE];
        for (i, b) in data.iter_mut().enumerate() {
            *b = (0xC0 + (i & 0x3F)) as u8;
        }
        assert!(
            f.write_sector(0, 0, 1, &data),
            "valid CHS with media must accept write"
        );
        let read_back = f.read_sector(0, 0, 1).expect("sector readable after write");
        assert_eq!(read_back, data);
        assert!(
            !f.write_sector(0, 0, 19, &data),
            "OOR R must reject write_sector"
        );
        f.eject();
        assert!(
            !f.write_sector(0, 0, 1, &data),
            "no media must reject write_sector"
        );
    }

    /// WRITE DATA path remains the no-media NW stub when no image is attached.
    #[test]
    fn finish_write_data_still_nw_without_media() {
        let mut f = Fdc82077::new();
        assert!(!f.has_media());
        let mut data = [0x5Au8; FDC_SECTOR_SIZE];
        data[0] = 0x11;
        f.latch_write_sector(data);
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_MFM | FDC_CMD_WRITE_DATA));
        for p in [0x00u8, 0x00, 0x00, 0x01, 0x02, 0x12, 0x1B, 0xFF] {
            f.port_write(FDC_FIFO, 1, u32::from(p));
        }
        assert!(f.irq_line());
        let st0 = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(st0 & FDC_ST0_IC_ABNORMAL, FDC_ST0_IC_ABNORMAL);
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, FDC_ST1_NW);
        assert!(
            f.last_write().is_none(),
            "no-media NW path must not keep a successful last_write latch"
        );
        for _ in 0..5 {
            let _ = f.port_read(FDC_FIFO, 1);
        }
        assert_eq!(f.phase, Phase::Command);
    }

    /// Spec: Intel 82077AA §5.1.2 WRITE DATA result — with media + N=2 + valid
    /// CHS, normal termination (IC=00 | H | US), ST1=ST2=0, C/H/R/N ENDaddress,
    /// EOT→`sc_eot`, IRQ6; accepts one 512-byte sector from `latch_write_sector`
    /// (device-only DMA stand-in; does not arm `dma_write_pending`) via
    /// `write_sector` into the image and latches `last_write`. Multi-sector MT
    /// ignored — one sector only. Spec: IBM 1.44MB geometry.
    #[test]
    fn write_data_with_media_normal_result_and_last_write() {
        let mut f = Fdc82077::with_image(vec![0u8; FDC_1440_IMAGE_SIZE]);
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));

        let mut data = [0u8; FDC_SECTOR_SIZE];
        for (i, b) in data.iter_mut().enumerate() {
            *b = (i & 0xFF) as u8;
        }
        f.latch_write_sector(data);

        // WRITE DATA MFM: C=0,H=0,R=1,N=2 — first sector of the image.
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_MFM | FDC_CMD_WRITE_DATA));
        for p in [0x00u8, 0x00, 0x00, 0x01, 0x02, 0x12, 0x1B, 0xFF] {
            f.port_write(FDC_FIFO, 1, u32::from(p));
        }

        assert_eq!(
            f.port_read(FDC_MSR, 1) as u8,
            FDC_MSR_RQM | FDC_MSR_DIO,
            "result phase: RQM|DIO"
        );
        assert!(
            f.irq_line(),
            "WRITE DATA asserts IRQ6 on media success completion"
        );

        let st0 = f.port_read(FDC_FIFO, 1) as u8;
        assert!(!f.irq_line(), "first result byte clears IRQ");
        assert_eq!(
            st0, FDC_ST0_IC_NORMAL,
            "ST0 = IC=00 | H=0 | US=0 (normal termination)"
        );
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x00, "ST1 clear");
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x00, "ST2 clear");
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x00); // C
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x00); // H
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x01); // R
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x02); // N
        assert_eq!(f.phase, Phase::Command);
        assert_eq!(f.sc_eot, 0x12, "EOT latched for DUMPREG");

        let written = *f.last_write().expect("media WRITE DATA latches last_write");
        assert_eq!(written, data);
        let image_sector = f
            .read_sector(0, 0, 1)
            .expect("image must contain written sector");
        assert_eq!(
            image_sector, data,
            "finish_write_data writes latched sector"
        );
        assert!(
            !f.take_pending_dma_write(),
            "pre-latch path must not arm Machine DMA write pending"
        );
    }

    /// Spec: Intel 82077AA DMA mode + DOR bit3 — media WRITE DATA without a
    /// pre-latch arms `dma_write_pending` for MachineBus ISA ch2 Read; image
    /// stays untouched until `commit_dma_write_sector`.
    #[test]
    fn write_data_with_media_arms_dma_write_pending_without_latch() {
        let mut f = Fdc82077::with_image(vec![0xAAu8; FDC_1440_IMAGE_SIZE]);
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_MFM | FDC_CMD_WRITE_DATA));
        for p in [0x00u8, 0x00, 0x00, 0x01, 0x02, 0x12, 0x1B, 0xFF] {
            f.port_write(FDC_FIFO, 1, u32::from(p));
        }
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, FDC_ST0_IC_NORMAL);
        assert!(
            f.last_write().is_none(),
            "deferred until Machine DMA commit"
        );
        let sector = f.read_sector(0, 0, 1).expect("media");
        assert!(
            sector.iter().all(|&b| b == 0xAA),
            "image untouched before commit"
        );
        assert!(f.take_pending_dma_write(), "DOR DMA arms write pending");
        assert!(!f.take_pending_dma_write(), "one-shot arm");
        let mut data = [0u8; FDC_SECTOR_SIZE];
        for (i, b) in data.iter_mut().enumerate() {
            *b = (i & 0xFF) as u8;
        }
        assert!(f.commit_dma_write_sector(data));
        assert_eq!(*f.last_write().expect("commit latches"), data);
        assert_eq!(f.read_sector(0, 0, 1).expect("written"), data);
    }

    /// Spec: Intel 82077AA §5.1.2 / §6.2 — out-of-range sector ID (R>18 on
    /// 1.44MB) with media attached still completes as NW abnormal (no image
    /// write). IBM 1.44MB SPT=18.
    #[test]
    fn write_data_with_media_oor_r_nw_abnormal() {
        let mut f = Fdc82077::with_image(vec![0xAAu8; FDC_1440_IMAGE_SIZE]);
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.latch_write_sector([0x55u8; FDC_SECTOR_SIZE]);

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_MFM | FDC_CMD_WRITE_DATA));
        // C=0,H=0,R=19 (OOR), N=2
        for p in [0x00u8, 0x00, 0x00, 0x13, 0x02, 0x12, 0x1B, 0xFF] {
            f.port_write(FDC_FIFO, 1, u32::from(p));
        }
        assert!(f.irq_line());
        let st0 = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(st0 & FDC_ST0_IC_ABNORMAL, FDC_ST0_IC_ABNORMAL);
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, FDC_ST1_NW);
        assert!(
            f.last_write().is_none(),
            "OOR CHS must not keep a successful last_write latch"
        );
        // First sector of AA image must be untouched.
        let sector = f.read_sector(0, 0, 1).expect("media still attached");
        assert!(
            sector.iter().all(|&b| b == 0xAA),
            "OOR must not write image"
        );
        for _ in 0..5 {
            let _ = f.port_read(FDC_FIFO, 1);
        }
        assert_eq!(f.phase, Phase::Command);
    }
}
