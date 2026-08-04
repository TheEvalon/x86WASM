//! Intel 82077AA floppy disk controller — port stub + Specify/Recalibrate/Seek/
//! Sense Int/Sense Drive/Version/Configure/LOCK/PERPENDICULAR/DUMPREG/
//! READ DATA / WRITE DATA (no-media stubs) + IRQ6.
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
//!   Sense Interrupt Status (`0x08`) result ST0+PCN; Sense Drive Status
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
//!   bytes then 7-byte result; no-media stub completes with ST0 IC=01 +
//!   ST1 ND + C/H/R/N ENDaddress and IRQ6; WRITE DATA (`0x05` with optional
//!   MT/MFM in bits 7:6, §5.1.2 / Table 5-1) same 8-param / 7-result shape;
//!   no-media stub completes with ST0 IC=01 + ST1 NW + C/H/R/N ENDaddress
//!   and IRQ6; DOR bit3 DMA/IRQ enable; IRQ6 on command / reset completion.
//! - OSDev Wiki Floppy Disk Controller — port map; MSR RQM/DIO; Specify timing
//!   params; Recalibrate/Seek → IRQ then Sense Interrupt; Sense Interrupt clears
//!   IRQ; post-reset Sense Interrupt polling; Sense Drive Status ST3 fields;
//!   Version returns `0x90` for 82077AA-class controllers; Configure stores
//!   EIS/FIFO/POLL/FIFOTHR/PRETRK with no result bytes; Lock/Unlock via MT bit;
//!   Perpendicular Mode configures GAP/WGATE (and enhanced Dn bits);
//!   DUMPREG dumps internal registers; READ/WRITE DATA MT/MFM(/SK) forms →
//!   IRQ then 7-byte result (not Sense Interrupt).
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
//! - Sense Interrupt Status (`0x08`): command byte → 2-byte result (ST0, PCN);
//!   returns latched Recalibrate/Seek ST0 when present, else post-reset/`assert_irq6`
//!   stub `0xC0 | DOR[1:0]`; PCN is Present Cylinder Number for ST0 US bits
//!   (`pcn[ST0[1:0]]`); clears latched IRQ; MSR RQM|DIO during result phase
//! - Sense Drive Status (`0x04`): command byte → one HD|US parameter (same
//!   packing as Seek param0) → 1-byte ST3 result; no execution phase, no IRQ
//!   assert/clear; T0 stub reflects `pcn[unit] == 0` for the selected unit;
//!   WP stub always 0 (no media); reserved bits 3/5 always 1 per 82077AA §6.4;
//!   MSR RQM during parameter, RQM|DIO during result phase
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
//!   `sc_eot` updated from READ DATA EOT param; byte7 bits 5:0 reflect
//!   stored PERPENDICULAR D3–D0|GAP|WGATE (OW not returned).
//! - READ DATA (`0x06` | MT/MFM/SK): Spec Intel 82077AA §5.1.1 / Table 5-1 —
//!   command byte lower 5 bits `00110`; optional MT (`0x80`)/MFM (`0x40`)/
//!   SK (`0x20`); eight params (HD|US, C, H, R, N, EOT, GPL, DTL); no media
//!   image → skip execution/DMA, immediate result ST0 (IC=01 abnormal | H |
//!   US), ST1 ND (No Data / sector not found), ST2=0, C/H/R/N = command
//!   ENDaddress; asserts IRQ6 (cleared on first result byte); MSR RQM during
//!   params, RQM|DIO during result. No DMA channel 2 transfer engine.
//! - WRITE DATA (`0x05` | MT/MFM): Spec Intel 82077AA §5.1.2 / Table 5-1 —
//!   command byte lower 5 bits `00101`; optional MT (`0x80`)/MFM (`0x40`);
//!   same eight params and 7-byte result as READ DATA; no media image → skip
//!   execution/DMA, immediate result ST0 (IC=01 abnormal | H | US), ST1 NW
//!   (Not Writable — no media to write), ST2=0, C/H/R/N = command ENDaddress;
//!   asserts IRQ6 (cleared on first result byte); EOT latched into `sc_eot`.
//! - `PortDevice` for MachineBus wiring
//!
//! # Unsupported (explicit)
//!
//! - READ DELETED DATA / WRITE DELETED / FORMAT / READ TRACK /
//!   READ ID / VERIFY and other transfer commands
//! - Media image, seek step timing, real sector transfers
//! - DMA channel 2 transfers (ND bit / Specify stored only; not enforced)
//! - Implied seek from Configure EIS; multi-sector TC termination
//! - Drive sensing, disk-change edge timing
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
/// READ DATA base opcode (bits 4:0). Spec: Intel 82077AA §5.1.1 / Table 5-1 —
/// command byte is `MT|MFM|SK|0 0 1 1 0`; match with [`FDC_CMD_OPCODE_MASK`].
pub const FDC_CMD_READ_DATA: u8 = 0x06;
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
/// SeaBIOS-common READ DATA form: MT|MFM|SK|0x06. Spec: 82077AA Table 5-1.
pub const FDC_CMD_READ_DATA_MT_MFM_SK: u8 =
    FDC_CMD_MT | FDC_CMD_MFM | FDC_CMD_SK | FDC_CMD_READ_DATA;
