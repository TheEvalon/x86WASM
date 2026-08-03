//! MC146818-compatible CMOS / RTC register file (ports `0x70` / `0x71`).
//!
//! # Spec refs
//!
//! - Motorola MC146818 Real-Time Clock Plus RAM datasheet — address/data
//!   multiplexing, register map 0x00–0x0D (time + status A–D), CMOS RAM.
//! - IBM PC/AT Technical Reference — CMOS index port `0x70` (bit7 = NMI mask),
//!   data port `0x71`.
//! - `docs/machine-model-pc-v1.md`, `plan.md` §15.3 RTC.
//!
//! # Scope (this slice)
//!
//! 128-byte register bank with index/data port access and NMI-mask bit tracking.
//! Status A/B/C/D defaults are documented; time fields start at zero.
//!
//! # Unsupported (explicit)
//!
//! - IRQ8 / PIE / AIE / UIE interrupt delivery
//! - Host wall-clock sync / automatic time progression
//! - NMI signal delivery (bit7 on `0x70` is stored only)
//! - Century byte / ACPI extended CMOS beyond 128 bytes
//! - Wiring into `machine-pc` / `MachineBus`

use crate::PortDevice;

/// CMOS index / NMI-mask port (classic PC).
pub const CMOS_INDEX: u16 = 0x70;
/// CMOS data port (classic PC).
pub const CMOS_DATA: u16 = 0x71;

/// Status Register A.
pub const REG_STATUS_A: u8 = 0x0A;
/// Status Register B.
pub const REG_STATUS_B: u8 = 0x0B;
/// Status Register C (IRQ flags; read-to-clear on real hardware).
pub const REG_STATUS_C: u8 = 0x0C;
/// Status Register D (valid RAM / battery).
pub const REG_STATUS_D: u8 = 0x0D;

/// Index port bit7: NMI disable when set (PC/AT).
const NMI_DISABLE: u8 = 1 << 7;
const INDEX_MASK: u8 = 0x7F;

/// Default Status A: UIP=0, divider=010 (32.768 kHz), rate=0110 (1024 Hz).
/// Common AT POST default; UIP never set in this slice (no timebase).
const DEFAULT_STATUS_A: u8 = 0x26;
/// Default Status B: 24-hour mode bit (DM/binary cleared → BCD; PIE/AIE/UIE off).
const DEFAULT_STATUS_B: u8 = 0x02;
/// Default Status C: no IRQ flags pending.
const DEFAULT_STATUS_C: u8 = 0x00;
/// Default Status D: VRT=1 (valid RAM and time / battery OK).
const DEFAULT_STATUS_D: u8 = 0x80;

/// 128-byte CMOS/RTC image with index+data port access.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CmosRtc {
    /// Register file (0x00–0x7F).
    pub ram: [u8; 128],
    /// Last index written (low 7 bits); bit7 tracks NMI disable separately.
    index: u8,
    /// NMI-disable latch from index-port bit7 (not delivered as NMI).
    pub nmi_disabled: bool,
}

impl CmosRtc {
    pub fn new() -> Self {
        let mut s = Self {
            ram: [0; 128],
            index: 0,
            nmi_disabled: false,
        };
        s.apply_reset_defaults();
        s
    }

    fn apply_reset_defaults(&mut self) {
        self.ram = [0; 128];
        self.ram[REG_STATUS_A as usize] = DEFAULT_STATUS_A;
        self.ram[REG_STATUS_B as usize] = DEFAULT_STATUS_B;
        self.ram[REG_STATUS_C as usize] = DEFAULT_STATUS_C;
        self.ram[REG_STATUS_D as usize] = DEFAULT_STATUS_D;
        self.index = 0;
        self.nmi_disabled = false;
    }

    pub fn reset(&mut self) {
        self.apply_reset_defaults();
    }

    pub fn selected_index(&self) -> u8 {
        self.index & INDEX_MASK
    }

    pub fn read_reg(&self, index: u8) -> u8 {
        self.ram[(index & INDEX_MASK) as usize]
    }

    pub fn write_reg(&mut self, index: u8, value: u8) {
        self.ram[(index & INDEX_MASK) as usize] = value;
    }
}

impl Default for CmosRtc {
    fn default() -> Self {
        Self::new()
    }
}

impl PortDevice for CmosRtc {
    fn port_read(&mut self, port: u16, _size: u8) -> u32 {
        match port {
            // Index port reads are undefined on many chipsets; return last index|NMI.
            CMOS_INDEX => {
                let nmi = if self.nmi_disabled { NMI_DISABLE } else { 0 };
                u32::from(self.selected_index() | nmi)
            }
            CMOS_DATA => {
                let idx = self.selected_index();
                let value = self.ram[idx as usize];
                // Status C: IRQ flags are read-to-clear on MC146818. This slice
                // keeps them as sticky zeros, so the clear is a no-op but the
                // read path documents the architectural side effect.
                if idx == REG_STATUS_C {
                    self.ram[REG_STATUS_C as usize] = 0;
                }
                u32::from(value)
            }
            _ => 0xFFFF_FFFF,
        }
    }

