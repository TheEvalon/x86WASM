//! Intel 82077AA floppy disk controller — port stub + Sense Interrupt + IRQ6.
//!
//! Classic PC primary FDC at `0x3F0`–`0x3F7`, **excluding** `0x3F6` (owned by
//! primary IDE alternate status / device control on AT machines).
//!
//! # Spec refs
//!
//! - Intel 82077AA CHMOS Single-Chip Floppy Disk Controller — DOR, MSR, FIFO,
//!   DIR/CCR; Sense Interrupt Status command (`0x08`) result ST0+PCN; DOR bit3
//!   DMA/IRQ enable; IRQ6 on command / reset completion.
//! - OSDev Wiki Floppy Disk Controller — port map; MSR RQM/DIO; Sense Interrupt
//!   clears IRQ; post-reset Sense Interrupt polling.
//! - IBM PC/AT — floppy controller → IRQ6 (8259 master IR6).
//! - `docs/machine-model-pc-v1.md`, `plan.md` §21 Floppy boot (foundation stub).
//!
//! # Scope (this slice)
//!
//! - Accept R/W on SRA/SRB/DOR/TDR/MSR|DSR/FIFO/DIR|CCR with stored values
//! - Reset defaults: MSR = RQM (`0x80`), DOR = `0x00` (controller held in reset)
//! - IRQ6 stub: `assert_irq6` / `clear_irq6` + `irq_line` gated by DOR nRESET∧DMA/IRQ
//! - Sense Interrupt Status (`0x08`): command byte → 2-byte result (ST0, PCN);
//!   clears latched IRQ; MSR RQM|DIO during result phase
//! - `PortDevice` for MachineBus wiring
//!
//! # Unsupported (explicit)
//!
//! - Other commands (Specify/Recalibrate/Seek/READ/WRITE/FORMAT/VERSION/…)
//! - Media image, seek/format/read/write transfers
//! - DMA channel 2 transfers
//! - Automatic IRQ on real media command completion (host may still use assert API)
//! - Drive sensing, disk-change edge timing, perpendicular mode
//! - Non-DMA / FIFO threshold / implied seek

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

/// Sense Interrupt Status command opcode. Spec: Intel 82077AA / OSDev FDC.
pub const FDC_CMD_SENSE_INT: u8 = 0x08;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Idle / accept command byte when RQM && !DIO.
    Command,
    /// Sense Interrupt result: ST0 then PCN.
    SenseIntResult { index: u8 },
}

/// 82077AA-class FDC port stub with Sense Interrupt + IRQ6 assert API.
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
    /// Latched IRQ request (command-complete / reset stub). Spec: 82077AA → ISA IRQ6.
    irq_pending: bool,
    phase: Phase,
    /// Sense Interrupt ST0 result latch.
    sense_st0: u8,
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
            irq_pending: false,
            phase: Phase::Command,
            sense_st0: 0,
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
                Phase::Command => FDC_MSR_RQM,
                Phase::SenseIntResult { .. } => FDC_MSR_RQM | FDC_MSR_DIO,
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
        self.sense_st0 = 0;
    }

    /// Begin Sense Interrupt Status result phase.
    ///
    /// Spec: Intel 82077AA Sense Interrupt Status — no parameters; result ST0,
    /// PCN; clears interrupt. ST0 IC=11 (`0xC0`) models post-reset “ready line
    /// changed” status used by BIOS polling; unit select from DOR[1:0].
    fn start_sense_interrupt(&mut self) {
        self.sense_st0 = 0xC0 | (self.dor & 0x03);
        self.irq_pending = false;
        self.phase = Phase::SenseIntResult { index: 0 };
    }

    fn fifo_read(&mut self) -> u8 {
        match self.phase {
            Phase::Command => 0xFF, // no result pending — open-bus style
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
        }
    }

    fn fifo_write(&mut self, v: u8) {
        // Spec: Intel 82077AA — controller held in reset ignores command stream.
        if self.dor & FDC_DOR_RESET_N == 0 {
            return;
        }
        match self.phase {
            Phase::Command => {
                if v == FDC_CMD_SENSE_INT {
                    self.start_sense_interrupt();
                }
                // Other opcodes: accept/drop until a command engine exists.
            }
            Phase::SenseIntResult { .. } => {
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
        f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ | 0x01));
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
}
