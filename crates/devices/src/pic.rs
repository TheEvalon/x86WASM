//! Dual Intel 8259A Programmable Interrupt Controllers — ICW1–ICW4 only.
//!
//! Classic PC ports: master `0x20`/`0x21`, slave `0xA0`/`0xA1`, cascade on IRQ2.
//!
//! # Spec refs
//!
//! - Intel 8259A Programmable Interrupt Controller datasheet — Initialization
//!   Command Words ICW1–ICW4 programming sequence (A0/D4 decode; SNGL; IC4;
//!   master/slave ICW3; µPM / 8086 mode in ICW4).
//! - Classic IBM PC/AT: master at `0x20`/`0x21`, slave at `0xA0`/`0xA1`, slave
//!   cascaded on master IR2.
//! - `docs/machine-model-pc-v1.md`, `plan.md` §15.3 / §21 / §23.3.
//!
//! # Scope (this slice)
//!
//! Accepts ICW1–ICW4 initialization for single and cascaded configurations.
//! Records vector base (ICW2), cascade wiring (ICW3), and ICW4 8086-mode bit.
//!
//! # Unsupported (explicit)
//!
//! - OCW1–OCW3 (IMR, EOI, rotate, IRR/ISR read select)
//! - IRQ assertion, priority, Auto-EOI runtime delivery
//! - `poll_external_irq` / PIC→CPU delivery (port decode is owned by `machine-pc`)
//!
//! # Invalid / incomplete init (documented behavior)
//!
//! - Command-port writes with D4=0 are OCW2/OCW3: **ignored** (no OCW state).
//! - Data-port writes outside an active ICW sequence are OCW1: **ignored**.
//! - Incomplete sequences leave `initialized == false`; prior completed state is
//!   cleared when a new ICW1 restarts the sequence.
//! - A new ICW1 (A0=0, D4=1) always restarts initialization on that chip.

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InitPhase {
    /// Not in an ICW sequence (ready for OCW — ignored here — or a new ICW1).
    Idle,
    ExpectIcw2,
    ExpectIcw3,
    ExpectIcw4,
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
    /// ICW1.LTIM — level-triggered when true.
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

    fn write_cmd(&mut self, value: u8) {
        if value & ICW1_D4 == 0 {
            // OCW2 / OCW3 — out of scope for this slice.
            return;
        }
        self.begin_icw1(value);
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
        self.phase = InitPhase::ExpectIcw2;
    }

    fn write_data(&mut self, value: u8) {
        match self.phase {
            InitPhase::Idle => {
                // OCW1 (IMR) — out of scope; ignore.
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

    fn chip_mut(&mut self, port: u16) -> Option<(&mut Pic8259, bool)> {
        match port {
            PIC_MASTER_CMD => Some((&mut self.master, false)),
            PIC_MASTER_DATA => Some((&mut self.master, true)),
            PIC_SLAVE_CMD => Some((&mut self.slave, false)),
            PIC_SLAVE_DATA => Some((&mut self.slave, true)),
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
        // IRR/ISR/IMR reads require OCW3/OCW1 — unsupported; open-bus style.
        if matches!(
            port,
            PIC_MASTER_CMD | PIC_MASTER_DATA | PIC_SLAVE_CMD | PIC_SLAVE_DATA
        ) {
            0xFF
        } else {
            0xFFFF_FFFF
        }
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let mut pic2 = DualPic::new();
        // Pollute then reset.
        pic2.port_write(PIC_MASTER_CMD, 1, 0x11);
        pic2.reset();
        assert_eq!(pic2, DualPic::new());
    }

    /// Spec: ICW1 SNGL=1, IC4=1 → ICW2 then ICW4 (no ICW3). Intel 8259A datasheet.
    #[test]
    fn single_mode_icw_sequence() {
        let mut pic = DualPic::new();
        // ICW1: D4=1, SNGL=1, IC4=1 → 0x13
        pic.port_write(PIC_MASTER_CMD, 1, 0x13);
        assert!(!pic.master.initialized);
        pic.port_write(PIC_MASTER_DATA, 1, 0x08); // ICW2
        assert!(!pic.master.initialized);
        pic.port_write(PIC_MASTER_DATA, 1, 0x01); // ICW4 8086
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

        // Master: ICW1=0x11 (cascade, need ICW4), ICW2=0x08, ICW3=0x04, ICW4=0x01
        pic.port_write(PIC_MASTER_CMD, 1, 0x11);
        pic.port_write(PIC_MASTER_DATA, 1, 0x08);
        pic.port_write(PIC_MASTER_DATA, 1, 0x04);
        pic.port_write(PIC_MASTER_DATA, 1, 0x01);
        assert!(pic.master.initialized);
        assert!(!pic.master.single);
        assert_eq!(pic.master.vector_base, 0x08);
        assert_eq!(pic.master.slave_ir_mask(), 0x04);
        assert!(pic.master.mode_8086);
        assert_eq!(pic.master.irq_vector(0), Some(0x08));
        assert_eq!(pic.master.irq_vector(2), Some(0x0A));

        // Slave: ICW1=0x11, ICW2=0x70, ICW3=0x02, ICW4=0x01
        pic.port_write(PIC_SLAVE_CMD, 1, 0x11);
        pic.port_write(PIC_SLAVE_DATA, 1, 0x70);
        pic.port_write(PIC_SLAVE_DATA, 1, 0x02);
        pic.port_write(PIC_SLAVE_DATA, 1, 0x01);
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
        chip.write_cmd(0x13); // single + ICW4
        chip.write_data(0x28 | 0x03); // base 0x28 with junk in 2:0
        chip.write_data(0x01);
        assert!(chip.initialized);
        assert_eq!(chip.irq_vector(0), Some(0x28));
        assert_eq!(chip.irq_vector(5), Some(0x2D));
    }

    /// Incomplete init stays uninitialized; OCW-looking writes do not complete it.
    #[test]
    fn invalid_incomplete_init_stays_uninitialized() {
        let mut pic = DualPic::new();

        // Start ICW1 cascade+ICW4 but stop after ICW2.
        pic.port_write(PIC_MASTER_CMD, 1, 0x11);
        pic.port_write(PIC_MASTER_DATA, 1, 0x20);
        assert!(!pic.master.initialized);
        assert_eq!(pic.master.phase, InitPhase::ExpectIcw3);

        // OCW2-style write on command port (D4=0) must not finish init.
        pic.port_write(PIC_MASTER_CMD, 1, 0x20); // nonspecific EOI pattern
        assert!(!pic.master.initialized);
        assert_eq!(pic.master.phase, InitPhase::ExpectIcw3);

        // Fresh DualPic: data write before ICW1 is OCW1 → ignored.
        let mut pic2 = DualPic::new();
        pic2.port_write(PIC_MASTER_DATA, 1, 0xFF);
        assert!(!pic2.master.initialized);
        assert_eq!(pic2.master.phase, InitPhase::Idle);

        // New ICW1 restarts and clears prior partial state.
        pic.port_write(PIC_MASTER_CMD, 1, 0x13); // restart as single+ICW4
        assert!(!pic.master.initialized);
        assert_eq!(pic.master.vector_base, 0);
        assert_eq!(pic.master.phase, InitPhase::ExpectIcw2);
    }

    /// Master ICW3 bitmask and slave ICW3 ID stored for cascade IRQ2 wiring.
    #[test]
    fn master_slave_icw3_wiring() {
        let mut pic = DualPic::new();
        for (cmd, data, icw2, icw3) in [
            (PIC_MASTER_CMD, PIC_MASTER_DATA, 0x20u8, 0x04u8),
            (PIC_SLAVE_CMD, PIC_SLAVE_DATA, 0x28u8, 0x02u8),
        ] {
            pic.port_write(cmd, 1, 0x11);
            pic.port_write(data, 1, u32::from(icw2));
            pic.port_write(data, 1, u32::from(icw3));
            pic.port_write(data, 1, 0x01);
        }
        assert_eq!(pic.master.slave_ir_mask(), 1 << 2);
        assert_eq!(pic.slave.slave_id(), 2);
        // Classic PC: slave identity matches master's IR2 bit.
        assert_ne!(pic.master.slave_ir_mask() & (1 << pic.slave.slave_id()), 0);
    }

    /// Clone / PartialEq round-trip of architectural ICW state.
    #[test]
    fn state_clone_equality_round_trip() {
        let mut pic = DualPic::new();
        pic.port_write(PIC_MASTER_CMD, 1, 0x11);
        pic.port_write(PIC_MASTER_DATA, 1, 0x20);
        pic.port_write(PIC_MASTER_DATA, 1, 0x04);
        pic.port_write(PIC_MASTER_DATA, 1, 0x01);
        pic.port_write(PIC_SLAVE_CMD, 1, 0x11);
        pic.port_write(PIC_SLAVE_DATA, 1, 0x28);
        pic.port_write(PIC_SLAVE_DATA, 1, 0x02);
        pic.port_write(PIC_SLAVE_DATA, 1, 0x01);

        let cloned = pic.clone();
        assert_eq!(pic, cloned);
        assert!(cloned.master.initialized);
        assert!(cloned.slave.initialized);
        assert_eq!(cloned.master.vector_base, 0x20);
        assert_eq!(cloned.slave.vector_base, 0x28);
        assert_eq!(cloned.master.icw3, 0x04);
        assert_eq!(cloned.slave.icw3, 0x02);
        assert!(cloned.master.mode_8086);
        assert!(cloned.slave.mode_8086);
    }

    #[test]
    fn port_decode_ignores_unrelated_ports() {
        let mut pic = DualPic::new();
        pic.port_write(0x3F8, 1, 0x11);
        assert!(!pic.master.initialized);
        assert_eq!(pic.port_read(0x3F8, 1), 0xFFFF_FFFF);
        assert_eq!(pic.port_read(PIC_MASTER_CMD, 1), 0xFF);
    }

    /// Cascade without ICW4 (IC4=0) completes after ICW3.
    #[test]
    fn cascade_without_icw4_completes_after_icw3() {
        let mut chip = Pic8259::new_master();
        chip.write_cmd(0x10); // D4=1, SNGL=0, IC4=0
        chip.write_data(0x08);
        assert!(!chip.initialized);
        chip.write_data(0x04);
        assert!(chip.initialized);
        assert!(!chip.expect_icw4);
        assert!(!chip.mode_8086);
        assert_eq!(chip.icw4, 0);
    }
}
