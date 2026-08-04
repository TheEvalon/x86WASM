//! Dual Intel 8259A Programmable Interrupt Controllers — ICW + OCW runtime.
//!
//! Classic PC ports: master `0x20`/`0x21`, slave `0xA0`/`0xA1`, cascade on IRQ2.
//!
//! # Spec refs
//!
//! - Intel 8259A Programmable Interrupt Controller datasheet — ICW1–ICW4;
//!   OCW1 (IMR); OCW2 non-specific / specific EOI; OCW3 IRR/ISR read select;
//!   OCW3 poll command (`P=1`); IRR/ISR; fully nested priority; cascade EOI
//!   (master + slave).
//! - Classic IBM PC/AT: master at `0x20`/`0x21`, slave at `0xA0`/`0xA1`, slave
//!   cascaded on master IR2.
//! - `docs/machine-model-pc-v1.md`, `plan.md` §15.3 / §21 / §23.3.
//!
//! # Scope (this slice)
//!
//! - ICW1–ICW4 initialization (single and cascaded)
//! - OCW1 IMR read/write on data port after init
//! - OCW2 non-specific EOI (`R=0,SL=0,EOI=1`) and specific EOI (`R=0,SL=1,EOI=1`)
//! - OCW3 read-register select (`RR`/`RIS`) for IRR/ISR on command-port reads
//! - OCW3 poll command (`P=1`): one-shot acknowledging command-port read
//!   returning `0x80 | level`, including software-sequenced cascaded polling
//! - Edge-triggered IR line assert, IRR→ISR on acknowledge, vector selection
//! - `DualPic::acknowledge` / `poll_irq` for `MachineBus::poll_external_irq`
//!
//! # Unsupported (explicit)
//!
//! - Auto-EOI (ICW4.AEOI), rotate modes (OCW2 `R=1`), special mask mode,
//!   special fully-nested mode
//! - Level-triggered delivery beyond storing ICW1.LTIM (runtime uses edge model)
//! - PIT IRQ0 / CMOS IRQ8 / device→PIC wiring (callers use `set_irq_line`)

use crate::PortDevice;

/// Master PIC command / data ports (classic PC).
pub const PIC_MASTER_CMD: u16 = 0x20;
pub const PIC_MASTER_DATA: u16 = 0x21;
/// Slave PIC command / data ports (classic PC).
pub const PIC_SLAVE_CMD: u16 = 0xA0;
pub const PIC_SLAVE_DATA: u16 = 0xA1;

/// ICW1 bit4 must be set (datasheet: identifies ICW1 vs OCW2/OCW3).
const ICW1_D4: u8 = 1 << 4;
/// ICW1 bit0: ICW4 will follow.
const ICW1_IC4: u8 = 1 << 0;
/// ICW1 bit1: single mode (no ICW3); clear = cascade (ICW3 follows).
const ICW1_SNGL: u8 = 1 << 1;
/// ICW1 bit3: level-triggered (1) vs edge-triggered (0).
const ICW1_LTIM: u8 = 1 << 3;
/// ICW4 bit0: 8086/8088 mode (µPM).
const ICW4_UPM: u8 = 1 << 0;

/// OCW2/OCW3: D4 must be 0 (else ICW1).
/// OCW2: D3=0; OCW3: D3=1 (datasheet Operation Command Word format).
const OCW_D3: u8 = 1 << 3;
/// OCW2 bits.
const OCW2_EOI: u8 = 1 << 5;
const OCW2_SL: u8 = 1 << 6;
const OCW2_R: u8 = 1 << 7;
/// OCW3 bits.
const OCW3_RIS: u8 = 1 << 0;
const OCW3_RR: u8 = 1 << 1;
const OCW3_P: u8 = 1 << 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InitPhase {
    /// Not in an ICW sequence (ready for OCW or a new ICW1).
    Idle,
    ExpectIcw2,
    ExpectIcw3,
    ExpectIcw4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReadReg {
    /// Default after init (Intel 8259A: IRR selected until OCW3 changes it).
    Irr,
    Isr,
}

/// One 8259A controller (master or slave role for ICW3 interpretation).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pic8259 {
    /// `true` = master (ICW3 = slave IR bitmask); `false` = slave (ICW3 ID).
    pub is_master: bool,
    /// Set only after the full expected ICW sequence completes.
    pub initialized: bool,
    phase: InitPhase,
    /// ICW1.IC4 — expect ICW4.
    pub expect_icw4: bool,
    /// ICW1.SNGL — single (no cascade / no ICW3).
    pub single: bool,
    /// ICW1.LTIM — level-triggered when true (stored; runtime uses edge model).
    pub level_triggered: bool,
    /// Raw ICW1 byte from the last started sequence (0 after reset).
    pub icw1: u8,
    /// ICW2 vector base (8086: bits 7:3 select base; IRQ n → base|n).
    pub vector_base: u8,
    /// ICW3: master = slave connection bitmask; slave = cascade ID (bits 2:0).
    pub icw3: u8,
    /// Raw ICW4 byte (0 if ICW4 was skipped or not yet received).
    pub icw4: u8,
    /// ICW4.µPM — 8086/8088 mode.
    pub mode_8086: bool,
    /// Interrupt Mask Register (OCW1). Bit=1 masks that IR. Reset: all masked.
    pub imr: u8,
    /// Interrupt Request Register.
    pub irr: u8,
    /// In-Service Register.
    pub isr: u8,
    /// Latched IR line levels (bit N = IRN high) for edge detection.
    ir_level: u8,
    /// OCW3 read-register selection for command-port reads.
    read_reg: ReadReg,
    /// OCW3 `P=1` armed: the next command-port read is an interrupt acknowledge.
    ///
    /// This is the 8259A software *poll command*, not to be confused with
    /// [`DualPic::poll_irq`], which samples an INTA vector for the machine bus.
    poll_command_armed: bool,
}

