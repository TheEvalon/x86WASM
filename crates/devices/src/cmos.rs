//! MC146818-compatible CMOS / RTC register file (ports `0x70` / `0x71`).
//!
//! # Spec refs
//!
//! - Motorola MC146818 Real-Time Clock Plus RAM datasheet — address/data
//!   multiplexing, register map 0x00–0x0D (time + status A–D), status B
//!   PIE/AIE/UIE, status C PF/AF/UF/IRQF (read-to-clear), IRQ pin.
//! - IBM PC/AT Technical Reference — CMOS index port `0x70` (bit7 = NMI mask),
//!   data port `0x71`; RTC IRQ → ISA IRQ8 (8259A slave IR0).
//! - `docs/machine-model-pc-v1.md`, `plan.md` §15.3 RTC.
//!
//! # Scope (this slice)
//!
//! 128-byte register bank with index/data port access, NMI-mask bit tracking,
//! status B PIE/AIE/UIE subset, model `tick` that sets PF/UF (and AF on alarm
//! match), IRQF → IRQ line for MachineBus → DualPic IRQ8.
//!
//! # Unsupported (explicit)
//!
//! - Host wall-clock sync / automatic calendar progression
//! - NMI signal delivery (bit7 on `0x70` is stored only)
//! - Status A UIP bit / exact crystal divider timing (period quantum is explicit)
//! - Century byte / ACPI extended CMOS beyond 128 bytes
//! - Square-wave output (SQWE)

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

/// Seconds / minutes / hours (time).
const REG_SEC: u8 = 0x00;
const REG_MIN: u8 = 0x02;
const REG_HOUR: u8 = 0x04;
/// Alarm seconds / minutes / hours.
const REG_SEC_ALARM: u8 = 0x01;
const REG_MIN_ALARM: u8 = 0x03;
const REG_HOUR_ALARM: u8 = 0x05;

/// Index port bit7: NMI disable when set (PC/AT).
const NMI_DISABLE: u8 = 1 << 7;
const INDEX_MASK: u8 = 0x7F;

/// Status B: SET (inhibit update), PIE, AIE, UIE.
pub const STB_SET: u8 = 1 << 7;
pub const STB_PIE: u8 = 1 << 6;
pub const STB_AIE: u8 = 1 << 5;
pub const STB_UIE: u8 = 1 << 4;

/// Status C: IRQF, PF, AF, UF (bits 3:0 reserved 0).
pub const STC_IRQF: u8 = 1 << 7;
pub const STC_PF: u8 = 1 << 6;
pub const STC_AF: u8 = 1 << 5;
pub const STC_UF: u8 = 1 << 4;

/// Default Status A: UIP=0, divider=010 (32.768 kHz), rate=0110 (1024 Hz).
/// Common AT POST default; UIP never set in this slice (no crystal model).
const DEFAULT_STATUS_A: u8 = 0x26;
/// Default Status B: 24-hour mode bit (DM/binary cleared → BCD; PIE/AIE/UIE off).
const DEFAULT_STATUS_B: u8 = 0x02;
/// Default Status C: no IRQ flags pending.
const DEFAULT_STATUS_C: u8 = 0x00;
/// Default Status D: VRT=1 (valid RAM and time / battery OK).
const DEFAULT_STATUS_D: u8 = 0x80;

