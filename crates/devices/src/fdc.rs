//! Intel 82077AA floppy disk controller — port register stub (no media engine).
//!
//! Classic PC primary FDC at `0x3F0`–`0x3F7`, **excluding** `0x3F6` (owned by
//! primary IDE alternate status / device control on AT machines).
//!
//! # Spec refs
//!
//! - Intel 82077AA CHMOS Single-Chip Floppy Disk Controller — DOR, MSR, FIFO,
//!   DIR/CCR programming model.
//! - OSDev Wiki Floppy Disk Controller — port map `0x3F0`–`0x3F7` excluding
//!   `0x3F6`; MSR RQM bit; DOR motor/reset/DMA-IRQ enable.
//! - `docs/machine-model-pc-v1.md`, `plan.md` §21 Floppy boot (foundation stub).
//!
//! # Scope (this slice)
//!
//! - Accept R/W on SRA/SRB/DOR/TDR/MSR|DSR/FIFO/DIR|CCR with stored values
//! - Reset defaults: MSR = RQM (`0x80`), DOR = `0x00` (controller held in reset)
//! - `PortDevice` for MachineBus wiring
//!
//! # Unsupported (explicit)
//!
//! - Command execution / result phases beyond FIFO byte accept
//! - Media image, seek/format/read/write transfers
//! - DMA channel 2 / IRQ6 assertion
//! - Drive sensing, disk-change edge timing, perpendicular mode

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
/// DOR bit2 — when clear, FDC held in reset. Spec: Intel 82077AA / OSDev.
pub const FDC_DOR_RESET_N: u8 = 0x04;

/// 82077AA-class FDC port stub.
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
    /// Last FIFO byte written (no command engine — read returns last write).
    pub fifo_latched: u8,
    /// Status A read value (fixed stub).
    pub sra: u8,
    /// Status B read value (fixed stub).
    pub srb: u8,
    /// Digital Input Register read value (disk-change stub; bit7 often media).
    pub dir: u8,
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
            fifo_latched: 0x00,
            // Open-bus style defaults for largely unused status ports.
            sra: 0x00,
            srb: 0x00,
            dir: 0x00,
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

    /// Main Status Register for the stub: RQM set when not held in DOR reset.
    ///
    /// Spec: OSDev FDC / Intel 82077AA — RQM indicates FIFO may be touched;
    /// without a command engine we advertise RQM whenever out of reset so
    /// firmware polls do not spin forever.
    pub fn msr(&self) -> u8 {
        if self.dor & FDC_DOR_RESET_N == 0 {
            0
        } else {
            FDC_MSR_RQM
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
            FDC_FIFO => self.fifo_latched,
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
            FDC_DOR => self.dor = v,
            FDC_TDR => self.tdr = v,
            FDC_MSR => self.dsr = v, // DSR write-only side
            FDC_FIFO => self.fifo_latched = v,
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
    fn dor_dsr_ccr_fifo_round_trip() {
        let mut f = Fdc82077::new();
        f.port_write(FDC_DOR, 1, 0x1C); // nRESET + DMA/IRQ + motor0 style
        assert_eq!(f.port_read(FDC_DOR, 1) as u8, 0x1C);
        f.port_write(FDC_MSR, 1, 0x02); // DSR
        assert_eq!(f.dsr, 0x02);
        // MSR read side is status, not DSR.
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, FDC_MSR_RQM);
        f.port_write(FDC_DIR_CCR, 1, 0x00);
        assert_eq!(f.ccr, 0x00);
        f.port_write(FDC_FIFO, 1, 0x08); // e.g. Sense Interrupt Status opcode byte
        assert_eq!(f.port_read(FDC_FIFO, 1) as u8, 0x08);
        assert_eq!(f.fifo_latched, 0x08);
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
        f.port_write(FDC_FIFO, 1, 0xAA);
        f.port_write(FDC_DIR_CCR, 1, 0x01);
        f.reset();
        assert_eq!(f.dor, 0);
        assert_eq!(f.fifo_latched, 0);
        assert_eq!(f.ccr, 0);
        assert_eq!(f.port_read(FDC_MSR, 1) as u8, 0);
    }
}