impl Pic8259 {
    pub fn new_master() -> Self {
        Self::reset_role(true)
    }

    pub fn new_slave() -> Self {
        Self::reset_role(false)
    }

    fn reset_role(is_master: bool) -> Self {
        Self {
            is_master,
            initialized: false,
            phase: InitPhase::Idle,
            expect_icw4: false,
            single: false,
            level_triggered: false,
            icw1: 0,
            vector_base: 0,
            icw3: 0,
            icw4: 0,
            mode_8086: false,
            imr: 0xFF,
            irr: 0,
            isr: 0,
            ir_level: 0,
            read_reg: ReadReg::Irr,
            poll_command_armed: false,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::reset_role(self.is_master);
    }

    /// Master IR lines that have a slave attached (ICW3 bitmask). Meaningful when
    /// `is_master && initialized && !single`.
    pub fn slave_ir_mask(&self) -> u8 {
        if self.is_master {
            self.icw3
        } else {
            0
        }
    }

    /// Slave cascade identity (ICW3 bits 2:0). Meaningful when
    /// `!is_master && initialized && !single`.
    pub fn slave_id(&self) -> u8 {
        if self.is_master {
            0
        } else {
            self.icw3 & 0x07
        }
    }

    /// Vector for IRQ line `irq` (0–7) after init: `(vector_base & 0xF8) | irq`.
    pub fn irq_vector(&self, irq: u8) -> Option<u8> {
        if !self.initialized || irq > 7 {
            return None;
        }
        Some((self.vector_base & 0xF8) | (irq & 0x07))
    }

    /// Drive IR`irq` (0–7). Edge-triggered: low→high sets IRR (Intel 8259A).
    pub fn set_irq_line(&mut self, irq: u8, high: bool) {
        if irq > 7 {
            return;
        }
        let bit = 1u8 << irq;
        let was_high = self.ir_level & bit != 0;
        if high {
            self.ir_level |= bit;
            if !was_high {
                // Edge sense: rising edge latches IRR (datasheet ICW1 / edge mode).
                self.irr |= bit;
            }
        } else {
            self.ir_level &= !bit;
        }
    }

    /// Highest-priority unmasked IRR request not blocked by fully-nested ISR.
    /// Spec: Intel 8259A fully nested mode — IR0 highest … IR7 lowest.
    fn highest_priority_request(&self) -> Option<u8> {
        if !self.initialized {
            return None;
        }
        let limit = if self.isr == 0 {
            8u8
        } else {
            // Only IR lines strictly higher priority than the top ISR bit.
            self.isr.trailing_zeros() as u8
        };
        for ir in 0..limit {
            let bit = 1u8 << ir;
            if self.irr & bit != 0 && self.imr & bit == 0 {
                return Some(ir);
            }
        }
        None
    }

    /// True if this chip would assert INT (unmasked request not nested-blocked).
    pub fn int_pending(&self) -> bool {
        self.highest_priority_request().is_some()
    }

    fn ack_ir(&mut self, ir: u8) {
        let bit = 1u8 << ir;
        self.irr &= !bit;
        self.isr |= bit;
    }

    fn write_cmd(&mut self, value: u8) {
        if value & ICW1_D4 != 0 {
            self.begin_icw1(value);
            return;
        }
        // OCW only after init sequence is idle (incomplete ICW ignores OCW).
        if self.phase != InitPhase::Idle {
            return;
        }
        if value & OCW_D3 == 0 {
            self.write_ocw2(value);
        } else {
            self.write_ocw3(value);
        }
    }

    fn begin_icw1(&mut self, value: u8) {
        self.initialized = false;
        self.icw1 = value;
        self.expect_icw4 = value & ICW1_IC4 != 0;
        self.single = value & ICW1_SNGL != 0;
        self.level_triggered = value & ICW1_LTIM != 0;
        self.vector_base = 0;
        self.icw3 = 0;
        self.icw4 = 0;
        self.mode_8086 = false;
        // Datasheet ICW1: edge sense circuit is reset; clear request state.
        self.irr = 0;
        self.isr = 0;
        self.ir_level = 0;
        self.read_reg = ReadReg::Irr;
        self.poll_command_armed = false;
        self.phase = InitPhase::ExpectIcw2;
    }

    fn write_data(&mut self, value: u8) {
        match self.phase {
            InitPhase::Idle => {
                // OCW1 — IMR (Intel 8259A OCW1).
                self.imr = value;
            }
            InitPhase::ExpectIcw2 => {
                self.vector_base = value;
                if !self.single {
                    self.phase = InitPhase::ExpectIcw3;
                } else if self.expect_icw4 {
                    self.phase = InitPhase::ExpectIcw4;
                } else {
                    self.finish_init();
                }
            }
            InitPhase::ExpectIcw3 => {
                self.icw3 = value;
                if self.expect_icw4 {
                    self.phase = InitPhase::ExpectIcw4;
                } else {
                    self.finish_init();
                }
            }
            InitPhase::ExpectIcw4 => {
                self.icw4 = value;
                self.mode_8086 = value & ICW4_UPM != 0;
                self.finish_init();
            }
        }
    }

    fn finish_init(&mut self) {
        self.phase = InitPhase::Idle;
        self.initialized = true;
        self.read_reg = ReadReg::Irr;
    }

    /// OCW2: non-specific / specific EOI only (`R=0`). Rotate unsupported.
    fn write_ocw2(&mut self, value: u8) {
        if value & OCW2_R != 0 {
            // Rotate / set-priority forms — unsupported this slice.
            return;
        }
        if value & OCW2_EOI == 0 {
            return;
        }
        if value & OCW2_SL != 0 {
            // Specific EOI: clear ISR bit L2–L0.
            let level = value & 0x07;
            self.isr &= !(1u8 << level);
        } else {
            // Non-specific EOI: clear highest-priority (lowest index) ISR bit.
            if self.isr != 0 {
                let level = self.isr.trailing_zeros() as u8;
                self.isr &= !(1u8 << level);
            }
        }
    }

    /// OCW3: poll command (`P=1`) and IRR/ISR read select. Special-mask
    /// (`ESMM`/`SMM`) unsupported.
    ///
    /// Spec: Intel 8259A datasheet — OCW3 format / Poll Command. Model choice:
    /// `P=1` arms the poll for the next command-port read and leaves the
    /// `RR`/`RIS` selection untouched (the datasheet does not define combining
    /// a poll with a read-register select).
    fn write_ocw3(&mut self, value: u8) {
        if value & OCW3_P != 0 {
            self.poll_command_armed = true;
            return;
        }
        if value & OCW3_RR != 0 {
            self.read_reg = if value & OCW3_RIS != 0 {
                ReadReg::Isr
            } else {
                ReadReg::Irr
            };
        }
    }

    /// Consume an armed OCW3 poll command: acknowledge the highest-priority
    /// unmasked request like an INTA cycle and return the poll byte.
    ///
    /// Spec: Intel 8259A datasheet — Poll Command: bit7 = 1 when an interrupt is
    /// pending, bits 2:0 = binary level of that request; the IS bit is set and
    /// the IR bit cleared (edge model) via the shared [`Pic8259::ack_ir`] path.
    /// Model choice: with nothing pending the byte is `0x00` (bit7 clear, level
    /// bits zero) — the datasheet leaves the level bits unspecified there.
    fn take_poll_command_byte(&mut self) -> u8 {
        self.poll_command_armed = false;
        match self.highest_priority_request() {
            Some(ir) => {
                self.ack_ir(ir);
                0x80 | ir
            }
            None => 0x00,
        }
    }

    fn read_cmd(&self) -> u8 {
        match self.read_reg {
            ReadReg::Irr => self.irr,
            ReadReg::Isr => self.isr,
        }
    }

    fn read_data(&self) -> u8 {
        self.imr
    }
}

/// Dual 8259A as used on the classic PC (master + slave).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DualPic {
    pub master: Pic8259,
    pub slave: Pic8259,
}