/// Status A RS field mask (bits 3:0); 0 = periodic interrupt disabled.
const STATUS_A_RS_MASK: u8 = 0x0F;

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
        let idx = index & INDEX_MASK;
        if idx == REG_STATUS_C {
            return;
        }
        self.ram[idx as usize] = value;
        if idx == REG_STATUS_B {
            self.recompute_irqf();
        }
    }

    /// RTC IRQ pin level (MC146818 IRQ); true when status C IRQF is set.
    ///
    /// Spec: IRQ is asserted while IRQF=1; reading status C clears flags / pin.
    pub fn irq_line(&self) -> bool {
        self.ram[REG_STATUS_C as usize] & STC_IRQF != 0
    }

    /// Advance `periods` model periodic quanta (not host-real-time).
    ///
    /// Spec (MC146818): when RS≠0, each period sets PF; when SET=0 each period
    /// also sets UF (update-ended colocated with the quantum — honesty note:
    /// calendar fields are not advanced). AIE sets AF when alarm regs match time.
    /// IRQF = (PF∧PIE) ∨ (AF∧AIE) ∨ (UF∧UIE). Returns true on IRQ pin rising edge.
    pub fn tick(&mut self, periods: u64) -> bool {
        if periods == 0 {
            return false;
        }
        let prev = self.irq_line();
        let status_a = self.ram[REG_STATUS_A as usize];
        let status_b = self.ram[REG_STATUS_B as usize];
        let rs = status_a & STATUS_A_RS_MASK;
        let set_inhibits_update = status_b & STB_SET != 0;

        for _ in 0..periods {
            if rs != 0 {
                self.ram[REG_STATUS_C as usize] |= STC_PF;
            }
            if !set_inhibits_update {
                self.ram[REG_STATUS_C as usize] |= STC_UF;
            }
            self.maybe_set_alarm_flag();
        }
        self.recompute_irqf();
        !prev && self.irq_line()
    }

    fn maybe_set_alarm_flag(&mut self) {
        let b = self.ram[REG_STATUS_B as usize];
        if b & STB_AIE == 0 {
            return;
        }
        let sec = self.ram[REG_SEC as usize];
        let min = self.ram[REG_MIN as usize];
        let hour = self.ram[REG_HOUR as usize];
        let a_sec = self.ram[REG_SEC_ALARM as usize];
        let a_min = self.ram[REG_MIN_ALARM as usize];
        let a_hour = self.ram[REG_HOUR_ALARM as usize];
        // Spec: MC146818 alarm "don't care" when bit7 of an alarm register is set.
        let sec_ok = (a_sec & 0x80) != 0 || a_sec == sec;
        let min_ok = (a_min & 0x80) != 0 || a_min == min;
        let hour_ok = (a_hour & 0x80) != 0 || a_hour == hour;
        if sec_ok && min_ok && hour_ok {
            self.ram[REG_STATUS_C as usize] |= STC_AF;
        }
    }

    fn recompute_irqf(&mut self) {
        let b = self.ram[REG_STATUS_B as usize];
        let mut c = self.ram[REG_STATUS_C as usize] & (STC_PF | STC_AF | STC_UF);
        let irq = (c & STC_PF != 0 && b & STB_PIE != 0)
            || (c & STC_AF != 0 && b & STB_AIE != 0)
            || (c & STC_UF != 0 && b & STB_UIE != 0);
        if irq {
            c |= STC_IRQF;
        }
        self.ram[REG_STATUS_C as usize] = c;
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
                // Spec: MC146818 status C is read-to-clear (PF/AF/UF/IRQF).
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
                if idx == REG_STATUS_B {
                    self.recompute_irqf();
                }
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
        assert!(!c.irq_line());

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

    /// Spec: MC146818 status C read-to-clear; writes ignored.
    #[test]
    fn status_c_read_to_clear() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_STATUS_B, DEFAULT_STATUS_B | STB_PIE);
        assert!(c.tick(1));
        assert!(c.irq_line());
        let flags = c.read_reg(REG_STATUS_C);
        assert_ne!(flags & STC_PF, 0);
        assert_ne!(flags & STC_IRQF, 0);
        c.port_write(CMOS_INDEX, 1, u32::from(REG_STATUS_C));
        assert_eq!(c.port_read(CMOS_DATA, 1) as u8, flags);
        assert_eq!(c.read_reg(REG_STATUS_C), 0);
        assert!(!c.irq_line());
        // Writes to C ignored.
        c.port_write(CMOS_DATA, 1, 0xFF);
        assert_eq!(c.read_reg(REG_STATUS_C), 0);
    }

    /// Spec: MC146818 PIE + RS≠0 → PF/IRQF on tick; IRQ pin follows IRQF.
    #[test]
    fn pie_tick_asserts_irq_line() {
        let mut c = CmosRtc::new();
        c.port_write(CMOS_INDEX, 1, u32::from(REG_STATUS_B));
        c.port_write(CMOS_DATA, 1, u32::from(DEFAULT_STATUS_B | STB_PIE));
        assert!(!c.irq_line());
        assert!(c.tick(1));
        assert!(c.irq_line());
        assert_ne!(c.read_reg(REG_STATUS_C) & STC_PF, 0);
        assert_ne!(c.read_reg(REG_STATUS_C) & STC_IRQF, 0);
    }

    /// Spec: PF may set, but IRQF/IRQ require PIE.
    #[test]
    fn tick_without_pie_does_not_assert_irq() {
        let mut c = CmosRtc::new();
        assert!(!c.tick(1));
        assert!(!c.irq_line());
        // UF set (SET clear) but UIE off → no IRQF.
        assert_ne!(c.read_reg(REG_STATUS_C) & STC_UF, 0);
        assert_eq!(c.read_reg(REG_STATUS_C) & STC_IRQF, 0);
    }

    /// Spec: UIE + update-ended (UF) asserts IRQF.
    #[test]
    fn uie_tick_asserts_irq_line() {
        let mut c = CmosRtc::new();
        c.port_write(CMOS_INDEX, 1, u32::from(REG_STATUS_B));
        c.port_write(CMOS_DATA, 1, u32::from(DEFAULT_STATUS_B | STB_UIE));
        assert!(c.tick(1));
        assert!(c.irq_line());
        assert_ne!(c.read_reg(REG_STATUS_C) & STC_UF, 0);
    }

    /// Spec: AIE + matching alarm registers → AF/IRQF.
    #[test]
    fn aie_matching_alarm_asserts_irq() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_SEC, 0x30);
        c.write_reg(REG_MIN, 0x15);
        c.write_reg(REG_HOUR, 0x10);
        c.write_reg(REG_SEC_ALARM, 0x30);
        c.write_reg(REG_MIN_ALARM, 0x15);
        c.write_reg(REG_HOUR_ALARM, 0x10);
        c.port_write(CMOS_INDEX, 1, u32::from(REG_STATUS_B));
        c.port_write(CMOS_DATA, 1, u32::from(DEFAULT_STATUS_B | STB_AIE));
        assert!(c.tick(1));
        assert!(c.irq_line());
        assert_ne!(c.read_reg(REG_STATUS_C) & STC_AF, 0);
    }

    /// Spec: RS=0 disables periodic (PF not set from rate).
    #[test]
    fn rs_zero_skips_periodic_flag() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_STATUS_A, DEFAULT_STATUS_A & !STATUS_A_RS_MASK); // RS=0
        c.write_reg(REG_STATUS_B, DEFAULT_STATUS_B | STB_PIE);
        assert!(!c.tick(1));
        assert_eq!(c.read_reg(REG_STATUS_C) & STC_PF, 0);
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
