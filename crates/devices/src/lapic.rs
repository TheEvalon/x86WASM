//! Local APIC MMIO with timer + LVT/spurious stub (default base `0xFEE0_0000`).
//!
//! Spec: Intel SDM Vol. 3A Chapter 10 "Advanced Programmable Interrupt
//! Controller (APIC)":
//! - §10.4.4 — default physical base `FEE0_0000H` (4 KiB window)
//! - §10.4.6 / §10.4.8 — Local APIC ID @ `20H`; Version @ `30H`
//! - §10.4.14 / §10.9 — Spurious Interrupt Vector Register @ `F0H`
//! - §10.5.1 — LVT Timer @ `320H`
//! - §10.5.4 — Timer ICR `380H`, CCR `390H`, DCR `3E0H`
//! - §10.8.3 / §10.8.4 — IRR @ `200H`–`270H`, ISR @ `100H`–`170H` (32-bit
//!   bitmaps; bit *N* = vector *N*)
//! - §10.8.5 — EOI @ `B0H` clears the highest-priority ISR bit
//! - §10.8.6 — TMR @ `180H`–`1F0H` (level vs edge for accepted vectors)
//! - §10.8.3.1 / §10.8.3.2 — TPR @ `80H`, PPR @ `A0H` (firmware probe stub)
//!
//! Round-7: software-enabled APIC + programmed one-shot/periodic timer can
//! latch a **local** interrupt vector via [`LocalApicMmio::take_interrupt`].
//! Round-8: IRR/ISR dword readback + EOI clears the matching ISR bit (and
//! clears the single in-service tracker). Round-10: Trigger Mode Register
//! (TMR) tracks edge vs level on Fixed accept into IRR; Task/Processor
//! Priority (TPR/PPR) store/readback gates `take_interrupt` when the pending
//! vector class is not strictly above PPR. CPUID leaf 1 EDX bit 9 (`APIC`)
//! stays clear — presence ≠ advertised APIC.
//! See `docs/lapic-r7-timer-lvt.md`, `docs/lapic-r8-eoi-isr.md`,
//! `docs/lapic-r10-tmr.md`, `docs/lapic-r10-tpr-ppr.md`.

/// Default Local APIC physical base (SDM Vol. 3A §10.4.4).
pub const LAPIC_DEFAULT_BASE: u64 = 0xFEE0_0000;

/// Local APIC MMIO window size (4 KiB).
pub const LAPIC_WINDOW_SIZE: u64 = 0x1000;

/// Local APIC ID Register offset.
pub const LAPIC_REG_ID: u32 = 0x20;

/// Local APIC Version Register offset.
pub const LAPIC_REG_VERSION: u32 = 0x30;

/// End Of Interrupt Register offset.
pub const LAPIC_REG_EOI: u32 = 0xB0;

/// Task Priority Register offset. Spec: SDM §10.8.3.1.
pub const LAPIC_REG_TPR: u32 = 0x80;

/// Processor Priority Register offset (RO). Spec: SDM §10.8.3.2.
pub const LAPIC_REG_PPR: u32 = 0xA0;

/// In-Service Register base (8×32-bit; vector bitmaps). Spec: SDM §10.8.4.
pub const LAPIC_REG_ISR_BASE: u32 = 0x100;

/// Interrupt Request Register base (8×32-bit). Spec: SDM §10.8.3.
pub const LAPIC_REG_IRR_BASE: u32 = 0x200;

/// Trigger Mode Register base (8×32-bit). Spec: SDM §10.8.6.
pub const LAPIC_REG_TMR_BASE: u32 = 0x180;

/// Spurious Interrupt Vector Register offset (SDM §10.9).
pub const LAPIC_REG_SVR: u32 = 0xF0;

/// LVT Timer Register offset (SDM §10.5.1).
pub const LAPIC_REG_LVT_TIMER: u32 = 0x320;

/// Initial Count Register (timer) offset.
pub const LAPIC_REG_TIMER_ICR: u32 = 0x380;

/// Current Count Register (timer) offset.
pub const LAPIC_REG_TIMER_CCR: u32 = 0x390;

/// Divide Configuration Register offset.
pub const LAPIC_REG_TIMER_DCR: u32 = 0x3E0;

/// Version field (bits 7:0) — `0x14` is in the integrated local-APIC range
/// documented for P6 / Pentium 4 class (SDM Vol. 3A §10.4.8).
pub const LAPIC_VERSION_ID: u8 = 0x14;

/// Max LVT Entry (bits 23:16). Value `3` → LVT entries 0..=3.
pub const LAPIC_MAX_LVT_ENTRY: u8 = 3;