impl DualPic {
    pub fn new() -> Self {
        Self {
            master: Pic8259::new_master(),
            slave: Pic8259::new_slave(),
        }
    }

    pub fn reset(&mut self) {
        self.master.reset();
        self.slave.reset();
    }

    /// Assert/deassert a global ISA IRQ line (0–15). IRQ8–15 → slave IR0–7.
    pub fn set_irq_line(&mut self, irq: u8, high: bool) {
        if irq < 8 {
            self.master.set_irq_line(irq, high);
        } else if irq < 16 {
            self.slave.set_irq_line(irq - 8, high);
            self.sync_cascade();
        }
    }

    /// Drive master's cascade IR from slave INT (PC AT: slave on IR2).
    ///
    /// Spec: Intel 8259A cascade — slave INT feeds the master's cascaded IR.
    fn sync_cascade(&mut self) {
        if !self.master.initialized || self.master.single {
            return;
        }
        if !self.slave.initialized || self.slave.single {
            return;
        }
        let cascade_ir = self.slave.slave_id();
        if self.master.slave_ir_mask() & (1 << cascade_ir) == 0 {
            return;
        }
        // Slave INT high when slave has a deliverable request (fully nested).
        let slave_int = self.slave.int_pending();
        self.master.set_irq_line(cascade_ir, slave_int);
    }

