//! Intel 82077AA floppy disk controller — port stub + Specify/Recalibrate/Seek/
//! Sense Int/Sense Drive/Version + IRQ6.
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
//!   `0x90` (82077AA identification); DOR bit3 DMA/IRQ enable; IRQ6 on command /
//!   reset completion.
//! - OSDev Wiki Floppy Disk Controller — port map; MSR RQM/DIO; Specify timing
//!   params; Recalibrate/Seek → IRQ then Sense Interrupt; Sense Interrupt clears
//!   IRQ; post-reset Sense Interrupt polling; Sense Drive Status ST3 fields;
//!   Version returns `0x90` for 82077AA-class controllers.
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
//!   sets `pcn = 0`, latches ST0 Seek End (`0x20 | unit`) for Sense Interrupt,
//!   asserts IRQ; MSR RQM (!DIO) during parameter phase; no result phase
//! - Seek (`0x0F`): command byte → HD|US + NCN; sets `pcn = NCN`, latches ST0
//!   Seek End (`0x20 | unit`; H bit always 0 per 82077AA), asserts IRQ; no result
//! - Sense Interrupt Status (`0x08`): command byte → 2-byte result (ST0, PCN);
//!   returns latched Recalibrate/Seek ST0 when present, else post-reset/`assert_irq6`
//!   stub `0xC0 | DOR[1:0]`; clears latched IRQ; MSR RQM|DIO during result phase
//! - Sense Drive Status (`0x04`): command byte → one HD|US parameter (same
//!   packing as Seek param0) → 1-byte ST3 result; no execution phase, no IRQ
//!   assert/clear; T0 stub reflects `pcn == 0` (single shared `pcn`, not
//!   per-drive); WP stub always 0 (no media); reserved bits 3/5 always 1 per
//!   82077AA §6.4; MSR RQM during parameter, RQM|DIO during result phase
//! - Version (`0x10`): command byte → 1-byte result `0x90` (82077AA id); no
//!   parameters, no IRQ assert/clear; MSR RQM|DIO during result phase
//! - `PortDevice` for MachineBus wiring
//!
//! # Unsupported (explicit)
//!
//! - Other commands (READ/WRITE/FORMAT/Configure/LOCK/PERPENDICULAR/DUMPREG/…)
//! - Media image, seek step timing, format/read/write transfers
//! - DMA channel 2 transfers (ND bit stored only; not enforced)
//! - Automatic IRQ on real media command completion (host may still use assert API)
//! - Drive sensing, disk-change edge timing, perpendicular mode
//! - FIFO threshold / implied seek

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
/// Recalibrate command opcode. Spec: Intel 82077AA / OSDev FDC — 1 unit parameter.
pub const FDC_CMD_RECALIBRATE: u8 = 0x07;
/// Sense Interrupt Status command opcode. Spec: Intel 82077AA / OSDev FDC.
pub const FDC_CMD_SENSE_INT: u8 = 0x08;
/// Seek command opcode. Spec: Intel 82077AA / OSDev FDC — HD|US + NCN.
pub const FDC_CMD_SEEK: u8 = 0x0F;
/// Version command opcode. Spec: Intel 82077AA / OSDev FDC — no parameters,
/// 1-byte result identifying the controller class.
pub const FDC_CMD_VERSION: u8 = 0x10;
/// Version result byte for 82077AA-class controllers. Spec: Intel 82077AA /
/// OSDev FDC — `0x90` identifies enhanced/82077AA (vs `0x80` for older 8272A).
pub const FDC_VERSION_82077AA: u8 = 0x90;
/// ST0 Seek End (SE) bit. Spec: Intel 82077AA status register 0.
pub const FDC_ST0_SEEK_END: u8 = 0x20;
/// ST0 Interrupt Code = 11 (abnormal/ready-line-changed stub). Spec: 82077AA / OSDev.
pub const FDC_ST0_IC_READY_CHANGE: u8 = 0xC0;

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
}

