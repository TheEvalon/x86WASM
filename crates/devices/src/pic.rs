//! Dual Intel 8259A Programmable Interrupt Controllers — ICW + OCW runtime.
//!
//! Classic PC ports: master `0x20`/`0x21`, slave `0xA0`/`0xA1`, cascade on IRQ2.
//!
//! # Spec refs
//!
//! - Intel 8259A Programmable Interrupt Controller datasheet — ICW1–ICW4
//!   (incl. ICW4.AEOI Automatic EOI); OCW1 (IMR); OCW2 non-specific / specific
//!   EOI; OCW2 Automatic Rotation (Rotate on Non-Specific EOI + Rotate in
//!   Automatic EOI Mode); OCW2 Specific Rotation (Set Priority Command +
//!   Rotate on Specific EOI); OCW3 IRR/ISR read select; OCW3 poll command
//!   (`P=1`); OCW3 Special Mask Mode (`ESMM`/`SMM`); IRR/ISR; fully nested
//!   priority; Special Fully Nested Mode (ICW4.SFNM on master); cascade EOI
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
//! - OCW2 Automatic Rotation: Rotate on Non-Specific EOI (`R=1,SL=0,EOI=1`) and
//!   Rotate in Automatic EOI Mode set/clear (`R=1,SL=0,EOI=0` / `R=0,SL=0,EOI=0`)
//! - OCW2 Specific Rotation: Set Priority Command (`R=1,SL=1,EOI=0` + L2–L0) and
//!   Rotate on Specific EOI (`R=1,SL=1,EOI=1` + L2–L0)
//! - OCW3 read-register select (`RR`/`RIS`) for IRR/ISR on command-port reads
//! - OCW3 poll command (`P=1`): one-shot acknowledging command-port read
//!   returning `0x80 | level`, including software-sequenced cascaded polling
//! - Automatic EOI (ICW4.AEOI): after INTA / OCW3-poll acknowledge sets an ISR
//!   bit, that bit is cleared at the end of the acknowledge sequence (no OCW2
//!   EOI required)
//! - Special Mask Mode (OCW3 `ESMM`/`SMM`): when active, a masked in-service IR
//!   does not block lower-priority unmasked recognition (IMR still applies);
//!   non-specific EOI skips masked IS bits
//! - Special Fully Nested Mode (ICW4.SFNM on master): a slave-connected in-service
//!   IR does not lock that cascade line out of the master's priority logic, so a
//!   higher-priority IR on the same slave can be delivered without a master EOI
//! - Edge-triggered IR line assert (ICW1.LTIM=0): rising edge latches IRR
//! - Level-triggered IR line assert (ICW1.LTIM=1): IRR follows IR level; deassert
//!   clears IRR; acknowledge while still high re-pending IRR for post-EOI delivery
//! - Spurious / DEFAULT IR7 when IR pin is low at first INTA (vector IR7, ISR
//!   bit7 not set); cascade master IR is never remapped — empty/spurious slave
//!   still sets master cascade IS
//! - IRR→ISR on acknowledge, vector selection
//! - `DualPic::acknowledge` / `poll_irq` for `MachineBus::poll_external_irq`
//!
//! # Unsupported (explicit)
//!
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
/// ICW4 bit1: Automatic EOI (AEOI).
const ICW4_AEOI: u8 = 1 << 1;
/// ICW4 bit4: Special Fully Nested Mode (SFNM) — programmed on the master.
const ICW4_SFNM: u8 = 1 << 4;