/// WRITE DATA parameter count after the command byte. Spec: 82077AA §5.1.2.
pub const FDC_WRITE_DATA_PARAM_LEN: u8 = 8;
/// WRITE DATA result byte count (ST0, ST1, ST2, C, H, R, N). Spec: 82077AA §5.1.2.
pub const FDC_WRITE_DATA_RESULT_LEN: u8 = 7;
/// READ DATA parameter count after the command byte. Spec: 82077AA §5.1.1.
pub const FDC_READ_DATA_PARAM_LEN: u8 = 8;
/// READ DATA result byte count (ST0, ST1, ST2, C, H, R, N). Spec: 82077AA §5.1.1.
pub const FDC_READ_DATA_RESULT_LEN: u8 = 7;
/// Recalibrate command opcode. Spec: Intel 82077AA / OSDev FDC — 1 unit parameter.
pub const FDC_CMD_RECALIBRATE: u8 = 0x07;
/// Sense Interrupt Status command opcode. Spec: Intel 82077AA / OSDev FDC.
pub const FDC_CMD_SENSE_INT: u8 = 0x08;
/// DUMPREG command opcode. Spec: Intel 82077AA §5.2.10 / Table 5-1 — no
/// parameters; 10-byte result dumping internal registers; no IRQ.
pub const FDC_CMD_DUMPREG: u8 = 0x0E;
/// Number of DUMPREG result bytes. Spec: Intel 82077AA Table 5-1 / §5.3.3.
pub const FDC_DUMPREG_RESULT_LEN: u8 = 10;
/// Seek command opcode. Spec: Intel 82077AA / OSDev FDC — HD|US + NCN.
pub const FDC_CMD_SEEK: u8 = 0x0F;
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
    /// WRITE DATA parameters: 8 bytes (HD|US, C, H, R, N, EOT, GPL, DTL).
    /// Spec: Intel 82077AA §5.1.2 / Table 5-1.
    WriteDataParams { index: u8 },
    /// WRITE DATA result: 7 bytes (ST0, ST1, ST2, C, H, R, N). Spec: §5.1.2.
    WriteDataResult { index: u8 },
}