    /// INTA-style acknowledge: move IRR→ISR and return 8086 vector, or `None`.
    ///
    /// Spec: Intel 8259A interrupt sequence / cascade — slave vector when master
    /// selects a cascaded IR; EOI must later clear both slave and master ISR bits.
    pub fn acknowledge(&mut self) -> Option<u8> {
        self.sync_cascade();
        let ir = self.master.highest_priority_request()?;
        let bit = 1u8 << ir;
        if !self.master.single && (self.master.slave_ir_mask() & bit) != 0 {
            let slave_ir = self.slave.highest_priority_request()?;
            self.slave.ack_ir(slave_ir);
            self.master.ack_ir(ir);
            let vec = self.slave.irq_vector(slave_ir);
            self.sync_cascade();
            return vec;
        }
        self.master.ack_ir(ir);
        self.master.irq_vector(ir)
    }

    /// Vector for `Bus::poll_external_irq` (acknowledge on poll).
    ///
    /// Note: this is external IRQ sampling for the machine bus, *not* the 8259A
    /// OCW3 poll command (see [`Pic8259::take_poll_command_byte`]).
    pub fn poll_irq(&mut self) -> Option<u8> {
        self.acknowledge()
    }

    /// Command-port read while an OCW3 poll command is armed (`P=1`).
    ///
    /// Spec: Intel 8259A datasheet — Poll Command: the read is an interrupt
    /// acknowledge for the addressed chip only. Cascaded polling is therefore
    /// software sequenced: poll the master (returns the cascade level, IR2 on the
    /// PC AT), then poll the slave's own command port for the slave level.
    fn poll_command_read(&mut self, slave: bool) -> u8 {
        self.sync_cascade();
        let byte = if slave {
            self.slave.take_poll_command_byte()
        } else {
            self.master.take_poll_command_byte()
        };
        // Slave INT may drop once the acknowledge moved its request IRR→ISR.
        self.sync_cascade();
        byte
    }

    fn chip_mut(&mut self, port: u16) -> Option<(&mut Pic8259, bool)> {
        match port {
            PIC_MASTER_CMD => Some((&mut self.master, false)),
            PIC_MASTER_DATA => Some((&mut self.master, true)),
            PIC_SLAVE_CMD => Some((&mut self.slave, false)),
            PIC_SLAVE_DATA => Some((&mut self.slave, true)),
            _ => None,
        }
    }

    fn chip(&self, port: u16) -> Option<(&Pic8259, bool)> {
        match port {
            PIC_MASTER_CMD => Some((&self.master, false)),
            PIC_MASTER_DATA => Some((&self.master, true)),
            PIC_SLAVE_CMD => Some((&self.slave, false)),
            PIC_SLAVE_DATA => Some((&self.slave, true)),
            _ => None,
        }
    }
}

impl Default for DualPic {
    fn default() -> Self {
        Self::new()
    }
}

impl PortDevice for DualPic {
    fn port_read(&mut self, port: u16, _size: u8) -> u32 {
        let Some((chip, is_data)) = self.chip(port) else {
            return 0xFFFF_FFFF;
        };
        if is_data {
            return u32::from(chip.read_data());
        }
        if !chip.poll_command_armed {
            return u32::from(chip.read_cmd());
        }
        u32::from(self.poll_command_read(port == PIC_SLAVE_CMD))
    }