    fn port_write(&mut self, port: u16, _size: u8, value: u32) {
        let v = value as u8;
        match port {
            CMOS_INDEX => {
                self.nmi_disabled = v & NMI_DISABLE != 0;
                self.index = v & INDEX_MASK;
            }
            CMOS_DATA => {
                let idx = self.selected_index();
                // Status C is read-only on real RTC; ignore writes.
                if idx == REG_STATUS_C {
                    return;
                }
                self.ram[idx as usize] = v;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spec: reset defaults for status A–D; time/calendar zeros (MC146818 / PC AT).
    #[test]
    fn reset_state() {
        let c = CmosRtc::new();
        assert_eq!(c.read_reg(0x00), 0);
        assert_eq!(c.read_reg(REG_STATUS_A), DEFAULT_STATUS_A);
        assert_eq!(c.read_reg(REG_STATUS_B), DEFAULT_STATUS_B);
        assert_eq!(c.read_reg(REG_STATUS_C), DEFAULT_STATUS_C);
        assert_eq!(c.read_reg(REG_STATUS_D), DEFAULT_STATUS_D);
        assert!(!c.nmi_disabled);
        assert_eq!(c.selected_index(), 0);

        let mut c2 = CmosRtc::new();
        c2.port_write(CMOS_INDEX, 1, 0x10);
        c2.port_write(CMOS_DATA, 1, 0xAB);
        c2.reset();
        assert_eq!(c2, CmosRtc::new());
    }

    #[test]
    fn index_data_read_write() {
        let mut c = CmosRtc::new();
        c.port_write(CMOS_INDEX, 1, 0x10);
        c.port_write(CMOS_DATA, 1, 0x5A);
        assert_eq!(c.selected_index(), 0x10);
        assert_eq!(c.port_read(CMOS_DATA, 1) as u8, 0x5A);
        assert_eq!(c.read_reg(0x10), 0x5A);
    }

    /// Spec: port 0x70 bit7 is NMI mask; low 7 bits select register.
    #[test]
    fn nmi_disable_bit_tracked() {
        let mut c = CmosRtc::new();
        c.port_write(CMOS_INDEX, 1, 0x80 | 0x0B);
        assert!(c.nmi_disabled);
        assert_eq!(c.selected_index(), REG_STATUS_B);
        c.port_write(CMOS_DATA, 1, 0x06);
        assert_eq!(c.read_reg(REG_STATUS_B), 0x06);

        c.port_write(CMOS_INDEX, 1, 0x0B); // clear NMI disable
        assert!(!c.nmi_disabled);
        assert_eq!(c.selected_index(), REG_STATUS_B);
    }

    #[test]
    fn index_masked_to_7_bits() {
        let mut c = CmosRtc::new();
        // 0xFF → index 0x7F with NMI disable
        c.port_write(CMOS_INDEX, 1, 0xFF);
        assert_eq!(c.selected_index(), 0x7F);
        assert!(c.nmi_disabled);
        c.port_write(CMOS_DATA, 1, 0x11);
        assert_eq!(c.read_reg(0x7F), 0x11);
        // Direct write_reg also masks.
        c.write_reg(0x80 | 0x05, 0x22);
        assert_eq!(c.read_reg(0x05), 0x22);
    }

    #[test]
    fn status_c_read_to_clear_stays_zero() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_STATUS_C, 0xF0); // inject flags for the side-effect path
        c.port_write(CMOS_INDEX, 1, u32::from(REG_STATUS_C));
        assert_eq!(c.port_read(CMOS_DATA, 1) as u8, 0xF0);
        assert_eq!(c.read_reg(REG_STATUS_C), 0);
        // Writes to C ignored.
        c.port_write(CMOS_DATA, 1, 0xFF);
        assert_eq!(c.read_reg(REG_STATUS_C), 0);
    }

    #[test]
    fn state_clone_equality_round_trip() {
        let mut c = CmosRtc::new();
        c.port_write(CMOS_INDEX, 1, 0x80 | 0x14);
        c.port_write(CMOS_DATA, 1, 0xBE);
        let cloned = c.clone();
        assert_eq!(c, cloned);
        assert!(cloned.nmi_disabled);
        assert_eq!(cloned.read_reg(0x14), 0xBE);
    }

    #[test]
    fn unrelated_ports_ignored() {
        let mut c = CmosRtc::new();
        c.port_write(0x3F8, 1, 0x10);
        assert_eq!(c.selected_index(), 0);
        assert_eq!(c.port_read(0x3F8, 1), 0xFFFF_FFFF);
    }
}