/// 82077AA-class FDC port stub with Specify/Recalibrate/Seek/Sense/Version/
/// Configure/LOCK/PERPENDICULAR/DUMPREG/READ·WRITE DATA (no-media) + IRQ6.
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
    /// Last SC (FORMAT) or EOT (READ/WRITE/…) parameter. Spec: Intel 82077AA
    /// Table 5-1 note — DUMPREG result byte 6. Updated by READ/WRITE DATA EOT.
    pub sc_eot: u8,
    /// Latched IRQ request (command-complete / reset stub). Spec: 82077AA → ISA IRQ6.
    irq_pending: bool,
    phase: Phase,
    /// Command-completion ST0 for Sense Interrupt (Recalibrate/Seek Seek End); consumed once.
    pending_sense_st0: Option<u8>,
    /// Seek param0 (HD|US) latched between the two Seek parameter bytes.
    seek_head_unit: u8,
    /// Sense Interrupt ST0 result byte (set when entering result phase).
    sense_st0: u8,
    /// Sense Interrupt PCN result byte (PCN of unit in `sense_st0` US bits).
    sense_pcn: u8,
    /// Sense Drive Status ST3 result byte (set when entering result phase).
    sense_st3: u8,
    /// READ/WRITE DATA parameter bytes (8). Spec: Intel 82077AA §5.1.1 / §5.1.2.
    /// Shared — transfer commands are mutually exclusive in the command stream.
    read_params: [u8; FDC_READ_DATA_PARAM_LEN as usize],
    /// READ/WRITE DATA result bytes (ST0..N). Spec: Intel 82077AA §5.1.1 / §5.1.2.
    read_result: [u8; FDC_READ_DATA_RESULT_LEN as usize],
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
            sense_st0: 0,
            sense_pcn: 0,
            sense_st3: 0,
            read_params: [0; FDC_READ_DATA_PARAM_LEN as usize],
            read_result: [0; FDC_READ_DATA_RESULT_LEN as usize],
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
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
                | Phase::SenseDriveStatusParam
                | Phase::ConfigureParams { .. }
                | Phase::PerpendicularParam
                | Phase::ReadDataParams { .. }
                | Phase::WriteDataParams { .. } => FDC_MSR_RQM,
                Phase::SenseIntResult { .. }
                | Phase::SenseDriveStatusResult
                | Phase::VersionResult
                | Phase::LockResult
                | Phase::DumpRegResult { .. }
                | Phase::ReadDataResult { .. }
                | Phase::WriteDataResult { .. } => FDC_MSR_RQM | FDC_MSR_DIO,
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

    /// Begin Sense Interrupt Status result phase.
    ///
    /// Spec: Intel 82077AA Sense Interrupt Status — no parameters; result ST0,
    /// PCN; clears interrupt. When a seek-class command latched ST0 (Recalibrate
    /// / Seek), return that value; otherwise ST0 IC=11 (`0xC0`) models post-reset
    /// “ready line changed” / `assert_irq6`-only status; unit select from DOR[1:0].
    /// PCN is the Present Cylinder Number for the unit in ST0 bits 1:0.
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
    /// reflects `pcn[unit] == 0` for the selected unit, WP stub always 0 (no
    /// media), reserved bits 3/5 always 1. No IRQ.
    fn finish_sense_drive_status(&mut self, param: u8) {
        let head_unit = param & (FDC_ST3_HEAD | FDC_ST3_UNIT_MASK);
        let unit = (param & FDC_ST3_UNIT_MASK) as usize;
        let mut st3 = head_unit | FDC_ST3_RESERVED_BIT3 | FDC_ST3_RESERVED_BIT5;
        if self.pcn[unit] == 0 {
            st3 |= FDC_ST3_TRACK0;
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

    /// Complete READ DATA after eight parameters — no-media stub.
    ///
    /// Spec: Intel 82077AA §5.1.1 / §6.1 / §6.2 — with no media image this stub
    /// skips the execution/DMA transfer phase and enters result immediately:
    /// ST0 IC=01 (abnormal termination) | H | US; ST1 ND (sector not found);
    /// ST2=0; C/H/R/N reflect command ENDaddress (sector ID after command);
    /// latches EOT into `sc_eot` (DUMPREG Table 5-1); asserts IRQ (cleared
    /// when the host reads the first result byte). No Sense Interrupt.
    fn finish_read_data(&mut self) {
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
        self.phase = Phase::ReadDataResult { index: 0 };
    }

    /// Begin WRITE DATA parameter phase (8 bytes). Spec: Intel 82077AA §5.1.2.
    fn start_write_data(&mut self) {
        self.read_params = [0; FDC_WRITE_DATA_PARAM_LEN as usize];
        self.phase = Phase::WriteDataParams { index: 0 };
    }

    /// Complete WRITE DATA after eight parameters — no-media stub.
    ///
    /// Spec: Intel 82077AA §5.1.2 / §6.1 / §6.2 — with no media image this stub
    /// skips the execution/DMA transfer phase and enters result immediately:
    /// ST0 IC=01 (abnormal termination) | H | US; ST1 NW (Not Writable — no
    /// media to accept a write); ST2=0; C/H/R/N reflect command ENDaddress;
    /// latches EOT into `sc_eot` (DUMPREG Table 5-1); asserts IRQ (cleared
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
        // GPL (params[6]) and DTL (params[7]) accepted; unused without media.

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

    /// True if `cmd` is WRITE DATA including optional MT/MFM modifiers.
    ///
    /// Spec: Intel 82077AA §5.1.2 / Table 5-1 — opcode bits 4:0 = `00101`;
    /// bits 7:6 are MT/MFM (`FDC_CMD_MT` / `FDC_CMD_MFM`); bit5 is 0 in the
    /// documented form (SK not used for WRITE DATA).
    #[inline]
    fn is_write_data_command(cmd: u8) -> bool {
        cmd & FDC_CMD_OPCODE_MASK == (FDC_CMD_WRITE_DATA_MT_MFM & FDC_CMD_OPCODE_MASK)
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
            // Spec: Specify/Recalibrate/Seek/Configure/PERPENDICULAR have no result
            // phase; open-bus when idle/params.
            Phase::Command
            | Phase::SpecifyParams { .. }
            | Phase::RecalibrateParams
            | Phase::SeekParams { .. }
            | Phase::SenseDriveStatusParam
            | Phase::ConfigureParams { .. }
            | Phase::PerpendicularParam
            | Phase::ReadDataParams { .. }
            | Phase::WriteDataParams { .. } => 0xFF,
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
                } else if Self::is_write_data_command(v) {
                    // Spec: Intel 82077AA §5.1.2 — MT/MFM | 00101; eight params.
                    self.start_write_data();
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
            Phase::WriteDataParams { index } => {
                // Spec: Intel 82077AA §5.1.2 — HD|US, C, H, R, N, EOT, GPL, DTL.
                self.read_params[index as usize] = v;
                if index + 1 >= FDC_WRITE_DATA_PARAM_LEN {
                    self.finish_write_data();
                } else {
                    self.phase = Phase::WriteDataParams { index: index + 1 };
                }
            }
            Phase::SenseIntResult { .. }
            | Phase::SenseDriveStatusResult
            | Phase::VersionResult
            | Phase::LockResult
            | Phase::DumpRegResult { .. }
            | Phase::ReadDataResult { .. }
            | Phase::WriteDataResult { .. } => {
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

    /// Spec: WP reflects the WP pin; stub has no media, so always 0.
    #[test]
    fn sense_drive_status_write_protect_stub_always_clear() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_DRIVE_STATUS));
        f.port_write(FDC_FIFO, 1, 0x00);
        let st3 = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(
            st3 & FDC_ST3_WRITE_PROTECT,
            0,
            "no media in stub: WP always clear"
        );
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
}