    fn port_write(&mut self, port: u16, _size: u8, value: u32) {
        let Some((chip, is_data)) = self.chip_mut(port) else {
            return;
        };
        let v = value as u8;
        if is_data {
            chip.write_data(v);
        } else {
            chip.write_cmd(v);
        }
        // Cascade line may change after OCW1 unmask / EOI on slave.
        self.sync_cascade();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_at_cascade(pic: &mut DualPic) {
        // Master: ICW1=0x11, ICW2=0x08, ICW3=0x04, ICW4=0x01
        pic.port_write(PIC_MASTER_CMD, 1, 0x11);
        pic.port_write(PIC_MASTER_DATA, 1, 0x08);
        pic.port_write(PIC_MASTER_DATA, 1, 0x04);
        pic.port_write(PIC_MASTER_DATA, 1, 0x01);
        // Slave: ICW1=0x11, ICW2=0x70, ICW3=0x02, ICW4=0x01
        pic.port_write(PIC_SLAVE_CMD, 1, 0x11);
        pic.port_write(PIC_SLAVE_DATA, 1, 0x70);
        pic.port_write(PIC_SLAVE_DATA, 1, 0x02);
        pic.port_write(PIC_SLAVE_DATA, 1, 0x01);
    }

    /// Spec: reset leaves both PICs uninitialized (Intel 8259A; PC cold start).
    #[test]
    fn reset_state_uninitialized() {
        let pic = DualPic::new();
        assert!(!pic.master.initialized);
        assert!(!pic.slave.initialized);
        assert_eq!(pic.master.phase, InitPhase::Idle);
        assert_eq!(pic.slave.phase, InitPhase::Idle);
        assert_eq!(pic.master.vector_base, 0);
        assert_eq!(pic.slave.vector_base, 0);
        assert_eq!(pic.master.icw3, 0);
        assert_eq!(pic.slave.icw3, 0);
        assert!(!pic.master.mode_8086);
        assert!(!pic.slave.mode_8086);
        assert!(pic.master.is_master);
        assert!(!pic.slave.is_master);
        assert_eq!(pic.master.imr, 0xFF);
        assert_eq!(pic.master.irr, 0);
        assert_eq!(pic.master.isr, 0);

        let mut pic2 = DualPic::new();
        pic2.port_write(PIC_MASTER_CMD, 1, 0x11);
        pic2.reset();
        assert_eq!(pic2, DualPic::new());
    }

    /// Spec: ICW1 SNGL=1, IC4=1 → ICW2 then ICW4 (no ICW3). Intel 8259A datasheet.
    #[test]
    fn single_mode_icw_sequence() {
        let mut pic = DualPic::new();
        pic.port_write(PIC_MASTER_CMD, 1, 0x13);
        assert!(!pic.master.initialized);
        pic.port_write(PIC_MASTER_DATA, 1, 0x08);
        assert!(!pic.master.initialized);
        pic.port_write(PIC_MASTER_DATA, 1, 0x01);
        assert!(pic.master.initialized);
        assert!(pic.master.single);
        assert!(pic.master.expect_icw4);
        assert!(pic.master.mode_8086);
        assert_eq!(pic.master.vector_base, 0x08);
        assert_eq!(pic.master.icw3, 0);
        assert_eq!(pic.master.irq_vector(0), Some(0x08));
        assert_eq!(pic.master.irq_vector(7), Some(0x0F));
        assert!(!pic.slave.initialized);
    }

    /// Spec: cascaded PC AT init — master ICW3=0x04 (slave on IR2), slave ID=2.
    #[test]
    fn cascaded_master_slave_with_vector_offsets() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        assert!(pic.master.initialized);
        assert!(!pic.master.single);
        assert_eq!(pic.master.vector_base, 0x08);
        assert_eq!(pic.master.slave_ir_mask(), 0x04);
        assert!(pic.master.mode_8086);
        assert_eq!(pic.master.irq_vector(0), Some(0x08));
        assert_eq!(pic.master.irq_vector(2), Some(0x0A));
        assert!(pic.slave.initialized);
        assert_eq!(pic.slave.vector_base, 0x70);
        assert_eq!(pic.slave.slave_id(), 2);
        assert!(pic.slave.mode_8086);
        assert_eq!(pic.slave.irq_vector(0), Some(0x70));
        assert_eq!(pic.slave.irq_vector(1), Some(0x71));
    }

    /// Spec: ICW2 bits 2:0 ignored for 8086 vector numbering (base aligned to 8).
    #[test]
    fn vector_base_masks_low_bits() {
        let mut chip = Pic8259::new_master();
        chip.write_cmd(0x13);
        chip.write_data(0x28 | 0x03);
        chip.write_data(0x01);
        assert!(chip.initialized);
        assert_eq!(chip.irq_vector(0), Some(0x28));
        assert_eq!(chip.irq_vector(5), Some(0x2D));
    }