/// Composed Version register value (RO).
pub const LAPIC_VERSION_VALUE: u32 =
    (LAPIC_VERSION_ID as u32) | ((LAPIC_MAX_LVT_ENTRY as u32) << 16);

/// SVR software-enable bit (SDM §10.9 bit 8).
pub const LAPIC_SVR_SW_ENABLE: u32 = 1 << 8;

/// SVR spurious vector field (bits 7:0).
pub const LAPIC_SVR_VECTOR_MASK: u32 = 0xFF;

/// LVT mask bit (bit 16).
pub const LAPIC_LVT_MASK: u32 = 1 << 16;

/// LVT Timer mode bit (bit 17): 0 = one-shot, 1 = periodic.
pub const LAPIC_LVT_TIMER_PERIODIC: u32 = 1 << 17;

/// LVT vector field (bits 7:0).
pub const LAPIC_LVT_VECTOR_MASK: u32 = 0xFF;

/// Local APIC MMIO: ID/Version + SVR + LVT Timer + timer ICR/CCR/DCR stub
/// plus IRR/ISR/TMR bitmap readback for EOI / trigger-mode honesty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalApicMmio {
    base: u64,
    /// APIC ID in bits 31:24 of the ID register (SDM §10.4.6).
    apic_id: u8,
    /// Spurious Interrupt Vector Register.
    svr: u32,
    /// Task Priority Register (bits 7:0). Spec: SDM §10.8.3.1.
    tpr: u32,
    /// LVT Timer register.
    lvt_timer: u32,
    /// Timer initial count.
    timer_icr: u32,
    /// Timer current count.
    timer_ccr: u32,
    /// Divide configuration (only bits 3,1,0 matter).
    timer_dcr: u32,
    /// Accumulator of bus clocks toward one CCR decrement.
    divide_accum: u32,
    /// Latched local interrupt vector awaiting [`Self::take_interrupt`].
    pending_vector: Option<u8>,
    /// In-service vector after accept (cleared by EOI). Mirrors highest ISR.
    in_service: Option<u8>,
    /// Interrupt Request Register — 256 bits as eight little-endian dwords.
    irr: [u32; 8],
    /// In-Service Register — 256 bits as eight little-endian dwords.
    isr: [u32; 8],
    /// Trigger Mode Register — bit set = level, clear = edge (SDM §10.8.6).
    tmr: [u32; 8],
    /// Scratch for byte-lane assembly of dword writes.
    dword_scratch: [u8; 4],
}