/// OCW2/OCW3: D4 must be 0 (else ICW1).
/// OCW2: D3=0; OCW3: D3=1 (datasheet Operation Command Word format).
const OCW_D3: u8 = 1 << 3;
/// OCW2 bits.
const OCW2_EOI: u8 = 1 << 5;
const OCW2_SL: u8 = 1 << 6;
const OCW2_R: u8 = 1 << 7;
/// OCW3 bits (Intel 8259A: `D7=0, ESMM, SMM, D4=0, D3=1, P, RR, RIS`).
const OCW3_RIS: u8 = 1 << 0;
const OCW3_RR: u8 = 1 << 1;
const OCW3_P: u8 = 1 << 2;
const OCW3_SMM: u8 = 1 << 5;
const OCW3_ESMM: u8 = 1 << 6;

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
    /// ICW1.LTIM — level-triggered when true (edge-triggered when false).
    ///
    /// Spec: Intel 8259A datasheet — LTIM=1 disables edge detect; a high level
    /// on IR is a valid request. The request must be removed before EOI (or
    /// before IF is re-enabled) to avoid a second interrupt.
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
    /// ICW4.AEOI — Automatic End of Interrupt.
    ///
    /// Spec: Intel 8259A datasheet — when AEOI is set, the ISR bit latched by
    /// the interrupt-acknowledge sequence is cleared at the end of that
    /// sequence (no OCW2 EOI required). Applies to hardware INTA and to the
    /// OCW3 poll-command acknowledge path (shared [`Pic8259::ack_ir`]).
    pub auto_eoi: bool,
    /// ICW4.SFNM — Special Fully Nested Mode (meaningful on the master).
    ///
    /// Spec: Intel 8259A datasheet — when SFNM is programmed on the master in a
    /// cascade configuration, a slave whose request is in service is not locked
    /// out of the master's priority logic: further interrupt requests from
    /// higher-priority IRs within that slave are recognized (the master's
    /// slave-connected IS bit blocks only strictly lower-priority master IRs,
    /// not equal-priority re-entry on the cascade line). Cleared by ICW1 / reset.
    /// Software should EOI the slave, read slave ISR, and only EOI the master
    /// when the slave ISR is empty.
    pub special_fully_nested: bool,
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
    /// OCW3 Special Mask Mode (`ESMM`/`SMM`). Cleared by ICW1 / reset.
    ///
    /// Spec: Intel 8259A datasheet — when SMM is set, masking an in-service IR
    /// via OCW1 inhibits that level but enables other unmasked levels (including
    /// lower priority) that fully-nested mode would otherwise block. Cleared
    /// when `ESMM=1,SMM=0`; unchanged when `ESMM=0`.
    pub special_mask_mode: bool,
    /// IR line currently assigned lowest priority (0–7).
    ///
    /// Spec: Intel 8259A — after init IR0 is highest and IR7 lowest
    /// (`lowest_priority = 7`). Automatic Rotation assigns the just-serviced IR
    /// as the new bottom; the next IR (mod 8) becomes highest.
    pub lowest_priority: u8,
    /// Rotate in Automatic EOI Mode (OCW2 `R=1,SL=0,EOI=0` set /
    /// `R=0,SL=0,EOI=0` clear). When set with ICW4.AEOI, each acknowledge
    /// rotates so the serviced IR becomes lowest priority.
    pub rotate_on_auto_eoi: bool,
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
            auto_eoi: false,
            special_fully_nested: false,
            imr: 0xFF,
            irr: 0,
            isr: 0,
            ir_level: 0,
            read_reg: ReadReg::Irr,
            poll_command_armed: false,
            special_mask_mode: false,
            lowest_priority: 7,
            rotate_on_auto_eoi: false,
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

    /// Drive IR`irq` (0–7).
    ///
    /// Spec: Intel 8259A ICW1.LTIM —
    /// - Edge (LTIM=0): low→high latches IRR; holding high does not re-request;
    ///   deassert does not clear a latched IRR bit.
    /// - Level (LTIM=1): high level sets IRR; deassert clears that IRR bit.
    pub fn set_irq_line(&mut self, irq: u8, high: bool) {
        if irq > 7 {
            return;
        }
        let bit = 1u8 << irq;
        let was_high = self.ir_level & bit != 0;
        if high {
            self.ir_level |= bit;
            if self.level_triggered {
                self.irr |= bit;
            } else if !was_high {
                // Edge sense: rising edge latches IRR (datasheet ICW1 / edge mode).
                self.irr |= bit;
            }
        } else {
            self.ir_level &= !bit;
            if self.level_triggered {
                // Level mode: removing the IR level removes the request.
                self.irr &= !bit;
            }
        }
    }

    /// Priority rank of `ir`: 0 = highest … 7 = lowest (rotated).
    ///
    /// Spec: Intel 8259A — with bottom priority `lowest_priority`, the next IR
    /// (mod 8) has highest priority.
    fn priority_rank(&self, ir: u8) -> u8 {
        ir.wrapping_sub(self.lowest_priority.wrapping_add(1)) & 7
    }

    /// Highest-priority unmasked IRR request not blocked by nested ISR rules.
    ///
    /// Spec: Intel 8259A fully nested mode — default IR0 highest … IR7 lowest;
    /// Automatic Rotation reassigns the bottom. With Special Mask Mode, an
    /// in-service IR whose IMR bit is set does not block lower-priority
    /// unmasked requests (IMR still masks its own level). With Special Fully
    /// Nested Mode on a master, a slave-connected IS bit does not lock out
    /// equal-priority re-entry on that cascade IR.
    fn highest_priority_request(&self) -> Option<u8> {
        if !self.initialized {
            return None;
        }
        let highest = self.lowest_priority.wrapping_add(1) & 7;
        for offset in 0u8..8 {
            let ir = highest.wrapping_add(offset) & 7;
            let bit = 1u8 << ir;
            if self.irr & bit == 0 || self.imr & bit != 0 {
                continue;
            }
            if self.isr_blocks(ir) {
                continue;
            }
            return Some(ir);
        }
        None
    }

    /// Whether fully-nested / special-mask / SFNM ISR state blocks recognizing `ir`.
    fn isr_blocks(&self, ir: u8) -> bool {
        if self.isr == 0 {
            return false;
        }
        let ir_rank = self.priority_rank(ir);
        if self.special_mask_mode {
            // SMM: a higher-or-equal priority IS bit blocks only while unmasked.
            for hp in 0u8..8 {
                let hp_bit = 1u8 << hp;
                if self.isr & hp_bit == 0 || self.imr & hp_bit != 0 {
                    continue;
                }
                if self.priority_rank(hp) <= ir_rank {
                    return true;
                }
            }
            false
        } else {
            // Fully nested: in-service bits block equal-or-lower priority IRs.
            // Special Fully Nested Mode (master): a slave-connected IS bit does
            // not lock that cascade IR out — it blocks only strictly lower
            // priority (Intel 8259A SFNM: slave not locked out of master logic).
            for hp in 0u8..8 {
                let hp_bit = 1u8 << hp;
                if self.isr & hp_bit == 0 {
                    continue;
                }
                let hp_rank = self.priority_rank(hp);
                let sfnm_slave_is = self.special_fully_nested
                    && self.is_master
                    && (self.slave_ir_mask() & hp_bit) != 0;
                if sfnm_slave_is {
                    if hp_rank < ir_rank {
                        return true;
                    }
                } else if hp_rank <= ir_rank {
                    return true;
                }
            }
            false
        }
    }

    /// Highest-priority (best rank) ISR bit, optionally skipping IMR-masked bits
    /// (Special Mask Mode non-specific EOI).
    fn highest_priority_isr_bit(&self, skip_masked: bool) -> Option<u8> {
        if self.isr == 0 {
            return None;
        }
        let highest = self.lowest_priority.wrapping_add(1) & 7;
        for offset in 0u8..8 {
            let ir = highest.wrapping_add(offset) & 7;
            let bit = 1u8 << ir;
            if self.isr & bit == 0 {
                continue;
            }
            if skip_masked && self.imr & bit != 0 {
                continue;
            }
            return Some(ir);
        }
        None
    }

    /// Assign `ir` as lowest priority (Automatic / Specific Rotation).
    fn rotate_lowest_to(&mut self, ir: u8) {
        self.lowest_priority = ir & 7;
    }

    /// True if this chip would assert INT (unmasked request not nested-blocked).
    pub fn int_pending(&self) -> bool {
        self.highest_priority_request().is_some()
    }

    /// Acknowledge selected IR`ir` and return the IR index used for the vector.
    ///
    /// Spec: Intel 8259A — IR inputs must remain high until after the falling
    /// edge of the first INTA. If the pin is low at acknowledge, a DEFAULT IR7
    /// occurs: the vector is IR7 but ISR bit7 is **not** set (a real IR7 does
    /// set it). Cascade IR on a master is never remapped to DEFAULT — the
    /// cascaded slave may still deliver DEFAULT IR7 on its own chip.
    ///
    /// Spec: level mode — if IR remains high after a normal acknowledge, IRR is
    /// re-pended so EOI can re-deliver. AEOI clears the IS bit (and may rotate)
    /// only for a non-spurious acknowledge.
    fn ack_ir(&mut self, ir: u8) -> u8 {
        let cascade_ir =
            self.is_master && !self.single && (self.slave_ir_mask() & (1u8 << ir)) != 0;
        self.ack_ir_inner(ir, !cascade_ir)
    }

    fn ack_ir_inner(&mut self, ir: u8, detect_spurious: bool) -> u8 {
        let bit = 1u8 << ir;
        self.irr &= !bit;
        if detect_spurious && self.ir_level & bit == 0 {
            // DEFAULT IR7 — do not set ISR bit7 (or any other IS bit).
            return 7;
        }
        self.isr |= bit;
        if self.level_triggered && self.ir_level & bit != 0 {
            self.irr |= bit;
        }
        if self.auto_eoi {
            self.isr &= !bit;
            if self.rotate_on_auto_eoi {
                self.rotate_lowest_to(ir);
            }
        }
        ir
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
        self.auto_eoi = false;
        self.special_fully_nested = false;
        // Datasheet ICW1: edge sense circuit is reset; clear request state;
        // Special Mask Mode is cleared and Status Read is set to IRR.
        self.irr = 0;
        self.isr = 0;
        self.ir_level = 0;
        self.read_reg = ReadReg::Irr;
        self.poll_command_armed = false;
        self.special_mask_mode = false;
        self.lowest_priority = 7;
        self.rotate_on_auto_eoi = false;
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
                self.auto_eoi = value & ICW4_AEOI != 0;
                // SFNM is specified for the master in cascade configurations;
                // store the bit only when this chip is the master.
                self.special_fully_nested = self.is_master && value & ICW4_SFNM != 0;
                self.finish_init();
            }
        }
    }

    fn finish_init(&mut self) {
        self.phase = InitPhase::Idle;
        self.initialized = true;
        self.read_reg = ReadReg::Irr;
    }

    /// OCW2: EOI, Automatic Rotation, and Specific Rotation commands.
    ///
    /// Spec: Intel 8259A OCW2 (`R`/`SL`/`EOI` / L2–L0):
    /// - `R=0,SL=0,EOI=1` — non-specific EOI
    /// - `R=0,SL=1,EOI=1` — specific EOI
    /// - `R=1,SL=0,EOI=1` — Rotate on Non-Specific EOI
    /// - `R=1,SL=0,EOI=0` — Rotate in Automatic EOI Mode (set)
    /// - `R=0,SL=0,EOI=0` — Rotate in Automatic EOI Mode (clear)
    /// - `R=0,SL=1,EOI=0` — no operation
    /// - `R=1,SL=1,EOI=0` — Set Priority Command (L2–L0 → lowest)
    /// - `R=1,SL=1,EOI=1` — Rotate on Specific EOI (clear ISR L + L → lowest)
    fn write_ocw2(&mut self, value: u8) {
        let r = value & OCW2_R != 0;
        let sl = value & OCW2_SL != 0;
        let eoi = value & OCW2_EOI != 0;

        if r && sl {
            // Specific Rotation: L2–L0 becomes lowest priority.
            let level = value & 0x07;
            if eoi {
                // Rotate on Specific EOI: clear named ISR bit, then rotate.
                self.isr &= !(1u8 << level);
            }
            // Set Priority (`EOI=0`) rotates without touching ISR.
            self.rotate_lowest_to(level);
            return;
        }

        if !eoi {
            if !sl {
                // Rotate in Automatic EOI Mode set (`R=1`) / clear (`R=0`).
                self.rotate_on_auto_eoi = r;
            }
            // `R=0,SL=1,EOI=0` = nop.
            return;
        }

        if sl {
            // Specific EOI: clear ISR bit L2–L0.
            let level = value & 0x07;
            self.isr &= !(1u8 << level);
            return;
        }

        // Non-specific EOI (± rotate): clear highest-priority IS bit.
        let skip_masked = self.special_mask_mode;
        if let Some(level) = self.highest_priority_isr_bit(skip_masked) {
            self.isr &= !(1u8 << level);
            if r {
                // Rotate on Non-Specific EOI: that level becomes lowest priority.
                self.rotate_lowest_to(level);
            }
        }
    }

    /// OCW3: Special Mask Mode (`ESMM`/`SMM`), poll command (`P=1`), and
    /// IRR/ISR read select (`RR`/`RIS`).
    ///
    /// Spec: Intel 8259A datasheet — OCW3 format / Special Mask Mode / Poll
    /// Command. Model choice: `P=1` arms the poll for the next command-port
    /// read and leaves the `RR`/`RIS` selection untouched (the datasheet does
    /// not define combining a poll with a read-register select). `ESMM`/`SMM`
    /// are still applied when combined with `P=1`.
    fn write_ocw3(&mut self, value: u8) {
        if value & OCW3_ESMM != 0 {
            self.special_mask_mode = value & OCW3_SMM != 0;
        }
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
    /// the IR bit cleared via the shared [`Pic8259::ack_ir`] path (level mode
    /// may re-pending IRR while IR stays high; DEFAULT IR7 reports level 7
    /// without setting ISR bit7).
    /// With ICW4.AEOI the IS bit is cleared again at the end of that acknowledge.
    /// Model choice: with nothing pending the byte is `0x00` (bit7 clear, level
    /// bits zero) — the datasheet leaves the level bits unspecified there.
    fn take_poll_command_byte(&mut self) -> u8 {
        self.poll_command_armed = false;
        match self.highest_priority_request() {
            Some(ir) => {
                let vec_ir = self.ack_ir(ir);
                0x80 | vec_ir
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
    /// selects a cascaded IR. Without AEOI, EOI must later clear both slave and
    /// master ISR bits; with ICW4.AEOI on a chip, that chip clears its ISR bit
    /// at the end of its acknowledge (see [`Pic8259::ack_ir`]).
    ///
    /// Spec: Intel 8259A DEFAULT IR7 — if a device IR pin is low at first INTA,
    /// the chip returns the IR7 vector without setting ISR bit7. A cascaded
    /// master IR is never remapped; if the slave has no request (or its own IR
    /// pin dropped), the slave delivers DEFAULT IR7 while the master still sets
    /// its cascade IS bit.
    pub fn acknowledge(&mut self) -> Option<u8> {
        self.sync_cascade();
        let ir = self.master.highest_priority_request()?;
        let bit = 1u8 << ir;
        if !self.master.single && (self.master.slave_ir_mask() & bit) != 0 {
            let slave_vec_ir = match self.slave.highest_priority_request() {
                Some(slave_ir) => self.slave.ack_ir(slave_ir),
                // Cascade selected but slave IRR empty → slave DEFAULT IR7.
                None => 7,
            };
            let _ = self.master.ack_ir(ir);
            let vec = self.slave.irq_vector(slave_vec_ir);
            self.sync_cascade();
            return vec;
        }
        let vec_ir = self.master.ack_ir(ir);
        self.master.irq_vector(vec_ir)
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
        init_at_cascade_icw4(pic, 0x01);
    }

    /// PC AT cascade init with an explicit ICW4 byte (µPM / AEOI / SFNM / …).
    fn init_at_cascade_icw4(pic: &mut DualPic, icw4: u8) {
        init_at_cascade_icw4_roles(pic, icw4, icw4);
    }

    /// PC AT cascade with distinct master/slave ICW4 (SFNM is master-only).
    fn init_at_cascade_icw4_roles(pic: &mut DualPic, master_icw4: u8, slave_icw4: u8) {
        // Master: ICW1=0x11, ICW2=0x08, ICW3=0x04, ICW4=`master_icw4`
        pic.port_write(PIC_MASTER_CMD, 1, 0x11);
        pic.port_write(PIC_MASTER_DATA, 1, 0x08);
        pic.port_write(PIC_MASTER_DATA, 1, 0x04);
        pic.port_write(PIC_MASTER_DATA, 1, u32::from(master_icw4));
        // Slave: ICW1=0x11, ICW2=0x70, ICW3=0x02, ICW4=`slave_icw4`
        pic.port_write(PIC_SLAVE_CMD, 1, 0x11);
        pic.port_write(PIC_SLAVE_DATA, 1, 0x70);
        pic.port_write(PIC_SLAVE_DATA, 1, 0x02);
        pic.port_write(PIC_SLAVE_DATA, 1, u32::from(slave_icw4));
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
        assert!(!pic.master.auto_eoi);
        assert!(!pic.slave.auto_eoi);
        assert!(!pic.master.special_fully_nested);
        assert!(!pic.slave.special_fully_nested);
        assert!(!pic.master.special_mask_mode);
        assert!(!pic.slave.special_mask_mode);
        assert_eq!(pic.master.lowest_priority, 7);
        assert_eq!(pic.slave.lowest_priority, 7);
        assert!(!pic.master.rotate_on_auto_eoi);
        assert!(!pic.slave.rotate_on_auto_eoi);
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

    /// Spec: Intel 8259A datasheet — ICW4 bit1 (AEOI). With Automatic EOI, the
    /// ISR bit set by the interrupt-acknowledge sequence is cleared at the end
    /// of that sequence; no OCW2 EOI is required before the next delivery.
    #[test]
    fn aeoi_clears_isr_after_acknowledge_without_eoi() {
        let mut pic = DualPic::new();
        // ICW4 = 0x03: µPM | AEOI
        init_at_cascade_icw4(&mut pic, 0x03);
        assert!(pic.master.auto_eoi);
        assert!(pic.slave.auto_eoi);
        assert_eq!(pic.master.icw4, 0x03);

        pic.port_write(PIC_MASTER_DATA, 1, 0xFE); // unmask IR0
        pic.set_irq_line(0, true);
        assert_eq!(pic.poll_irq(), Some(0x08));
        assert_eq!(pic.master.irr, 0);
        // AEOI: ISR cleared automatically after INTA / vector delivery.
        assert_eq!(pic.master.isr, 0);

        // Fresh edge delivers again without an OCW2 EOI.
        pic.set_irq_line(0, false);
        pic.set_irq_line(0, true);
        assert_eq!(pic.poll_irq(), Some(0x08));
        assert_eq!(pic.master.isr, 0);
    }

    /// Spec: Intel 8259A datasheet — without AEOI (ICW4.AEOI=0), ISR remains set
    /// until an OCW2 EOI; a second edge cannot deliver while nested-blocked.
    #[test]
    fn without_aeoi_manual_eoi_still_required() {
        let mut pic = DualPic::new();
        init_at_cascade_icw4(&mut pic, 0x01); // µPM only
        assert!(!pic.master.auto_eoi);

        pic.port_write(PIC_MASTER_DATA, 1, 0xFE);
        pic.set_irq_line(0, true);
        assert_eq!(pic.poll_irq(), Some(0x08));
        assert_eq!(pic.master.isr, 0x01);
        pic.set_irq_line(0, false);
        pic.set_irq_line(0, true);
        // Still in service → fully nested blocks the new IR0 request.
        assert_eq!(pic.poll_irq(), None);
        pic.port_write(PIC_MASTER_CMD, 1, 0x20); // non-specific EOI
        assert_eq!(pic.master.isr, 0);
        assert_eq!(pic.poll_irq(), Some(0x08));
    }

    /// Spec: Intel 8259A datasheet — Poll Command is an interrupt acknowledge;
    /// with AEOI the IS bit set by that acknowledge is cleared automatically
    /// (same shared ack path as INTA).
    #[test]
    fn aeoi_clears_isr_on_ocw3_poll_command_ack() {
        let mut pic = DualPic::new();
        init_at_cascade_icw4(&mut pic, 0x03);
        pic.port_write(PIC_MASTER_DATA, 1, 0x00);
        pic.set_irq_line(3, true);

        pic.port_write(PIC_MASTER_CMD, 1, 0x0C); // OCW3 P=1
        assert_eq!(pic.port_read(PIC_MASTER_CMD, 1), 0x80 | 3);
        assert_eq!(pic.master.irr & (1 << 3), 0);
        assert_eq!(pic.master.isr, 0);
    }

    /// Spec: Intel 8259A cascade + AEOI — each chip clears its own ISR bit after
    /// its acknowledge; cascaded INTA therefore leaves master and slave ISR clear
    /// without OCW2 EOI on either chip.
    #[test]
    fn aeoi_cascade_clears_master_and_slave_isr() {
        let mut pic = DualPic::new();
        init_at_cascade_icw4(&mut pic, 0x03);
        pic.port_write(PIC_MASTER_DATA, 1, 0xFB); // unmask master IR2
        pic.port_write(PIC_SLAVE_DATA, 1, 0xFD); // unmask slave IR1
        pic.set_irq_line(9, true);
        assert_eq!(pic.poll_irq(), Some(0x71));
        assert_eq!(pic.slave.isr, 0);
        assert_eq!(pic.master.isr, 0);

        // Re-edge delivers again without EOI.
        pic.set_irq_line(9, false);
        pic.set_irq_line(9, true);
        assert_eq!(pic.poll_irq(), Some(0x71));
    }

    /// Spec: Intel 8259A OCW3 — ESMM=1,SMM=1 sets Special Mask Mode; ESMM=1,SMM=0
    /// clears it; ESMM=0 leaves SMM unchanged (SMM is don't-care).
    #[test]
    fn ocw3_sets_and_clears_special_mask_mode() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        assert!(!pic.master.special_mask_mode);

        // OCW3: ESMM=1, SMM=1, D3=1 → 0x68
        pic.port_write(PIC_MASTER_CMD, 1, 0x68);
        assert!(pic.master.special_mask_mode);
        assert!(!pic.slave.special_mask_mode);

        // ESMM=0, SMM=1 (0x28): SMM bit is don't-care — state unchanged.
        pic.port_write(PIC_MASTER_CMD, 1, 0x28);
        assert!(pic.master.special_mask_mode);

        // OCW3: ESMM=1, SMM=0, D3=1 → 0x48
        pic.port_write(PIC_MASTER_CMD, 1, 0x48);
        assert!(!pic.master.special_mask_mode);

        pic.port_write(PIC_SLAVE_CMD, 1, 0x68);
        assert!(pic.slave.special_mask_mode);
    }

    /// Spec: Intel 8259A Special Mask Mode — with SMM active, masking the
    /// in-service IR (OCW1) does not block lower-priority unmasked requests the
    /// normal fully-nested way.
    #[test]
    fn special_mask_mode_allows_lower_priority_when_servicing_ir_masked() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        pic.port_write(PIC_MASTER_DATA, 1, 0x00); // unmask all

        pic.set_irq_line(3, true);
        assert_eq!(pic.poll_irq(), Some(0x08 | 3));
        assert_eq!(pic.master.isr, 1 << 3);

        // Without SMM, lower-priority IR6 stays nested-blocked.
        pic.set_irq_line(6, true);
        assert_eq!(pic.poll_irq(), None);

        // Enter SMM and mask the in-service level (IR3).
        pic.port_write(PIC_MASTER_CMD, 1, 0x68); // ESMM|SMM
        pic.port_write(PIC_MASTER_DATA, 1, 1 << 3); // mask IR3 only
        assert_eq!(pic.poll_irq(), Some(0x08 | 6));
        assert_eq!(pic.master.isr & (1 << 6), 1 << 6);
    }

    /// Spec: with SMM on but the in-service IR left unmasked, fully-nested
    /// blocking of lower-priority requests still applies.
    #[test]
    fn special_mask_mode_still_nested_blocks_when_servicing_unmasked() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        pic.port_write(PIC_MASTER_DATA, 1, 0x00);
        pic.port_write(PIC_MASTER_CMD, 1, 0x68); // SMM on

        pic.set_irq_line(3, true);
        assert_eq!(pic.poll_irq(), Some(0x08 | 3));
        pic.set_irq_line(6, true);
        // IR3 still in service and unmasked → IR6 blocked.
        assert_eq!(pic.poll_irq(), None);
    }

    /// Spec: Special Mask Mode still applies IMR — a masked IR never delivers.
    #[test]
    fn special_mask_mode_still_respects_imr() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        pic.port_write(PIC_MASTER_CMD, 1, 0x68);
        pic.port_write(PIC_MASTER_DATA, 1, 1 << 1); // mask IR1
        pic.set_irq_line(1, true);
        assert_eq!(pic.poll_irq(), None);
        pic.port_write(PIC_MASTER_DATA, 1, 0x00);
        assert_eq!(pic.poll_irq(), Some(0x08 | 1));
    }

    /// Spec: Intel 8259A — an IS bit masked by IMR is not cleared by a
    /// non-specific EOI while Special Mask Mode is active.
    #[test]
    fn special_mask_nonspecific_eoi_skips_masked_isr_bit() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        pic.port_write(PIC_MASTER_DATA, 1, 0x00);

        pic.set_irq_line(3, true);
        assert_eq!(pic.poll_irq(), Some(0x08 | 3));
        pic.port_write(PIC_MASTER_CMD, 1, 0x68); // SMM
        pic.port_write(PIC_MASTER_DATA, 1, 1 << 3); // mask in-service IR3

        pic.port_write(PIC_MASTER_CMD, 1, 0x20); // non-specific EOI
        assert_eq!(pic.master.isr, 1 << 3); // masked IS bit retained

        // Specific EOI still clears the named bit.
        pic.port_write(PIC_MASTER_CMD, 1, 0x63); // EOI|SL|L=3
        assert_eq!(pic.master.isr, 0);
    }

    /// Spec: Intel 8259A ICW1 — Special Mask Mode is cleared on initialization.
    #[test]
    fn icw1_clears_special_mask_mode() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        pic.port_write(PIC_MASTER_CMD, 1, 0x68);
        assert!(pic.master.special_mask_mode);
        // Re-init master ICW sequence.
        pic.port_write(PIC_MASTER_CMD, 1, 0x11);
        assert!(!pic.master.special_mask_mode);
    }

    /// Spec: OCW3 poll + SMM share priority resolution — masking an in-service
    /// IR under SMM lets poll acknowledge a lower-priority request.
    #[test]
    fn special_mask_mode_applies_to_ocw3_poll_command() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        pic.port_write(PIC_MASTER_DATA, 1, 0x00);
        pic.set_irq_line(1, true);
        assert_eq!(pic.poll_irq(), Some(0x09));
        pic.port_write(PIC_MASTER_CMD, 1, 0x68);
        pic.port_write(PIC_MASTER_DATA, 1, 1 << 1);
        pic.set_irq_line(5, true);

        pic.port_write(PIC_MASTER_CMD, 1, 0x0C); // OCW3 P=1
        assert_eq!(pic.port_read(PIC_MASTER_CMD, 1), 0x80 | 5);
        assert_eq!(pic.master.isr & (1 << 5), 1 << 5);
    }

    /// Spec: Intel 8259A Automatic Rotation — Rotate on Non-Specific EOI
    /// (`OCW2 R=1,SL=0,EOI=1` = `0xA0`): after clearing the highest-priority
    /// IS bit, that IR level is assigned lowest priority (next IR becomes
    /// highest).
    #[test]
    fn rotate_on_nonspecific_eoi_assigns_lowest_priority() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        pic.port_write(PIC_MASTER_DATA, 1, 0x00); // unmask all
        assert_eq!(pic.master.lowest_priority, 7);

        pic.set_irq_line(0, true);
        assert_eq!(pic.poll_irq(), Some(0x08));
        assert_eq!(pic.master.isr, 1 << 0);

        // Rotate on non-specific EOI: clear IS0, IR0 becomes lowest → IR1 highest.
        pic.port_write(PIC_MASTER_CMD, 1, 0xA0);
        assert_eq!(pic.master.isr, 0);
        assert_eq!(pic.master.lowest_priority, 0);

        pic.set_irq_line(0, false);
        pic.set_irq_line(0, true);
        pic.set_irq_line(1, true);
        // With IR0 lowest, IR1 outranks IR0.
        assert_eq!(pic.poll_irq(), Some(0x09));
        assert_eq!(pic.master.isr, 1 << 1);
        assert_eq!(pic.master.irr, 1 << 0);
    }

    /// Spec: plain non-specific EOI (`R=0`) does not rotate priorities.
    #[test]
    fn nonspecific_eoi_without_r_does_not_rotate() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        pic.port_write(PIC_MASTER_DATA, 1, 0x00);

        pic.set_irq_line(0, true);
        assert_eq!(pic.poll_irq(), Some(0x08));
        pic.port_write(PIC_MASTER_CMD, 1, 0x20); // non-specific EOI, no rotate
        assert_eq!(pic.master.lowest_priority, 7);

        pic.set_irq_line(0, false);
        pic.set_irq_line(0, true);
        pic.set_irq_line(1, true);
        // Default fully nested: IR0 still highest.
        assert_eq!(pic.poll_irq(), Some(0x08));
    }

    /// Spec: Intel 8259A — Rotate in Automatic EOI Mode set (`OCW2 R=1,SL=0,EOI=0`
    /// = `0x80`): after AEOI acknowledge, the serviced IR becomes lowest priority.
    #[test]
    fn rotate_in_aeoi_mode_rotates_after_acknowledge() {
        let mut pic = DualPic::new();
        init_at_cascade_icw4(&mut pic, 0x03); // µPM | AEOI
        pic.port_write(PIC_MASTER_DATA, 1, 0x00);
        assert!(!pic.master.rotate_on_auto_eoi);

        pic.port_write(PIC_MASTER_CMD, 1, 0x80); // set Rotate in AEOI Mode
        assert!(pic.master.rotate_on_auto_eoi);

        pic.set_irq_line(3, true);
        assert_eq!(pic.poll_irq(), Some(0x0B));
        assert_eq!(pic.master.isr, 0); // AEOI cleared ISR
        assert_eq!(pic.master.lowest_priority, 3); // IR3 now lowest → IR4 highest

        pic.set_irq_line(3, false);
        pic.set_irq_line(3, true);
        pic.set_irq_line(4, true);
        assert_eq!(pic.poll_irq(), Some(0x0C)); // IR4 before IR3
        assert_eq!(pic.master.lowest_priority, 4);
        assert_eq!(pic.master.irr, 1 << 3);
    }

    /// Spec: Intel 8259A — Rotate in Automatic EOI Mode clear (`OCW2 R=0,SL=0,EOI=0`
    /// = `0x00`): further AEOI acknowledges do not rotate.
    #[test]
    fn rotate_in_aeoi_mode_clear_stops_rotation() {
        let mut pic = DualPic::new();
        init_at_cascade_icw4(&mut pic, 0x03);
        pic.port_write(PIC_MASTER_DATA, 1, 0x00);
        pic.port_write(PIC_MASTER_CMD, 1, 0x80); // set
        pic.set_irq_line(0, true);
        assert_eq!(pic.poll_irq(), Some(0x08));
        assert_eq!(pic.master.lowest_priority, 0);

        pic.port_write(PIC_MASTER_CMD, 1, 0x00); // clear Rotate in AEOI Mode
        assert!(!pic.master.rotate_on_auto_eoi);

        pic.set_irq_line(1, true);
        assert_eq!(pic.poll_irq(), Some(0x09));
        // Cleared: AEOI must not rotate further (lowest stays IR0).
        assert_eq!(pic.master.lowest_priority, 0);
        assert_eq!(pic.master.isr, 0);
    }

    /// Spec: ICW1 resets Automatic Rotation state (lowest = IR7, rotate-in-AEOI off).
    #[test]
    fn icw1_clears_rotate_state() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        pic.port_write(PIC_MASTER_CMD, 1, 0x80);
        pic.port_write(PIC_MASTER_DATA, 1, 0x00);
        // Use IR3 (not cascade IR2) so DualPic acknowledge stays on the master.
        pic.set_irq_line(3, true);
        // Without AEOI, 0x80 only arms the mode flag; force a rotate via 0xA0 path.
        assert_eq!(pic.poll_irq(), Some(0x0B));
        pic.port_write(PIC_MASTER_CMD, 1, 0xA0);
        assert_eq!(pic.master.lowest_priority, 3);
        assert!(pic.master.rotate_on_auto_eoi);

        pic.port_write(PIC_MASTER_CMD, 1, 0x11); // ICW1
        assert_eq!(pic.master.lowest_priority, 7);
        assert!(!pic.master.rotate_on_auto_eoi);
    }

    /// Spec: after rotate, fully nested blocking follows the rotated priority order.
    #[test]
    fn rotated_priority_nested_blocking() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        pic.port_write(PIC_MASTER_DATA, 1, 0x00);

        // Make IR0 lowest (IR1 highest) via rotate on non-specific EOI.
        pic.set_irq_line(0, true);
        assert_eq!(pic.poll_irq(), Some(0x08));
        pic.port_write(PIC_MASTER_CMD, 1, 0xA0);
        assert_eq!(pic.master.lowest_priority, 0);

        // Avoid cascade IR2 — exercise master-only lines IR3/IR1/IR4.
        pic.set_irq_line(3, true);
        assert_eq!(pic.poll_irq(), Some(0x0B));
        // IR1 is higher priority than IR3 after rotation and may nest.
        pic.set_irq_line(1, true);
        assert_eq!(pic.poll_irq(), Some(0x09));
        // IR4 is lower than IR3 and stays blocked while IR3 remains in service.
        pic.set_irq_line(4, true);
        assert_eq!(pic.poll_irq(), None);
        assert_eq!(pic.master.irr & (1 << 4), 1 << 4);
    }

    /// Spec: Intel 8259A Specific Rotation — Set Priority Command
    /// (`OCW2 R=1,SL=1,EOI=0` = `0xC0 | L`): L2–L0 becomes lowest priority;
    /// ISR is not cleared.
    #[test]
    fn set_priority_command_assigns_lowest_without_eoi() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        pic.port_write(PIC_MASTER_DATA, 1, 0x00);
        assert_eq!(pic.master.lowest_priority, 7);

        pic.set_irq_line(3, true);
        assert_eq!(pic.poll_irq(), Some(0x0B));
        assert_eq!(pic.master.isr, 1 << 3);

        // Set Priority: IR5 lowest → IR6 highest. No EOI.
        pic.port_write(PIC_MASTER_CMD, 1, 0xC0 | 5);
        assert_eq!(pic.master.lowest_priority, 5);
        assert_eq!(pic.master.isr, 1 << 3); // ISR unchanged

        pic.set_irq_line(3, false);
        pic.set_irq_line(6, true);
        pic.set_irq_line(0, true);
        // IR6 outranks IR0 with bottom = IR5; IR3 still in service blocks lower
        // ranks but IR6 is higher than IR3 in the rotated order
        // (ranks from bottom5: 6,7,0,1,2,3,4 — so IR6 highest, IR3 mid).
        assert_eq!(pic.poll_irq(), Some(0x0E)); // IR6 nests over IR3
        assert_eq!(pic.master.irr, 1 << 0);
    }

    /// Spec: Set Priority alone reorders pending delivery without an in-service bit.
    #[test]
    fn set_priority_reorders_pending_irqs() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        pic.port_write(PIC_MASTER_DATA, 1, 0x00);

        // Default: IR0 highest. Make IR0 lowest via Set Priority (no ISR).
        pic.port_write(PIC_MASTER_CMD, 1, 0xC0); // Set Priority IR0
        assert_eq!(pic.master.lowest_priority, 0);
        assert_eq!(pic.master.isr, 0);

        pic.set_irq_line(0, true);
        pic.set_irq_line(1, true);
        assert_eq!(pic.poll_irq(), Some(0x09)); // IR1 before IR0
        assert_eq!(pic.master.irr, 1 << 0);
    }

    /// Spec: Intel 8259A Specific Rotation — Rotate on Specific EOI
    /// (`OCW2 R=1,SL=1,EOI=1` = `0xE0 | L`): clear ISR bit L and assign L lowest.
    #[test]
    fn rotate_on_specific_eoi_clears_and_rotates() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        pic.port_write(PIC_MASTER_DATA, 1, 0x00);

        pic.set_irq_line(3, true);
        assert_eq!(pic.poll_irq(), Some(0x0B));
        assert_eq!(pic.master.isr, 1 << 3);

        // Rotate on Specific EOI IR3: clear IS3, IR3 becomes lowest → IR4 highest.
        pic.port_write(PIC_MASTER_CMD, 1, 0xE0 | 3);
        assert_eq!(pic.master.isr, 0);
        assert_eq!(pic.master.lowest_priority, 3);

        pic.set_irq_line(3, false);
        pic.set_irq_line(3, true);
        pic.set_irq_line(4, true);
        assert_eq!(pic.poll_irq(), Some(0x0C)); // IR4 before IR3
        assert_eq!(pic.master.irr, 1 << 3);
    }

    /// Spec: Rotate on Specific EOI names the level — wrong L leaves ISR and
    /// still rotates the named bottom (specific EOI of a clear bit is a no-op
    /// on ISR, but L still becomes lowest priority).
    #[test]
    fn rotate_on_specific_eoi_named_level_even_if_isr_clear() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        pic.port_write(PIC_MASTER_DATA, 1, 0x00);

        pic.set_irq_line(1, true);
        assert_eq!(pic.poll_irq(), Some(0x09));
        // Name IR5 (not in service): ISR bit1 stays; bottom becomes IR5.
        pic.port_write(PIC_MASTER_CMD, 1, 0xE0 | 5);
        assert_eq!(pic.master.isr, 1 << 1);
        assert_eq!(pic.master.lowest_priority, 5);
    }

    /// Spec: Specific Rotation on the slave chip (Set Priority + Rotate on
    /// Specific EOI) uses the same OCW2 encoding on `0xA0`.
    #[test]
    fn specific_rotation_on_slave() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        pic.port_write(PIC_MASTER_DATA, 1, 0x00);
        pic.port_write(PIC_SLAVE_DATA, 1, 0x00);

        pic.port_write(PIC_SLAVE_CMD, 1, 0xC0 | 2); // Set Priority: slave IR2 lowest
        assert_eq!(pic.slave.lowest_priority, 2);

        pic.set_irq_line(8, true); // slave IR0 → IRQ8
        pic.set_irq_line(11, true); // slave IR3 → IRQ11; with bottom=IR2, IR3 > IR0
                                    // Master cascade: poll delivers slave vector. IR3 outranks IR0.
        assert_eq!(pic.poll_irq(), Some(0x73)); // base 0x70 + 3
        pic.port_write(PIC_SLAVE_CMD, 1, 0xE0 | 3); // Rotate on Specific EOI IR3
        assert_eq!(pic.slave.isr, 0);
        assert_eq!(pic.slave.lowest_priority, 3);
        pic.port_write(PIC_MASTER_CMD, 1, 0x20); // master non-specific EOI
    }

    /// Spec: Intel 8259A ICW4 bit4 (SFNM) — Special Fully Nested Mode is
    /// programmed on the master; slave ICW4.SFNM is not applied to cascade
    /// nesting (datasheet: SFNM for the master in cascade configurations).
    #[test]
    fn icw4_sfnm_sets_special_fully_nested_on_master() {
        let mut pic = DualPic::new();
        // Master ICW4 = 0x11: µPM | SFNM; slave µPM only.
        init_at_cascade_icw4_roles(&mut pic, 0x11, 0x01);
        assert!(pic.master.special_fully_nested);
        assert!(!pic.slave.special_fully_nested);
        assert_eq!(pic.master.icw4, 0x11);
        assert!(!pic.master.auto_eoi);
    }

    /// Spec: Intel 8259A Special Fully Nested Mode — while a slave interrupt is
    /// in service (master cascade ISR bit set), the slave is not locked out of
    /// the master's priority logic; a higher-priority IR on the same slave is
    /// recognized and delivered without a master EOI first.
    #[test]
    fn sfnm_allows_higher_priority_slave_irq_while_cascade_in_service() {
        let mut pic = DualPic::new();
        init_at_cascade_icw4_roles(&mut pic, 0x11, 0x01);
        pic.port_write(PIC_MASTER_DATA, 1, 0xFB); // unmask master IR2
        pic.port_write(PIC_SLAVE_DATA, 1, 0x00); // unmask all slave IRs

        // Lower-priority slave IR1 (IRQ9) enters service first.
        pic.set_irq_line(9, true);
        assert_eq!(pic.poll_irq(), Some(0x71));
        assert_eq!(pic.master.isr, 1 << 2);
        assert_eq!(pic.slave.isr, 1 << 1);

        // Higher-priority slave IR0 (IRQ8) while cascade still in service.
        pic.set_irq_line(8, true);
        assert_eq!(pic.poll_irq(), Some(0x70));
        assert_eq!(pic.master.isr, 1 << 2); // cascade IS bit remains
        assert_eq!(pic.slave.isr, (1 << 1) | (1 << 0));
    }

    /// Spec: without SFNM, a master cascade ISR bit locks out further requests
    /// from that slave until the master receives EOI (normal fully nested).
    #[test]
    fn without_sfnm_cascade_isr_blocks_further_slave_until_master_eoi() {
        let mut pic = DualPic::new();
        init_at_cascade_icw4_roles(&mut pic, 0x01, 0x01);
        assert!(!pic.master.special_fully_nested);
        pic.port_write(PIC_MASTER_DATA, 1, 0xFB);
        pic.port_write(PIC_SLAVE_DATA, 1, 0x00);

        pic.set_irq_line(9, true);
        assert_eq!(pic.poll_irq(), Some(0x71));
        pic.set_irq_line(8, true);
        // Master IS2 still set → cascade IR locked out (equal priority).
        assert_eq!(pic.poll_irq(), None);

        // Slave EOI clears IR1; master EOI unlocks cascade for the pending IR0.
        pic.port_write(PIC_SLAVE_CMD, 1, 0x20);
        assert_eq!(pic.slave.isr, 0);
        pic.port_write(PIC_MASTER_CMD, 1, 0x20);
        assert_eq!(pic.master.isr, 0);
        assert_eq!(pic.poll_irq(), Some(0x70));
    }

    /// Spec: SFNM still nested-blocks master's own lower-priority IRs while the
    /// cascade line is in service (only equal-priority slave re-entry is opened).
    #[test]
    fn sfnm_still_nested_blocks_lower_priority_master_ir() {
        let mut pic = DualPic::new();
        init_at_cascade_icw4_roles(&mut pic, 0x11, 0x01);
        pic.port_write(PIC_MASTER_DATA, 1, 0x00); // unmask all master
        pic.port_write(PIC_SLAVE_DATA, 1, 0xFD); // unmask slave IR1

        pic.set_irq_line(9, true);
        assert_eq!(pic.poll_irq(), Some(0x71));
        assert_eq!(pic.master.isr, 1 << 2);

        pic.set_irq_line(3, true); // master IR3 — lower than cascade IR2
        assert_eq!(pic.poll_irq(), None);
    }

    /// Spec: SFNM does not relax the slave chip's own fully nested rules — a
    /// lower-priority IR on the slave stays blocked while a higher slave IS bit
    /// is set (master SFNM only unlocks equal-priority cascade recognition).
    #[test]
    fn sfnm_slave_still_fully_nested_blocks_lower_on_slave() {
        let mut pic = DualPic::new();
        init_at_cascade_icw4_roles(&mut pic, 0x11, 0x01);
        pic.port_write(PIC_MASTER_DATA, 1, 0xFB);
        pic.port_write(PIC_SLAVE_DATA, 1, 0x00);

        pic.set_irq_line(9, true); // slave IR1
        assert_eq!(pic.poll_irq(), Some(0x71));
        pic.set_irq_line(10, true); // slave IR2 — lower than IR1 in service
                                    // Slave INT stays low (nested-blocked) → master has nothing to deliver.
        assert_eq!(pic.poll_irq(), None);
        assert_eq!(pic.slave.irr & (1 << 2), 1 << 2);
    }

    /// Spec: ICW1 clears Special Fully Nested Mode (along with other ICW4 state).
    #[test]
    fn icw1_clears_special_fully_nested() {
        let mut pic = DualPic::new();
        init_at_cascade_icw4_roles(&mut pic, 0x11, 0x01);
        assert!(pic.master.special_fully_nested);
        pic.port_write(PIC_MASTER_CMD, 1, 0x11); // new ICW1
        assert!(!pic.master.special_fully_nested);
        assert!(!pic.master.initialized);
    }

    /// PC AT cascade with ICW1.LTIM set on both chips (level-triggered IR inputs).
    fn init_at_cascade_level(pic: &mut DualPic) {
        // ICW1 = 0x19: D4 | LTIM | IC4 (Intel 8259A ICW1 bit3 = level-triggered).
        pic.port_write(PIC_MASTER_CMD, 1, 0x19);
        pic.port_write(PIC_MASTER_DATA, 1, 0x08);
        pic.port_write(PIC_MASTER_DATA, 1, 0x04);
        pic.port_write(PIC_MASTER_DATA, 1, 0x01);
        pic.port_write(PIC_SLAVE_CMD, 1, 0x19);
        pic.port_write(PIC_SLAVE_DATA, 1, 0x70);
        pic.port_write(PIC_SLAVE_DATA, 1, 0x02);
        pic.port_write(PIC_SLAVE_DATA, 1, 0x01);
    }

    /// Spec: Intel 8259A ICW1.LTIM — LTIM=1 programs level interrupt mode.
    #[test]
    fn icw1_ltim_programs_level_triggered() {
        let mut pic = DualPic::new();
        init_at_cascade_level(&mut pic);
        assert!(pic.master.level_triggered);
        assert!(pic.slave.level_triggered);
        assert_eq!(pic.master.icw1, 0x19);
        assert_eq!(pic.slave.icw1, 0x19);
    }

    /// Spec: Intel 8259A level mode — IR high sets IRR; deassert removes the request
    /// (unlike edge mode, where IRR stays latched after a rising edge).
    #[test]
    fn level_triggered_deassert_clears_irr() {
        let mut pic = DualPic::new();
        init_at_cascade_level(&mut pic);
        pic.port_write(PIC_MASTER_DATA, 1, 0xFE); // unmask IR0
        pic.set_irq_line(0, true);
        assert_eq!(pic.master.irr, 0x01);
        pic.set_irq_line(0, false);
        assert_eq!(pic.master.irr, 0);
        assert_eq!(pic.poll_irq(), None);
    }

    /// Spec: Intel 8259A level mode — after INTA, if IR remains high the request
    /// stays pending (IRR reflects the level) so EOI can re-deliver without a
    /// new edge. Datasheet: request must be removed before EOI to avoid a second
    /// interrupt.
    #[test]
    fn level_triggered_ack_while_asserted_keeps_irr_and_redelivers_after_eoi() {
        let mut pic = DualPic::new();
        init_at_cascade_level(&mut pic);
        pic.port_write(PIC_MASTER_DATA, 1, 0xFE); // unmask IR0
        pic.set_irq_line(0, true);
        assert_eq!(pic.poll_irq(), Some(0x08));
        assert_eq!(pic.master.isr, 0x01);
        // Level still high → IRR re-pending while in service.
        assert_eq!(pic.master.irr & 0x01, 0x01);
        // Nested-blocked until EOI.
        assert_eq!(pic.poll_irq(), None);
        pic.port_write(PIC_MASTER_CMD, 1, 0x20); // non-specific EOI
        assert_eq!(pic.master.isr, 0);
        // No new edge: level still asserted → second delivery.
        assert_eq!(pic.poll_irq(), Some(0x08));
    }

    /// Spec: Intel 8259A level mode — removing the IR level before EOI prevents
    /// a second interrupt.
    #[test]
    fn level_triggered_deassert_before_eoi_prevents_redelivery() {
        let mut pic = DualPic::new();
        init_at_cascade_level(&mut pic);
        pic.port_write(PIC_MASTER_DATA, 1, 0xFE);
        pic.set_irq_line(0, true);
        assert_eq!(pic.poll_irq(), Some(0x08));
        pic.set_irq_line(0, false);
        assert_eq!(pic.master.irr & 0x01, 0);
        pic.port_write(PIC_MASTER_CMD, 1, 0x20);
        assert_eq!(pic.poll_irq(), None);
    }

    /// Spec: edge mode (LTIM=0) still requires a rising edge after EOI even if
    /// the IR line stayed high through the first acknowledge.
    #[test]
    fn edge_mode_held_high_does_not_redeliver_after_eoi() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        assert!(!pic.master.level_triggered);
        pic.port_write(PIC_MASTER_DATA, 1, 0xFE);
        pic.set_irq_line(0, true);
        assert_eq!(pic.poll_irq(), Some(0x08));
        assert_eq!(pic.master.irr & 0x01, 0); // edge: IRR cleared on ack
        pic.port_write(PIC_MASTER_CMD, 1, 0x20);
        // Line still high, no new edge → no delivery.
        assert_eq!(pic.poll_irq(), None);
        pic.set_irq_line(0, false);
        pic.set_irq_line(0, true);
        assert_eq!(pic.poll_irq(), Some(0x08));
    }

    /// Spec: level-triggered cascade — slave IR held high re-delivers after dual EOI.
    #[test]
    fn level_triggered_slave_irq_redelivers_while_held() {
        let mut pic = DualPic::new();
        init_at_cascade_level(&mut pic);
        pic.port_write(PIC_MASTER_DATA, 1, 0xFB); // unmask IR2
        pic.port_write(PIC_SLAVE_DATA, 1, 0xFD); // unmask slave IR1
        pic.set_irq_line(9, true);
        assert_eq!(pic.poll_irq(), Some(0x71));
        assert_eq!(pic.slave.irr & (1 << 1), 1 << 1);
        pic.port_write(PIC_SLAVE_CMD, 1, 0x20);
        pic.port_write(PIC_MASTER_CMD, 1, 0x20);
        assert_eq!(pic.poll_irq(), Some(0x71));
    }

    /// Spec: Intel 8259A — IR must remain high until after the falling edge of
    /// the first INTA. If the IR goes low before that, a DEFAULT IR7 occurs and
    /// ISR bit7 is not set (distinguishable from a real IR7).
    #[test]
    fn edge_deassert_before_ack_delivers_default_ir7_without_isr() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        pic.port_write(PIC_MASTER_DATA, 1, 0xF7); // unmask IR3
        pic.set_irq_line(3, true);
        assert_eq!(pic.master.irr, 1 << 3);
        pic.set_irq_line(3, false); // edge: IRR stays latched, pin low
        assert_eq!(pic.master.irr, 1 << 3);
        assert_eq!(pic.poll_irq(), Some(0x0F)); // DEFAULT IR7, base 0x08
        assert_eq!(pic.master.isr & (1 << 7), 0); // ISR bit7 clear
        assert_eq!(pic.master.isr, 0);
        assert_eq!(pic.master.irr & (1 << 3), 0);
    }

    /// Spec: Intel 8259A — a normal IR7 sets ISR bit7; DEFAULT IR7 does not.
    #[test]
    fn real_ir7_sets_isr_bit7() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        pic.port_write(PIC_MASTER_DATA, 1, 0x7F); // unmask IR7
        pic.set_irq_line(7, true);
        assert_eq!(pic.poll_irq(), Some(0x0F));
        assert_eq!(pic.master.isr & (1 << 7), 1 << 7);
    }

    /// Spec: slave DEFAULT IR7 (IRQ15) when slave IR drops before first INTA;
    /// master cascade IS bit is still set (EOI master, not slave).
    #[test]
    fn slave_edge_deassert_before_ack_default_ir7() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        pic.port_write(PIC_MASTER_DATA, 1, 0xFB); // unmask IR2
        pic.port_write(PIC_SLAVE_DATA, 1, 0xFD); // unmask slave IR1
        pic.set_irq_line(9, true);
        pic.set_irq_line(9, false); // edge: slave IRR latched, pin low
        assert_eq!(pic.poll_irq(), Some(0x77)); // slave base 0x70 | 7
        assert_eq!(pic.slave.isr, 0); // DEFAULT: no slave ISR bit7
        assert_eq!(pic.master.isr & (1 << 2), 1 << 2); // cascade still in service
    }

    /// Spec: cascade with empty slave IRR (level slave dropped; master edge-latched
    /// IR2) still yields slave DEFAULT IR7; master IS2 set, slave ISR clear.
    #[test]
    fn cascade_empty_slave_delivers_default_ir7() {
        let mut pic = DualPic::new();
        // Master edge (default ICW1), slave level — deassert clears slave IRR
        // while master's cascade IRR can remain latched from the rising edge.
        init_at_cascade_icw4_roles(&mut pic, 0x01, 0x01);
        // Re-init slave alone with LTIM so only slave is level-triggered.
        pic.port_write(PIC_SLAVE_CMD, 1, 0x19); // ICW1 LTIM + IC4
        pic.port_write(PIC_SLAVE_DATA, 1, 0x70);
        pic.port_write(PIC_SLAVE_DATA, 1, 0x02);
        pic.port_write(PIC_SLAVE_DATA, 1, 0x01);
        assert!(!pic.master.level_triggered);
        assert!(pic.slave.level_triggered);

        pic.port_write(PIC_MASTER_DATA, 1, 0xFB);
        pic.port_write(PIC_SLAVE_DATA, 1, 0xFD);
        pic.set_irq_line(9, true);
        assert!(pic.master.irr & (1 << 2) != 0);
        pic.set_irq_line(9, false); // level slave: IRR cleared; master IR2 pin low, IRR2 latched
        assert_eq!(pic.slave.irr, 0);
        assert!(pic.master.irr & (1 << 2) != 0);
        assert_eq!(pic.poll_irq(), Some(0x77));
        assert_eq!(pic.slave.isr, 0);
        assert_eq!(pic.master.isr & (1 << 2), 1 << 2);
    }

    /// Spec: OCW3 poll shares the INTA acknowledge path — DEFAULT IR7 reports
    /// level 7 with bit7 set in the poll byte, and does not set ISR bit7.
    #[test]
    fn ocw3_poll_default_ir7_without_isr() {
        let mut pic = DualPic::new();
        init_at_cascade(&mut pic);
        pic.port_write(PIC_MASTER_DATA, 1, 0xF7); // unmask IR3
        pic.set_irq_line(3, true);
        pic.set_irq_line(3, false);
        pic.port_write(PIC_MASTER_CMD, 1, 0x0C); // OCW3 P=1
        assert_eq!(pic.port_read(PIC_MASTER_CMD, 1), 0x80 | 7);
        assert_eq!(pic.master.isr, 0);
    }
}