/// 82077AA-class FDC port stub with Specify/Recalibrate/Seek/Sense/Version + IRQ6.
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
    /// Present cylinder number stub (Sense Interrupt result byte 2).
    pub pcn: u8,
    /// Specify parameter 1: SRT (bits 7–4) | HUT (bits 3–0). Spec: 82077AA.
    pub specify_srt_hut: u8,
    /// Specify parameter 2: HLT (bits 7–1) | ND (bit 0). Spec: 82077AA.
    pub specify_hlt_nd: u8,
    /// Latched IRQ request (command-complete / reset stub). Spec: 82077AA → ISA IRQ6.
    irq_pending: bool,
    phase: Phase,
    /// Command-completion ST0 for Sense Interrupt (Recalibrate/Seek Seek End); consumed once.
    pending_sense_st0: Option<u8>,
    /// Seek param0 (HD|US) latched between the two Seek parameter bytes.
    seek_head_unit: u8,
    /// Sense Interrupt ST0 result byte (set when entering result phase).
    sense_st0: u8,
    /// Sense Drive Status ST3 result byte (set when entering result phase).
    sense_st3: u8,
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
            pcn: 0x00,
            specify_srt_hut: 0x00,
            specify_hlt_nd: 0x00,
            irq_pending: false,
            phase: Phase::Command,
            pending_sense_st0: None,
            seek_head_unit: 0,
            sense_st0: 0,
            sense_st3: 0,
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
                | Phase::SenseDriveStatusParam => FDC_MSR_RQM,
                Phase::SenseIntResult { .. }
                | Phase::SenseDriveStatusResult
                | Phase::VersionResult => FDC_MSR_RQM | FDC_MSR_DIO,
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
    /// Spec: Intel 82077AA Recalibrate — retracts head to track 0; on completion
    /// PCN=0, ST0 SE|US (`0x20 | unit`), interrupt asserted; host uses Sense
    /// Interrupt Status (no Recalibrate result phase).
    fn finish_recalibrate(&mut self, param: u8) {
        let unit = param & 0x03;
        self.pcn = 0;
        self.pending_sense_st0 = Some(FDC_ST0_SEEK_END | unit);
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
    /// Spec: Intel 82077AA Seek — steps to NCN; on completion PCN=NCN, ST0
    /// SE|US (`0x20 | unit`; H in ST0 always 0), interrupt asserted; host uses
    /// Sense Interrupt Status (no Seek result phase). OSDev: param0 = (HD<<2)|US.
    fn finish_seek(&mut self, ncn: u8) {
        let unit = self.seek_head_unit & 0x03;
        self.pcn = ncn;
        self.pending_sense_st0 = Some(FDC_ST0_SEEK_END | unit);
        self.irq_pending = true;
        self.phase = Phase::Command;
    }

    /// Begin Sense Interrupt Status result phase.
    ///
    /// Spec: Intel 82077AA Sense Interrupt Status — no parameters; result ST0,
    /// PCN; clears interrupt. When a seek-class command latched ST0 (Recalibrate
    /// / Seek), return that value; otherwise ST0 IC=11 (`0xC0`) models post-reset
    /// “ready line changed” / `assert_irq6`-only status; unit select from DOR[1:0].
    fn start_sense_interrupt(&mut self) {
        self.sense_st0 = self
            .pending_sense_st0
            .take()
            .unwrap_or(FDC_ST0_IC_READY_CHANGE | (self.dor & 0x03));
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
    /// reflects `pcn == 0` (single shared `pcn`, not per-drive in this stub),
    /// WP stub always 0 (no media), reserved bits 3/5 always 1. No IRQ.
    fn finish_sense_drive_status(&mut self, param: u8) {
        let head_unit = param & (FDC_ST3_HEAD | FDC_ST3_UNIT_MASK);
        let mut st3 = head_unit | FDC_ST3_RESERVED_BIT3 | FDC_ST3_RESERVED_BIT5;
        if self.pcn == 0 {
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

    fn fifo_read(&mut self) -> u8 {
        match self.phase {
            // Spec: Specify/Recalibrate/Seek have no result phase; open-bus when idle/params.
            Phase::Command
            | Phase::SpecifyParams { .. }
            | Phase::RecalibrateParams
            | Phase::SeekParams { .. }
            | Phase::SenseDriveStatusParam => 0xFF,
            Phase::SenseIntResult { index } => {
                let v = match index {
                    0 => self.sense_st0,
                    _ => self.pcn,
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
                } else if v == FDC_CMD_VERSION {
                    // Spec: Intel 82077AA Version — no params; result 0x90; no IRQ.
                    self.start_version();
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
            Phase::SenseIntResult { .. } | Phase::SenseDriveStatusResult | Phase::VersionResult => {
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
        f.pcn = 0x12;
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
        assert_eq!(pcn, 0x12);
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
        f.pcn = 0x2A;
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
        assert_eq!(f.pcn, 0, "Recalibrate forces PCN=0");
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
        f.pcn = 0x05;
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_RECALIBRATE));
        f.port_write(FDC_FIFO, 1, 0x01);
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        assert_eq!(f.pcn, 0x05, "ignored while reset");
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
        f.pcn = 0x00;
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
        assert_eq!(f.pcn, 0x00, "PCN unchanged until NCN");
        assert!(!f.irq_line());

        f.port_write(FDC_FIFO, 1, 0x28); // NCN
        assert_eq!(f.pcn, 0x28, "Seek sets PCN = NCN");
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
    /// result (no execution phase, no IRQ). Track 0 reflects `pcn==0` (stub).
    #[test]
    fn sense_drive_status_result_reflects_track0_head_and_unit() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.pcn = 0x00;

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
            "T0 (pcn==0) | reserved bits | HD|US from param"
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

    /// Spec: T0 bit reflects TRK0 pin state (stub: `pcn==0`); clear when pcn!=0.
    #[test]
    fn sense_drive_status_track0_clear_when_pcn_nonzero() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        f.pcn = 0x28;

        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_DRIVE_STATUS));
        f.port_write(FDC_FIFO, 1, 0x01); // unit 1, head 0
        let st3 = f.port_read(FDC_FIFO, 1) as u8;
        assert_eq!(
            st3 & FDC_ST3_TRACK0,
            0,
            "T0 clear when pcn!=0 (stub, single shared pcn)"
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
        f.pcn = 0x05;
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SEEK));
        f.port_write(FDC_FIFO, 1, 0x01);
        f.port_write(FDC_FIFO, 1, 0x20);
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        assert_eq!(f.pcn, 0x05, "ignored while reset");
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
}