impl Default for LocalApicMmio {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalApicMmio {
    pub fn new() -> Self {
        Self {
            base: LAPIC_DEFAULT_BASE,
            apic_id: 0,
            // Spec: SDM §10.4.7.11 — SVR reset: vector often `0xFF`, enable clear.
            svr: 0xFF,
            tpr: 0,
            // LVT Timer reset: masked.
            lvt_timer: LAPIC_LVT_MASK,
            timer_icr: 0,
            timer_ccr: 0,
            timer_dcr: 0, // divide by 2
            divide_accum: 0,
            pending_vector: None,
            in_service: None,
            irr: [0; 8],
            isr: [0; 8],
            tmr: [0; 8],
            dword_scratch: [0; 4],
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn base(&self) -> u64 {
        self.base
    }

    /// APIC ID byte (bits 31:24 of the ID register).
    pub fn apic_id(&self) -> u8 {
        self.apic_id
    }

    pub fn svr(&self) -> u32 {
        self.svr
    }

    /// Task Priority Register value (bits 7:0).
    pub fn tpr(&self) -> u32 {
        self.tpr & 0xFF
    }

    /// Processor Priority Register (RO). Spec: SDM §10.8.3.2.
    ///
    /// `PPR = TPR` when TPR class ≥ highest ISR class; otherwise
    /// `PPR = (ISRV class << 4)` with subclass 0.
    pub fn ppr(&self) -> u32 {
        let tpr = self.tpr & 0xFF;
        let tpr_class = (tpr >> 4) & 0xF;
        match highest_set_bit(&self.isr) {
            Some(isrv) => {
                let isr_class = u32::from(isrv >> 4);
                if tpr_class >= isr_class {
                    tpr
                } else {
                    isr_class << 4
                }
            }
            None => tpr,
        }
    }

    pub fn lvt_timer(&self) -> u32 {
        self.lvt_timer
    }

    pub fn timer_icr(&self) -> u32 {
        self.timer_icr
    }

    pub fn timer_ccr(&self) -> u32 {
        self.timer_ccr
    }

    pub fn timer_dcr(&self) -> u32 {
        self.timer_dcr
    }

    /// Read one IRR dword (index 0..=7 → offsets `200H`..`270H`).
    pub fn irr_dword(&self, index: usize) -> Option<u32> {
        self.irr.get(index).copied()
    }

    /// Read one ISR dword (index 0..=7 → offsets `100H`..`170H`).
    pub fn isr_dword(&self, index: usize) -> Option<u32> {
        self.isr.get(index).copied()
    }

    /// True if vector bit is set in IRR.
    pub fn irr_bit(&self, vector: u8) -> bool {
        bitmap_get(&self.irr, vector)
    }

    /// True if vector bit is set in ISR.
    pub fn isr_bit(&self, vector: u8) -> bool {
        bitmap_get(&self.isr, vector)
    }

    /// Read one TMR dword (index 0..=7 → offsets `180H`..`1F0H`).
    pub fn tmr_dword(&self, index: usize) -> Option<u32> {
        self.tmr.get(index).copied()
    }

    /// True if TMR bit is set (level-triggered accept for that vector).
    ///
    /// Spec: SDM §10.8.6 — bit set = level; clear = edge.
    pub fn tmr_bit(&self, vector: u8) -> bool {
        bitmap_get(&self.tmr, vector)
    }

    /// Software enable from SVR bit 8.
    pub fn software_enabled(&self) -> bool {
        self.svr & LAPIC_SVR_SW_ENABLE != 0
    }

    pub fn owns(&self, addr: u64) -> bool {
        (self.base..self.base.saturating_add(LAPIC_WINDOW_SIZE)).contains(&addr)
    }

    /// Divide value from DCR bits 3/1/0 (SDM §10.5.4).
    pub fn timer_divide_value(dcr: u32) -> u32 {
        let key = ((dcr & 0x8) >> 1) | (dcr & 0x3);
        match key {
            0b000 => 2,
            0b001 => 4,
            0b010 => 8,
            0b011 => 16,
            0b100 => 32,
            0b101 => 64,
            0b110 => 128,
            _ => 1, // 0b111
        }
    }

    fn id_value(&self) -> u32 {
        u32::from(self.apic_id) << 24
    }

    fn read_dword(&self, off: u32) -> u32 {
        match off {
            LAPIC_REG_ID => self.id_value(),
            LAPIC_REG_VERSION => LAPIC_VERSION_VALUE,
            LAPIC_REG_TPR => self.tpr & 0xFF,
            LAPIC_REG_PPR => self.ppr(),
            LAPIC_REG_EOI => 0,
            LAPIC_REG_SVR => self.svr,
            LAPIC_REG_LVT_TIMER => self.lvt_timer,
            LAPIC_REG_TIMER_ICR => self.timer_icr,
            LAPIC_REG_TIMER_CCR => self.timer_ccr,
            LAPIC_REG_TIMER_DCR => self.timer_dcr & 0xB, // bits 3,1,0
            o if (LAPIC_REG_ISR_BASE..LAPIC_REG_ISR_BASE + 0x80).contains(&o) && o & 0xF == 0 => {
                let idx = ((o - LAPIC_REG_ISR_BASE) / 0x10) as usize;
                self.isr.get(idx).copied().unwrap_or(0)
            }
            o if (LAPIC_REG_TMR_BASE..LAPIC_REG_TMR_BASE + 0x80).contains(&o) && o & 0xF == 0 => {
                let idx = ((o - LAPIC_REG_TMR_BASE) / 0x10) as usize;
                self.tmr.get(idx).copied().unwrap_or(0)
            }
            o if (LAPIC_REG_IRR_BASE..LAPIC_REG_IRR_BASE + 0x80).contains(&o) && o & 0xF == 0 => {
                let idx = ((o - LAPIC_REG_IRR_BASE) / 0x10) as usize;
                self.irr.get(idx).copied().unwrap_or(0)
            }
            _ => 0,
        }
    }

    /// True if a local interrupt is latched and awaiting accept.
    ///
    /// Spec: SDM §10.8.3.1 — pending Fixed vectors with priority class ≤ PPR
    /// class are inhibited (remain latched / in IRR).
    pub fn interrupt_pending(&self) -> bool {
        match self.pending_vector {
            Some(v) => self.vector_above_ppr(v),
            None => false,
        }
    }

    /// Peek the latched local vector, if any (even when TPR-inhibited).
    pub fn pending_vector(&self) -> Option<u8> {
        self.pending_vector
    }

    /// Accept the latched local interrupt (IRR → ISR) when above PPR.
    ///
    /// Spec: SDM §10.8.3–§10.8.4 — accepting an interrupt clears the IRR bit
    /// and sets the ISR bit; EOI later clears ISR. This stub does not inject
    /// into the CPU interpreter — hosts must deliver. Returns `None` when
    /// TPR/PPR inhibits the pending vector (latch retained).
    pub fn take_interrupt(&mut self) -> Option<u8> {
        let vec = self.pending_vector?;
        if !self.vector_above_ppr(vec) {
            return None;
        }
        let vec = self.pending_vector.take()?;
        bitmap_clear(&mut self.irr, vec);
        bitmap_set(&mut self.isr, vec);
        self.in_service = Some(vec);
        Some(vec)
    }

    /// Priority class of `vector` is strictly above current PPR class.
    fn vector_above_ppr(&self, vector: u8) -> bool {
        let vec_class = u32::from(vector >> 4);
        let ppr_class = (self.ppr() >> 4) & 0xF;
        vec_class > ppr_class
    }

    /// Peek the current in-service vector (set by [`Self::take_interrupt`]).
    pub fn in_service_vector(&self) -> Option<u8> {
        self.in_service
    }

    /// Software EOI helper — same as writing the EOI register.
    ///
    /// Returns the vector whose ISR bit was cleared, if any.
    pub fn eoi(&mut self) -> Option<u8> {
        let vec = self
            .in_service
            .take()
            .or_else(|| highest_set_bit(&self.isr));
        if let Some(v) = vec {
            bitmap_clear(&mut self.isr, v);
        }
        vec
    }

    /// Latch a Fixed-mode vector from the I/O APIC (software-enable gated).
    ///
    /// Edge-triggered accept (clears TMR bit). Prefer
    /// [`Self::inject_fixed_trigger`] when the RTE trigger mode is known.
    ///
    /// Spec: SDM §10.8 / 82093AA Fixed delivery. Returns `true` when newly
    /// latched. Does not overwrite an already-pending vector.
    pub fn inject_fixed(&mut self, vector: u8) -> bool {
        self.inject_fixed_trigger(vector, false)
    }

    /// Latch a Fixed-mode vector and record TMR for the trigger mode.
    ///
    /// Spec: SDM §10.8.6 — on acceptance into IRR, TMR bit is set for level
    /// and cleared for edge. Returns `true` when newly latched.
    pub fn inject_fixed_trigger(&mut self, vector: u8, level: bool) -> bool {
        if !self.software_enabled() {
            return false;
        }
        if self.pending_vector.is_some() {
            return false;
        }
        bitmap_set(&mut self.irr, vector);
        if level {
            bitmap_set(&mut self.tmr, vector);
        } else {
            bitmap_clear(&mut self.tmr, vector);
        }
        self.pending_vector = Some(vector);
        true
    }

    fn fire_timer_interrupt(&mut self) {
        if !self.software_enabled() {
            return;
        }
        if self.lvt_timer & LAPIC_LVT_MASK != 0 {
            return;
        }
        let vector = (self.lvt_timer & LAPIC_LVT_VECTOR_MASK) as u8;
        // Soft-priority floor: vectors 0..=15 are reserved; still latch for
        // honesty when software programmed them (tests may use ≥0x20).
        // Local APIC timer is edge-triggered (SDM §10.5.1) → clear TMR.
        if self.pending_vector.is_none() {
            bitmap_set(&mut self.irr, vector);
            bitmap_clear(&mut self.tmr, vector);
            self.pending_vector = Some(vector);
        }
    }

    /// Advance the local APIC timer by `bus_clocks` (host-driven).
    ///
    /// Returns `true` if this tick newly latched a local interrupt.
    /// Not wired to the machine step clock or CPU INTR pin automatically.
    pub fn tick_timer(&mut self, bus_clocks: u64) -> bool {
        if bus_clocks == 0 || self.timer_ccr == 0 {
            return false;
        }
        let divide = Self::timer_divide_value(self.timer_dcr);
        let mut clocks = bus_clocks;
        let mut fired = false;
        while clocks > 0 && self.timer_ccr > 0 {
            let need = u64::from(divide.saturating_sub(self.divide_accum));
            if clocks < need {
                self.divide_accum += clocks as u32;
                break;
            }
            clocks -= need;
            self.divide_accum = 0;
            self.timer_ccr -= 1;
            if self.timer_ccr == 0 {
                let before = self.pending_vector;
                self.fire_timer_interrupt();
                if before.is_none() && self.pending_vector.is_some() {
                    fired = true;
                }
                if self.lvt_timer & LAPIC_LVT_TIMER_PERIODIC != 0 && self.timer_icr != 0 {
                    self.timer_ccr = self.timer_icr;
                } else {
                    break;
                }
            }
        }
        fired
    }

    fn write_dword(&mut self, off: u32, value: u32) {
        match off {
            LAPIC_REG_ID => {
                self.apic_id = ((value >> 24) & 0xFF) as u8;
            }
            LAPIC_REG_TPR => {
                // Spec: SDM §10.8.3.1 — bits 7:0 are the task priority.
                self.tpr = value & 0xFF;
            }
            LAPIC_REG_PPR => {
                // RO — claimed, ignored.
            }
            LAPIC_REG_EOI => {
                // Spec: SDM §10.8.5 — write to EOI clears the highest-priority
                // ISR bit. This stub clears the tracked in-service vector's
                // ISR bit (single outstanding interrupt model).
                let _ = self.eoi();
            }
            LAPIC_REG_SVR => {
                // Retain vector + software enable; other SVR bits dropped.
                self.svr = value & (LAPIC_SVR_VECTOR_MASK | LAPIC_SVR_SW_ENABLE);
            }
            LAPIC_REG_LVT_TIMER => {
                // Retain vector, mask, timer mode.
                self.lvt_timer =
                    value & (LAPIC_LVT_VECTOR_MASK | LAPIC_LVT_MASK | LAPIC_LVT_TIMER_PERIODIC);
            }
            LAPIC_REG_TIMER_ICR => {
                self.timer_icr = value;
                self.timer_ccr = value;
                self.divide_accum = 0;
            }
            LAPIC_REG_TIMER_CCR => {
                // CCR is read-only architecturally; accept write, ignore.
            }
            LAPIC_REG_TIMER_DCR => {
                self.timer_dcr = value & 0xB;
                self.divide_accum = 0;
            }
            _ => {}
        }
    }

    /// Byte read within the claimed window, or `None` if unclaimed.
    pub fn mmio_read_u8(&self, addr: u64) -> Option<u8> {
        if !self.owns(addr) {
            return None;
        }
        let off = (addr - self.base) as u32;
        let dword_off = off & !3;
        let lane = (off & 3) as usize;
        Some(self.read_dword(dword_off).to_le_bytes()[lane])
    }

    /// Byte write within the claimed window.
    pub fn mmio_write_u8(&mut self, addr: u64, val: u8) -> bool {
        if !self.owns(addr) {
            return false;
        }
        let off = (addr - self.base) as u32;
        let dword_off = off & !3;
        let lane = (off & 3) as usize;
        self.dword_scratch = self.read_dword(dword_off).to_le_bytes();
        self.dword_scratch[lane] = val;
        self.write_dword(dword_off, u32::from_le_bytes(self.dword_scratch));
        true
    }
}

fn bitmap_get(bits: &[u32; 8], vector: u8) -> bool {
    let idx = (vector / 32) as usize;
    let bit = vector % 32;
    bits[idx] & (1u32 << bit) != 0
}

fn bitmap_set(bits: &mut [u32; 8], vector: u8) {
    let idx = (vector / 32) as usize;
    let bit = vector % 32;
    bits[idx] |= 1u32 << bit;
}

fn bitmap_clear(bits: &mut [u32; 8], vector: u8) {
    let idx = (vector / 32) as usize;
    let bit = vector % 32;
    bits[idx] &= !(1u32 << bit);
}

/// Highest set vector bit in an 8-dword APIC bitmap, or `None`.
fn highest_set_bit(bits: &[u32; 8]) -> Option<u8> {
    for idx in (0..8).rev() {
        let word = bits[idx];
        if word != 0 {
            let bit = 31 - word.leading_zeros();
            return Some((idx as u8) * 32 + bit as u8);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_u32(lapic: &LocalApicMmio, off: u32) -> u32 {
        let mut b = [0u8; 4];
        for i in 0..4u64 {
            b[i as usize] = lapic
                .mmio_read_u8(LAPIC_DEFAULT_BASE + u64::from(off) + i)
                .unwrap();
        }
        u32::from_le_bytes(b)
    }

    fn write_u32(lapic: &mut LocalApicMmio, off: u32, value: u32) {
        for (i, byte) in value.to_le_bytes().into_iter().enumerate() {
            assert!(lapic.mmio_write_u8(LAPIC_DEFAULT_BASE + u64::from(off) + i as u64, byte));
        }
    }

    /// Spec: SDM Vol. 3A §10.4.4 / §10.4.8 — ID=0, Version `0x14` / MaxLVT=3.
    #[test]
    fn id_and_version_presence_defaults() {
        let lapic = LocalApicMmio::new();
        assert!(lapic.owns(LAPIC_DEFAULT_BASE));
        assert!(lapic.owns(LAPIC_DEFAULT_BASE + 0xFFF));
        assert!(!lapic.owns(LAPIC_DEFAULT_BASE + 0x1000));
        assert!(!lapic.owns(0xFED0_0000));
        assert_eq!(read_u32(&lapic, LAPIC_REG_ID), 0);
        assert_eq!(read_u32(&lapic, LAPIC_REG_VERSION), LAPIC_VERSION_VALUE);
        assert_eq!(LAPIC_VERSION_VALUE as u8, LAPIC_VERSION_ID);
        assert_eq!((LAPIC_VERSION_VALUE >> 16) as u8, LAPIC_MAX_LVT_ENTRY);
        assert_eq!(lapic.svr(), 0xFF);
        assert_eq!(lapic.lvt_timer() & LAPIC_LVT_MASK, LAPIC_LVT_MASK);
    }

    /// Spec: SDM §10.4.6 — ID bits 31:24 are writable.
    #[test]
    fn id_bits_31_24_store_readback() {
        let mut lapic = LocalApicMmio::new();
        assert!(lapic.mmio_write_u8(LAPIC_DEFAULT_BASE + u64::from(LAPIC_REG_ID) + 3, 0x02));
        assert_eq!(lapic.apic_id(), 0x02);
        assert_eq!(read_u32(&lapic, LAPIC_REG_ID), 0x0200_0000);
        assert_eq!(read_u32(&lapic, LAPIC_REG_VERSION), LAPIC_VERSION_VALUE);
    }

    #[test]
    fn unimplemented_offsets_read_zero_writes_claimed() {
        let mut lapic = LocalApicMmio::new();
        assert_eq!(lapic.mmio_read_u8(LAPIC_DEFAULT_BASE + 0x280), Some(0)); // ESR
        assert!(lapic.mmio_write_u8(LAPIC_DEFAULT_BASE + 0x280, 0x00));
    }

    #[test]
    fn reset_restores_defaults() {
        let mut lapic = LocalApicMmio::new();
        assert!(lapic.mmio_write_u8(LAPIC_DEFAULT_BASE + u64::from(LAPIC_REG_ID) + 3, 0x07));
        write_u32(&mut lapic, LAPIC_REG_SVR, LAPIC_SVR_SW_ENABLE | 0x20);
        lapic.reset();
        assert_eq!(lapic, LocalApicMmio::new());
    }

    /// Spec: SDM §10.5.4 — DCR divide encodings.
    #[test]
    fn dcr_divide_encodings() {
        assert_eq!(LocalApicMmio::timer_divide_value(0b0000), 2);
        assert_eq!(LocalApicMmio::timer_divide_value(0b0001), 4);
        assert_eq!(LocalApicMmio::timer_divide_value(0b0010), 8);
        assert_eq!(LocalApicMmio::timer_divide_value(0b0011), 16);
        assert_eq!(LocalApicMmio::timer_divide_value(0b1000), 32);
        assert_eq!(LocalApicMmio::timer_divide_value(0b1001), 64);
        assert_eq!(LocalApicMmio::timer_divide_value(0b1010), 128);
        assert_eq!(LocalApicMmio::timer_divide_value(0b1011), 1);
    }

    /// Spec: SDM §10.5.4 / §10.5.1 — one-shot ICR→CCR countdown raises LVT vector.
    #[test]
    fn oneshot_timer_latches_lvt_vector() {
        let mut lapic = LocalApicMmio::new();
        write_u32(&mut lapic, LAPIC_REG_SVR, LAPIC_SVR_SW_ENABLE | 0xFF);
        write_u32(&mut lapic, LAPIC_REG_TIMER_DCR, 0b1011); // divide by 1
        write_u32(&mut lapic, LAPIC_REG_LVT_TIMER, 0x40); // vector 0x40, unmasked, one-shot
        write_u32(&mut lapic, LAPIC_REG_TIMER_ICR, 3);
        assert_eq!(lapic.timer_ccr(), 3);
        assert!(!lapic.interrupt_pending());

        assert!(!lapic.tick_timer(2));
        assert_eq!(lapic.timer_ccr(), 1);
        assert!(lapic.tick_timer(1));
        assert_eq!(lapic.timer_ccr(), 0);
        assert_eq!(lapic.pending_vector(), Some(0x40));
        assert_eq!(lapic.take_interrupt(), Some(0x40));
        assert!(!lapic.interrupt_pending());

        // EOI clears in-service; one-shot stays at 0.
        write_u32(&mut lapic, LAPIC_REG_EOI, 0);
        assert!(!lapic.tick_timer(10));
        assert_eq!(lapic.timer_ccr(), 0);
    }

    /// Spec: SDM §10.5.1 — periodic mode reloads CCR from ICR.
    #[test]
    fn periodic_timer_reloads_and_refires() {
        let mut lapic = LocalApicMmio::new();
        write_u32(&mut lapic, LAPIC_REG_SVR, LAPIC_SVR_SW_ENABLE | 0xFF);
        write_u32(&mut lapic, LAPIC_REG_TIMER_DCR, 0b1011);
        write_u32(
            &mut lapic,
            LAPIC_REG_LVT_TIMER,
            0x41 | LAPIC_LVT_TIMER_PERIODIC,
        );
        write_u32(&mut lapic, LAPIC_REG_TIMER_ICR, 2);
        assert!(lapic.tick_timer(2));
        assert_eq!(lapic.take_interrupt(), Some(0x41));
        write_u32(&mut lapic, LAPIC_REG_EOI, 0);
        assert_eq!(lapic.timer_ccr(), 2);
        assert!(lapic.tick_timer(2));
        assert_eq!(lapic.pending_vector(), Some(0x41));
    }

    /// Spec: SDM §10.9 / §10.5.1 — masked LVT or software-disabled APIC blocks delivery.
    #[test]
    fn mask_and_software_enable_gate_delivery() {
        let mut lapic = LocalApicMmio::new();
        write_u32(&mut lapic, LAPIC_REG_TIMER_DCR, 0b1011);
        write_u32(&mut lapic, LAPIC_REG_LVT_TIMER, 0x40); // unmasked but SVR enable off
        write_u32(&mut lapic, LAPIC_REG_TIMER_ICR, 1);
        assert!(!lapic.tick_timer(1));
        assert!(!lapic.interrupt_pending());

        write_u32(&mut lapic, LAPIC_REG_SVR, LAPIC_SVR_SW_ENABLE | 0xFF);
        write_u32(&mut lapic, LAPIC_REG_LVT_TIMER, 0x40 | LAPIC_LVT_MASK);
        write_u32(&mut lapic, LAPIC_REG_TIMER_ICR, 1);
        assert!(!lapic.tick_timer(1));
        assert!(!lapic.interrupt_pending());
    }

    /// Spec: SDM §10.8.3–§10.8.5 — IRR set on latch, ISR on accept, EOI clears ISR.
    #[test]
    fn irr_isr_readback_and_eoi_clears_isr_bit() {
        let mut lapic = LocalApicMmio::new();
        write_u32(&mut lapic, LAPIC_REG_SVR, LAPIC_SVR_SW_ENABLE | 0xFF);
        assert!(lapic.inject_fixed(0x55));
        assert!(lapic.irr_bit(0x55));
        // IRR dword index 2 covers vectors 64..=95; bit 21 = 0x55.
        assert_eq!(read_u32(&lapic, LAPIC_REG_IRR_BASE + 0x20), 1 << 21);
        assert_eq!(read_u32(&lapic, LAPIC_REG_ISR_BASE + 0x20), 0);

        assert_eq!(lapic.take_interrupt(), Some(0x55));
        assert!(!lapic.irr_bit(0x55));
        assert!(lapic.isr_bit(0x55));
        assert_eq!(read_u32(&lapic, LAPIC_REG_IRR_BASE + 0x20), 0);
        assert_eq!(read_u32(&lapic, LAPIC_REG_ISR_BASE + 0x20), 1 << 21);

        write_u32(&mut lapic, LAPIC_REG_EOI, 0);
        assert!(!lapic.isr_bit(0x55));
        assert_eq!(read_u32(&lapic, LAPIC_REG_ISR_BASE + 0x20), 0);
        // ISR/IRR are read-only; writes are claimed but ignored.
        assert!(lapic.mmio_write_u8(LAPIC_DEFAULT_BASE + u64::from(LAPIC_REG_ISR_BASE), 0xFF));
        assert_eq!(read_u32(&lapic, LAPIC_REG_ISR_BASE), 0);
    }

    /// Spec: SDM §10.5 / §10.8 — timer fire sets IRR; accept moves to ISR.
    #[test]
    fn timer_path_sets_irr_then_isr() {
        let mut lapic = LocalApicMmio::new();
        write_u32(&mut lapic, LAPIC_REG_SVR, LAPIC_SVR_SW_ENABLE | 0xFF);
        write_u32(&mut lapic, LAPIC_REG_TIMER_DCR, 0b1011);
        write_u32(&mut lapic, LAPIC_REG_LVT_TIMER, 0x40);
        write_u32(&mut lapic, LAPIC_REG_TIMER_ICR, 1);
        assert!(lapic.tick_timer(1));
        assert!(lapic.irr_bit(0x40));
        assert!(!lapic.isr_bit(0x40));
        assert_eq!(lapic.take_interrupt(), Some(0x40));
        assert!(!lapic.irr_bit(0x40));
        assert!(lapic.isr_bit(0x40));
        write_u32(&mut lapic, LAPIC_REG_EOI, 0);
        assert!(!lapic.isr_bit(0x40));
    }

    /// Spec: SDM Vol. 3A §10.8.6 — TMR set on level accept into IRR, clear on
    /// edge; EOI does not invent or clear TMR bits; MMIO writes are ignored.
    #[test]
    fn tmr_tracks_edge_vs_level_accept_eoi_unchanged() {
        let mut lapic = LocalApicMmio::new();
        write_u32(&mut lapic, LAPIC_REG_SVR, LAPIC_SVR_SW_ENABLE | 0xFF);

        // Edge Fixed: TMR bit clear.
        assert!(lapic.inject_fixed_trigger(0x55, false));
        assert!(!lapic.tmr_bit(0x55));
        assert_eq!(read_u32(&lapic, LAPIC_REG_TMR_BASE + 0x20), 0);
        assert_eq!(lapic.take_interrupt(), Some(0x55));
        write_u32(&mut lapic, LAPIC_REG_EOI, 0);
        assert!(!lapic.tmr_bit(0x55));
        assert_eq!(read_u32(&lapic, LAPIC_REG_TMR_BASE + 0x20), 0);

        // Level Fixed: TMR bit set (dword index 2, bit 21 = vector 0x55).
        assert!(lapic.inject_fixed_trigger(0x55, true));
        assert!(lapic.tmr_bit(0x55));
        assert_eq!(read_u32(&lapic, LAPIC_REG_TMR_BASE + 0x20), 1 << 21);
        assert_eq!(lapic.take_interrupt(), Some(0x55));
        // Still set after accept into ISR.
        assert!(lapic.tmr_bit(0x55));
        write_u32(&mut lapic, LAPIC_REG_EOI, 0);
        // EOI must not invent or clear TMR.
        assert!(lapic.tmr_bit(0x55));
        assert_eq!(read_u32(&lapic, LAPIC_REG_TMR_BASE + 0x20), 1 << 21);

        // Software writes to TMR are claimed and ignored.
        write_u32(&mut lapic, LAPIC_REG_TMR_BASE + 0x20, 0);
        assert_eq!(read_u32(&lapic, LAPIC_REG_TMR_BASE + 0x20), 1 << 21);

        // Re-accept as edge clears the prior level bit.
        assert!(lapic.inject_fixed_trigger(0x55, false));
        assert!(!lapic.tmr_bit(0x55));
    }

    /// Spec: SDM §10.8.3.1 / §10.8.3.2 — TPR store/readback; PPR follows TPR
    /// when no ISR; pending Fixed below/equal PPR class is inhibited.
    #[test]
    fn tpr_ppr_store_readback_and_masks_pending() {
        let mut lapic = LocalApicMmio::new();
        write_u32(&mut lapic, LAPIC_REG_SVR, LAPIC_SVR_SW_ENABLE | 0xFF);
        write_u32(&mut lapic, LAPIC_REG_TPR, 0x41);
        assert_eq!(read_u32(&lapic, LAPIC_REG_TPR), 0x41);
        assert_eq!(lapic.tpr(), 0x41);
        // No ISR → PPR mirrors TPR.
        assert_eq!(read_u32(&lapic, LAPIC_REG_PPR), 0x41);
        assert_eq!(lapic.ppr(), 0x41);
        // PPR writes are ignored.
        write_u32(&mut lapic, LAPIC_REG_PPR, 0x00);
        assert_eq!(read_u32(&lapic, LAPIC_REG_PPR), 0x41);

        // Vector 0x20 class 2 ≤ TPR class 4 → inhibited.
        assert!(lapic.inject_fixed(0x20));
        assert!(lapic.irr_bit(0x20));
        assert_eq!(lapic.pending_vector(), Some(0x20));
        assert!(!lapic.interrupt_pending());
        assert!(lapic.take_interrupt().is_none());
        assert!(lapic.irr_bit(0x20));

        // Lower TPR so class 2 > class 1 → accept.
        write_u32(&mut lapic, LAPIC_REG_TPR, 0x10);
        assert!(lapic.interrupt_pending());
        assert_eq!(lapic.take_interrupt(), Some(0x20));
        assert!(lapic.isr_bit(0x20));
        // With ISR class 2 and TPR class 1 → PPR = ISRV class << 4.
        assert_eq!(lapic.ppr(), 0x20);
        write_u32(&mut lapic, LAPIC_REG_EOI, 0);
        assert_eq!(lapic.ppr(), 0x10);
    }
}