    /// Incomplete init stays uninitialized; OCW during ICW must not complete it.
    #[test]
    fn invalid_incomplete_init_stays_uninitialized() {
        let mut pic = DualPic::new();
        pic.port_write(PIC_MASTER_CMD, 1, 0x11);
        pic.port_write(PIC_MASTER_DATA, 1, 0x20);
        assert!(!pic.master.initialized);
        assert_eq!(pic.master.phase, InitPhase::ExpectIcw3);

        // OCW2-style write on command port (D4=0) must not finish init.
        pic.port_write(PIC_MASTER_CMD, 1, 0x20);
        assert!(!pic.master.initialized);
        assert_eq!(pic.master.phase, InitPhase::ExpectIcw3);

        let mut pic2 = DualPic::new();
        pic2.port_write(PIC_MASTER_DATA, 1, 0xFF);
        assert!(!pic2.master.initialized);
        assert_eq!(pic2.master.phase, InitPhase::Idle);
        // Before init, OCW1 still updates IMR (Idle phase).
        assert_eq!(pic2.master.imr, 0xFF);

        pic.port_write(PIC_MASTER_CMD, 1, 0x13);
        assert!(!pic.master.initialized);
        assert_eq!(pic.master.vector_base, 0);
        assert_eq!(pic.master.phase, InitPhase::ExpectIcw2);
    }

    /// Master ICW3 bitmask and slave ICW3 ID stored for cascade IRQ2 wiring.
    #[test]
    fn master_slave_icw3_wiring() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        assert_eq!(pic.master.slave_ir_mask(), 1 << 2);
        assert_eq!(pic.slave.slave_id(), 2);
        assert_ne!(pic.master.slave_ir_mask() & (1 << pic.slave.slave_id()), 0);
    }

    /// Clone / PartialEq round-trip of architectural state.
    #[test]
    fn state_clone_equality_round_trip() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        pic.port_write(PIC_MASTER_DATA, 1, 0x00); // OCW1 unmask all

        let cloned = pic.clone();
        assert_eq!(pic, cloned);
        assert!(cloned.master.initialized);
        assert!(cloned.slave.initialized);
        assert_eq!(cloned.master.vector_base, 0x08);
        assert_eq!(cloned.slave.vector_base, 0x70);
        assert_eq!(cloned.master.imr, 0x00);
    }

    #[test]
    fn port_decode_ignores_unrelated_ports() {
        let mut pic = DualPic::new();
        pic.port_write(0x3F8, 1, 0x11);
        assert!(!pic.master.initialized);
        assert_eq!(pic.port_read(0x3F8, 1), 0xFFFF_FFFF);
    }

    /// Cascade without ICW4 (IC4=0) completes after ICW3.
    #[test]
    fn cascade_without_icw4_completes_after_icw3() {
        let mut chip = Pic8259::new_master();
        chip.write_cmd(0x10);
        chip.write_data(0x08);
        assert!(!chip.initialized);
        chip.write_data(0x04);
        assert!(chip.initialized);
        assert!(!chip.expect_icw4);
        assert!(!chip.mode_8086);
        assert_eq!(chip.icw4, 0);
    }

    /// Spec: OCW1 programs IMR; data-port read returns IMR (Intel 8259A OCW1).
    #[test]
    fn ocw1_imr_read_write() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        assert_eq!(pic.port_read(PIC_MASTER_DATA, 1), 0xFF);
        pic.port_write(PIC_MASTER_DATA, 1, 0xFE); // unmask IR0
        assert_eq!(pic.master.imr, 0xFE);
        assert_eq!(pic.port_read(PIC_MASTER_DATA, 1), 0xFE);
    }

    /// Spec: OCW3 RR/RIS selects IRR vs ISR on next command-port reads.
    #[test]
    fn ocw3_irr_isr_read_select() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        pic.port_write(PIC_MASTER_DATA, 1, 0x00);
        pic.set_irq_line(0, true);
        assert_eq!(pic.master.irr, 0x01);
        // Default after ICW: IRR.
        assert_eq!(pic.port_read(PIC_MASTER_CMD, 1), 0x01);
        // OCW3: RR=1, RIS=1 → ISR (0x0B)
        pic.port_write(PIC_MASTER_CMD, 1, 0x0B);
        assert_eq!(pic.port_read(PIC_MASTER_CMD, 1), 0x00);
        let v = pic.poll_irq().unwrap();
        assert_eq!(v, 0x08);
        assert_eq!(pic.master.isr, 0x01);
        assert_eq!(pic.port_read(PIC_MASTER_CMD, 1), 0x01);
        // OCW3: RR=1, RIS=0 → IRR (0x0A)
        pic.port_write(PIC_MASTER_CMD, 1, 0x0A);
        assert_eq!(pic.port_read(PIC_MASTER_CMD, 1), 0x00);
    }

    /// Spec: edge assert + unmask → acknowledge returns vector; nonspecific EOI clears ISR.
    #[test]
    fn irq0_acknowledge_and_nonspecific_eoi() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        pic.port_write(PIC_MASTER_DATA, 1, 0xFE); // unmask IR0 only
        pic.set_irq_line(0, true);
        assert_eq!(pic.poll_irq(), Some(0x08));
        assert_eq!(pic.master.irr, 0);
        assert_eq!(pic.master.isr, 0x01);
        // Second poll: still in service, no new edge → None
        assert_eq!(pic.poll_irq(), None);
        // Non-specific EOI (OCW2 0x20)
        pic.port_write(PIC_MASTER_CMD, 1, 0x20);
        assert_eq!(pic.master.isr, 0);
        // Need a new edge for another delivery
        pic.set_irq_line(0, false);
        pic.set_irq_line(0, true);
        assert_eq!(pic.poll_irq(), Some(0x08));
    }

    /// Spec: specific EOI clears the named ISR bit (OCW2 EOI=1, SL=1).
    #[test]
    fn specific_eoi_clears_named_isr_bit() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        pic.port_write(PIC_MASTER_DATA, 1, 0x00);
        pic.set_irq_line(3, true);
        assert_eq!(pic.poll_irq(), Some(0x0B));
        assert_eq!(pic.master.isr, 1 << 3);
        pic.port_write(PIC_MASTER_CMD, 1, 0x60 | 3); // specific EOI IR3
        assert_eq!(pic.master.isr, 0);
    }

    /// Spec: masked IR does not deliver (OCW1 / IMR).
    #[test]
    fn masked_irq_does_not_deliver() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        // IMR still 0xFF
        pic.set_irq_line(0, true);
        assert_eq!(pic.master.irr, 0x01);
        assert_eq!(pic.poll_irq(), None);
    }

    /// Spec: cascade — slave IRQ9 (IR1) → vector base|1; EOI slave then master.
    #[test]
    fn cascade_slave_irq_vector_and_dual_eoi() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        pic.port_write(PIC_MASTER_DATA, 1, 0xFB); // unmask master IR2 (cascade)
        pic.port_write(PIC_SLAVE_DATA, 1, 0xFD); // unmask slave IR1
        pic.set_irq_line(9, true); // slave IR1
        assert_eq!(pic.poll_irq(), Some(0x71));
        assert_eq!(pic.slave.isr, 1 << 1);
        assert_eq!(pic.master.isr, 1 << 2);
        // Non-specific EOI slave then master (PC AT convention).
        pic.port_write(PIC_SLAVE_CMD, 1, 0x20);
        pic.port_write(PIC_MASTER_CMD, 1, 0x20);
        assert_eq!(pic.slave.isr, 0);
        assert_eq!(pic.master.isr, 0);
    }

    /// Spec: Intel 8259A datasheet — Poll Command (OCW3 `P=1`): the next
    /// command-port read is an interrupt acknowledge; it sets the IS bit of the
    /// highest-priority unmasked request, clears its IR bit (edge mode), and
    /// returns bit7=1 with the binary level in bits 2:0.
    #[test]
    fn ocw3_poll_command_acknowledges_pending_request() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        pic.port_write(PIC_MASTER_DATA, 1, 0x00); // OCW1 unmask all
        pic.set_irq_line(3, true);
        assert_eq!(pic.master.irr, 1 << 3);

        pic.port_write(PIC_MASTER_CMD, 1, 0x0C); // OCW3 D3=1, P=1
        assert_eq!(pic.port_read(PIC_MASTER_CMD, 1), 0x80 | 3);
        assert_eq!(pic.master.isr, 1 << 3);
        assert_eq!(pic.master.irr & (1 << 3), 0);
    }

    /// Spec: Intel 8259A datasheet — Poll Command: bit7 = 0 when no interrupt is
    /// pending. Documented model choice: the datasheet leaves bits 2:0
    /// unspecified in that case; this model returns `0x00`.
    #[test]
    fn ocw3_poll_command_without_pending_request() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        pic.port_write(PIC_MASTER_DATA, 1, 0x00); // OCW1 unmask all
        pic.port_write(PIC_MASTER_CMD, 1, 0x0C);
        assert_eq!(pic.port_read(PIC_MASTER_CMD, 1), 0x00);
        assert_eq!(pic.master.isr, 0);

        // Fully masked request: IRR is non-zero but nothing is pending, so the
        // poll byte is 0x00 rather than the IRR contents.
        pic.port_write(PIC_MASTER_DATA, 1, 0xFF);
        pic.set_irq_line(3, true);
        assert_eq!(pic.master.irr, 1 << 3);
        pic.port_write(PIC_MASTER_CMD, 1, 0x0C);
        assert_eq!(pic.port_read(PIC_MASTER_CMD, 1), 0x00);
        assert_eq!(pic.master.isr, 0);
        assert_eq!(pic.master.irr, 1 << 3);
    }

    /// Spec: Intel 8259A datasheet — Poll Command resolves the *highest*
    /// priority request (fully nested: IR0 highest … IR7 lowest).
    #[test]
    fn ocw3_poll_command_returns_highest_priority_level() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        pic.port_write(PIC_MASTER_DATA, 1, 0x00); // OCW1 unmask all
        pic.set_irq_line(6, true);
        pic.set_irq_line(3, true);

        pic.port_write(PIC_MASTER_CMD, 1, 0x0C);
        assert_eq!(pic.port_read(PIC_MASTER_CMD, 1), 0x80 | 3);
        assert_eq!(pic.master.isr, 1 << 3);
        assert_eq!(pic.master.irr, 1 << 6);
    }

    /// Spec: Intel 8259A datasheet — Poll Command / OCW1: a masked IR is not a
    /// pending request, so the poll returns the unmasked lower-priority level.
    #[test]
    fn ocw3_poll_command_respects_imr() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        pic.port_write(PIC_MASTER_DATA, 1, 0xBF); // unmask IR6 only
        pic.set_irq_line(3, true);
        pic.set_irq_line(6, true);

        pic.port_write(PIC_MASTER_CMD, 1, 0x0C);
        assert_eq!(pic.port_read(PIC_MASTER_CMD, 1), 0x80 | 6);
        assert_eq!(pic.master.isr, 1 << 6);
        assert_eq!(pic.master.irr, 1 << 3);
    }

    /// Spec: Intel 8259A datasheet — Poll Command applies to the *next* read
    /// only; later command-port reads use the OCW3 `RR`/`RIS` selection, and a
    /// fresh OCW3 `P=1` re-arms the poll.
    #[test]
    fn ocw3_poll_command_is_one_shot_and_rearmable() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        pic.port_write(PIC_MASTER_DATA, 1, 0x00); // OCW1 unmask all
        pic.port_write(PIC_MASTER_CMD, 1, 0x0B); // OCW3 RR=1, RIS=1 → ISR
        pic.set_irq_line(6, true);

        pic.port_write(PIC_MASTER_CMD, 1, 0x0C); // OCW3 P=1
        assert_eq!(pic.port_read(PIC_MASTER_CMD, 1), 0x80 | 6);
        // One-shot: reads revert to the previously selected register (ISR).
        assert_eq!(pic.port_read(PIC_MASTER_CMD, 1), 1 << 6);
        assert_eq!(pic.port_read(PIC_MASTER_CMD, 1), 1 << 6);
        // RR/RIS select still works after a poll.
        pic.port_write(PIC_MASTER_CMD, 1, 0x0A); // OCW3 RR=1, RIS=0 → IRR
        assert_eq!(pic.port_read(PIC_MASTER_CMD, 1), 0x00);

        pic.port_write(PIC_MASTER_CMD, 1, 0x20); // non-specific EOI
        pic.set_irq_line(1, true);
        assert_eq!(pic.port_read(PIC_MASTER_CMD, 1), 1 << 1); // IRR select
        pic.port_write(PIC_MASTER_CMD, 1, 0x0C); // re-arm poll
        assert_eq!(pic.port_read(PIC_MASTER_CMD, 1), 0x80 | 1);
        assert_eq!(pic.master.isr, 1 << 1);
    }

    /// Spec: Intel 8259A datasheet — Poll Command with cascade: polling is
    /// software sequenced. The master poll returns the cascade level (IR2 on the
    /// PC AT) and sets master IS2; polling the slave's own command port then
    /// returns the slave level and sets the slave IS bit.
    #[test]
    fn ocw3_poll_command_cascaded_master_then_slave() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        pic.port_write(PIC_MASTER_DATA, 1, 0xFB); // unmask master IR2 (cascade)
        pic.port_write(PIC_SLAVE_DATA, 1, 0xFD); // unmask slave IR1
        pic.set_irq_line(9, true); // slave IR1

        pic.port_write(PIC_MASTER_CMD, 1, 0x0C);
        assert_eq!(pic.port_read(PIC_MASTER_CMD, 1), 0x80 | 2);
        assert_eq!(pic.master.isr, 1 << 2);

        pic.port_write(PIC_SLAVE_CMD, 1, 0x0C);
        assert_eq!(pic.port_read(PIC_SLAVE_CMD, 1), 0x80 | 1);
        assert_eq!(pic.slave.isr, 1 << 1);
        assert_eq!(pic.slave.irr, 0);
    }

    /// Spec: fully nested — in-service IR0 blocks lower-priority IR1 until EOI.
    #[test]
    fn fully_nested_blocks_lower_priority() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        pic.port_write(PIC_MASTER_DATA, 1, 0x00);
        pic.set_irq_line(0, true);
        assert_eq!(pic.poll_irq(), Some(0x08));
        pic.set_irq_line(1, true);
        assert_eq!(pic.master.irr & 0x02, 0x02);
        assert_eq!(pic.poll_irq(), None); // IR1 blocked while IR0 in service
        pic.port_write(PIC_MASTER_CMD, 1, 0x20); // EOI IR0
        assert_eq!(pic.poll_irq(), Some(0x09));
    }
}
