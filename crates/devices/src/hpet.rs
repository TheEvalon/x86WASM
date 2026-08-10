//! HPET MMIO with one comparator IRQ stub (classic base `0xFED0_0000`).
//!
//! Spec: IA-PC HPET (High Precision Event Timers) Specification, Revision 1.0a:
//! - General Capabilities and ID Register at offset `00h` (64-bit RO)
//! - General Configuration Register at offset `10h` (`ENABLE_CNF` bit 0)
//! - General Interrupt Status Register at offset `20h` (level clear-by-write-1)
//! - Main Counter Register at offset `F0h`
//! - Timer 0 Configuration/Capability at `100h`, Comparator at `108h`
//!
//! Round-7: Timer 0 can raise a **device-level** interrupt latch when the main
//! counter reaches the comparator while globally and per-timer enabled. Hosts
//! advance the counter via [`HpetMmio::advance_main_counter`] (not step-clock).
//! Round-8: Timer 0 periodic `Tn_VAL_SET_CNF` sequences update the period on
//! the next comparator write (HPET 1.0a), then re-arm comparator = main+period
//! on each fire. Round-10: host `Machine::advance_hpet_ioapic` drives I/O APIC
//! GSI (default IRQ2) from [`HpetMmio::irq_line`] / [`HpetMmio::ioapic_gsi`] —
//! see `docs/hpet-r7-comparator-irq.md`, `docs/hpet-r8-periodic.md`,
//! `docs/hpet-r10-ioapic-wire.md`.

/// Classic HPET MMIO base (PC firmware convention / ACPI GAS address).
pub const HPET_DEFAULT_BASE: u64 = 0xFED0_0000;

/// Claimed HPET MMIO window (1 KiB minimum register block).
pub const HPET_WINDOW_SIZE: u64 = 0x400;

/// General Capabilities and ID Register offset.
pub const HPET_REG_CAPS_ID: u32 = 0x00;

/// General Configuration Register offset.
pub const HPET_REG_CONFIG: u32 = 0x10;

/// General Interrupt Status Register offset.
pub const HPET_REG_INTR_STATUS: u32 = 0x20;

/// Main Counter Register offset.
pub const HPET_REG_MAIN_COUNTER: u32 = 0xF0;

/// Timer 0 Configuration and Capability Register offset.
pub const HPET_REG_T0_CONFIG: u32 = 0x100;

/// Timer 0 Comparator Value Register offset.
pub const HPET_REG_T0_COMPARATOR: u32 = 0x108;

/// `ENABLE_CNF` — General Configuration bit 0.
pub const HPET_CFG_ENABLE: u64 = 1 << 0;

/// Timer n Interrupt Type (`Tn_INT_TYPE_CNF`) — bit 1 (1 = level).
pub const HPET_TN_INT_TYPE: u64 = 1 << 1;

/// Timer n Interrupt Enable (`Tn_INT_ENB_CNF`) — bit 2.
pub const HPET_TN_INT_ENB: u64 = 1 << 2;

/// Timer n Type (`Tn_TYPE_CNF`) — bit 3 (1 = periodic).
pub const HPET_TN_TYPE_PERIODIC: u64 = 1 << 3;

/// Timer n Periodic Interrupt Capable (`Tn_PER_INT_CAP`) — bit 4 (RO).
pub const HPET_TN_PER_INT_CAP: u64 = 1 << 4;

/// Timer n Value Set (`Tn_VAL_SET_CNF`) — bit 6 (W1; not retained).
pub const HPET_TN_VAL_SET: u64 = 1 << 6;

/// Timer n Interrupt Route field (`Tn_INT_ROUTE_CNF`) — bits 13:9.
pub const HPET_TN_INT_ROUTE_SHIFT: u32 = 9;

/// Mask for `Tn_INT_ROUTE_CNF`.
pub const HPET_TN_INT_ROUTE_MASK: u64 = 0x3E00;

/// Revision ID (CAPS bits 7:0).
pub const HPET_REV_ID: u8 = 0x01;

/// Number of timers minus one (CAPS bits 12:8). `0` → one timer (Timer 0).
pub const HPET_NUM_TIM_CAP: u8 = 0;

/// `COUNT_SIZE_CAP` clear — main counter treated as 32-bit capable in CAPS.
pub const HPET_COUNT_SIZE_CAP: u64 = 0;

/// Vendor ID (CAPS bits 31:16) — Intel PCI vendor for a PC-compatible stub.
pub const HPET_VENDOR_ID: u16 = 0x8086;

/// Counter clock period in femtoseconds (CAPS bits 63:32).
///
/// Model choice: period for a nominal 14.31818 MHz HPET
/// (`1e15 / 14_318_180 ≈ 69_841_279`). Informational for CAPS; hosts advance
/// the main counter in abstract ticks via [`HpetMmio::advance_main_counter`].
pub const HPET_COUNTER_CLK_PERIOD_FS: u32 = 69_841_279;

/// Composed 64-bit General Capabilities and ID value.
pub const HPET_CAPS_ID_VALUE: u64 = (HPET_REV_ID as u64)
    | ((HPET_NUM_TIM_CAP as u64) << 8)
    | HPET_COUNT_SIZE_CAP
    | ((HPET_VENDOR_ID as u64) << 16)
    | ((HPET_COUNTER_CLK_PERIOD_FS as u64) << 32);

/// Timer 0 interrupt routing capability (RO bits 63:32 of T0 config).
///
/// Spec: HPET 1.0a — bit *N* set ⇒ Timer may route to I/O APIC IRQ *N*.
/// This stub advertises IRQ2 only (common non-legacy route; QEMU/SeaBIOS-
/// compatible default GSI). See `docs/hpet-r10-ioapic-wire.md`.
pub const HPET_T0_INT_ROUTE_CAP: u32 = 1 << 2;

/// Default I/O APIC GSI for Timer 0 when `Tn_INT_ROUTE_CNF` is unset or
/// outside [`HPET_T0_INT_ROUTE_CAP`] (IRQ2).
pub const HPET_DEFAULT_IOAPIC_GSI: u8 = 2;

/// RO capability bits always present in Timer 0 config.
pub const HPET_T0_CONFIG_CAPS: u64 = HPET_TN_PER_INT_CAP | ((HPET_T0_INT_ROUTE_CAP as u64) << 32);

/// Writable Timer 0 config bits retained by this stub.
const HPET_T0_CONFIG_WRITABLE: u64 =
    HPET_TN_INT_TYPE | HPET_TN_INT_ENB | HPET_TN_TYPE_PERIODIC | HPET_TN_INT_ROUTE_MASK;

/// 32-bit main-counter / comparator mask (`COUNT_SIZE_CAP` clear).
const HPET_COUNTER_MASK: u64 = 0xFFFF_FFFF;

/// HPET MMIO: CAPS + config + main counter + Timer 0 comparator IRQ stub.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HpetMmio {
    base: u64,
    /// General Configuration — only `ENABLE_CNF` retained.
    config: u64,
    /// Main counter (32-bit modeled; upper bits forced 0).
    main_counter: u64,
    /// General Interrupt Status — bit 0 = Timer 0 (`T0_INT_STS`).
    intr_status: u64,
    /// Timer 0 writable config bits (OR'd with RO caps on read).
    t0_config: u64,
    /// Timer 0 comparator value (32-bit modeled).
    t0_comparator: u64,
    /// Periodic accumulator / next match (used when `Tn_TYPE_CNF` is set).
    t0_periodic_period: u64,
    /// Pending `Tn_VAL_SET_CNF`: next comparator write loads the period.
    t0_val_set_pending: bool,
    /// Edge-triggered IRQ latch (cleared when status is cleared).
    irq_edge_latched: bool,
    /// One-shot: suppress re-fire until comparator is rewritten.
    t0_oneshot_armed: bool,
    /// Scratch for assembling multi-byte writes.
    qword_scratch: [u8; 8],
}

impl Default for HpetMmio {
    fn default() -> Self {
        Self::new()
    }
}

impl HpetMmio {
    pub fn new() -> Self {
        Self {
            base: HPET_DEFAULT_BASE,
            config: 0,
            main_counter: 0,
            intr_status: 0,
            t0_config: 0,
            t0_comparator: 0,
            t0_periodic_period: 0,
            t0_val_set_pending: false,
            irq_edge_latched: false,
            t0_oneshot_armed: true,
            qword_scratch: [0; 8],
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn base(&self) -> u64 {
        self.base
    }

    pub fn config(&self) -> u64 {
        self.config
    }

    pub fn main_counter(&self) -> u64 {
        self.main_counter
    }

    pub fn intr_status(&self) -> u64 {
        self.intr_status
    }

    pub fn t0_comparator(&self) -> u64 {
        self.t0_comparator
    }

    /// Periodic period last loaded via `Tn_VAL_SET_CNF` + comparator write.
    pub fn t0_periodic_period(&self) -> u64 {
        self.t0_periodic_period
    }

    /// Timer 0 config as visible to software (writable bits + RO caps).
    pub fn t0_config(&self) -> u64 {
        (self.t0_config & HPET_T0_CONFIG_WRITABLE) | HPET_T0_CONFIG_CAPS
    }

    /// `Tn_INT_ROUTE_CNF` field (bits 13:9).
    pub fn t0_int_route(&self) -> u8 {
        ((self.t0_config & HPET_TN_INT_ROUTE_MASK) >> HPET_TN_INT_ROUTE_SHIFT) as u8
    }

    /// I/O APIC GSI selected for Timer 0 IRQ delivery.
    ///
    /// Uses programmed `Tn_INT_ROUTE_CNF` when that IRQ is advertised in
    /// [`HPET_T0_INT_ROUTE_CAP`]; otherwise [`HPET_DEFAULT_IOAPIC_GSI`] (2).
    /// Spec: HPET 1.0a route field + common QEMU/SeaBIOS non-legacy IRQ2.
    pub fn ioapic_gsi(&self) -> u8 {
        let route = self.t0_int_route();
        if route < 32 && (HPET_T0_INT_ROUTE_CAP & (1u32 << route)) != 0 {
            route
        } else {
            HPET_DEFAULT_IOAPIC_GSI
        }
    }

    pub fn owns(&self, addr: u64) -> bool {
        (self.base..self.base.saturating_add(HPET_WINDOW_SIZE)).contains(&addr)
    }

    /// Device-level interrupt request.
    ///
    /// Spec: HPET 1.0a — `Tn_INT_ENB_CNF` gates interrupt generation; level
    /// mode follows `Tn_INT_STS`, edge mode latches until status is cleared.
    /// Host wiring (R10) mirrors this onto the I/O APIC GSI from
    /// [`Self::ioapic_gsi`]; the device itself does not touch DualPic.
    pub fn irq_line(&self) -> bool {
        if self.config & HPET_CFG_ENABLE == 0 || self.t0_config & HPET_TN_INT_ENB == 0 {
            return false;
        }
        if self.t0_config & HPET_TN_INT_TYPE != 0 {
            self.intr_status & 1 != 0
        } else {
            self.irq_edge_latched
        }
    }

    /// Advance the main counter by `delta` ticks and evaluate Timer 0.
    ///
    /// Returns `true` if this advance caused a new Timer 0 interrupt event
    /// (status bit newly set / edge latched). When `ENABLE_CNF` is clear the
    /// counter is halted (HPET 1.0a) and this is a no-op. Not driven by the
    /// machine step clock — hosts must call this explicitly.
    pub fn advance_main_counter(&mut self, delta: u64) -> bool {
        if delta == 0 || self.config & HPET_CFG_ENABLE == 0 {
            return false;
        }
        let before = self.main_counter;
        self.main_counter = before.wrapping_add(delta) & HPET_COUNTER_MASK;
        self.eval_timer0(before)
    }

    fn eval_timer0(&mut self, before: u64) -> bool {
        let after = self.main_counter;
        let cmp = self.t0_comparator & HPET_COUNTER_MASK;
        let crossed = if before <= after {
            before < cmp && after >= cmp
        } else {
            // 32-bit wrap: fire if comparator was ahead of `before` or behind `after`.
            before < cmp || after >= cmp
        };
        if !crossed {
            return false;
        }

        let periodic = self.t0_config & HPET_TN_TYPE_PERIODIC != 0;
        if !periodic && !self.t0_oneshot_armed {
            return false;
        }

        let already = self.intr_status & 1 != 0;
        self.intr_status |= 1;
        if self.t0_config & HPET_TN_INT_TYPE == 0 {
            self.irq_edge_latched = true;
        }
        if periodic {
            let period = if self.t0_periodic_period == 0 {
                cmp.max(1)
            } else {
                self.t0_periodic_period
            };
            self.t0_comparator = after.wrapping_add(period) & HPET_COUNTER_MASK;
        } else {
            self.t0_oneshot_armed = false;
        }
        !already || self.t0_config & HPET_TN_INT_TYPE == 0
    }

    fn read_qword(&self, dword_off: u32) -> u64 {
        match dword_off {
            HPET_REG_CAPS_ID => HPET_CAPS_ID_VALUE,
            0x04 => HPET_CAPS_ID_VALUE >> 32,
            HPET_REG_CONFIG => self.config,
            0x14 => self.config >> 32,
            HPET_REG_INTR_STATUS => self.intr_status,
            0x24 => self.intr_status >> 32,
            HPET_REG_MAIN_COUNTER => self.main_counter,
            0xF4 => self.main_counter >> 32,
            HPET_REG_T0_CONFIG => self.t0_config(),
            0x104 => self.t0_config() >> 32,
            HPET_REG_T0_COMPARATOR => self.t0_comparator,
            0x10C => self.t0_comparator >> 32,
            _ => 0,
        }
    }

    fn write_config_byte(&mut self, byte_index: usize, val: u8) {
        self.qword_scratch = self.config.to_le_bytes();
        if byte_index < 8 {
            self.qword_scratch[byte_index] = val;
            let raw = u64::from_le_bytes(self.qword_scratch);
            self.config = raw & HPET_CFG_ENABLE;
        }
    }

    fn write_intr_status_byte(&mut self, byte_index: usize, val: u8) {
        // Spec: write-1-to-clear for level-triggered status bits.
        if byte_index == 0 {
            let clear = u64::from(val);
            self.intr_status &= !clear;
            if clear & 1 != 0 {
                self.irq_edge_latched = false;
            }
        }
    }

    fn write_main_counter_byte(&mut self, byte_index: usize, val: u8) {
        self.qword_scratch = self.main_counter.to_le_bytes();
        if byte_index < 8 {
            self.qword_scratch[byte_index] = val;
            let before = self.main_counter;
            self.main_counter = u64::from_le_bytes(self.qword_scratch) & HPET_COUNTER_MASK;
            // Software write can land exactly on the comparator.
            let _ = self.eval_timer0(before);
        }
    }

    fn write_t0_config_byte(&mut self, byte_index: usize, val: u8) {
        self.qword_scratch = self.t0_config().to_le_bytes();
        if byte_index < 8 {
            self.qword_scratch[byte_index] = val;
            let raw = u64::from_le_bytes(self.qword_scratch);
            let route = (raw & HPET_TN_INT_ROUTE_MASK) >> HPET_TN_INT_ROUTE_SHIFT;
            // Only advertised routes stick; others force 0.
            let route_ok = (HPET_T0_INT_ROUTE_CAP & (1u32 << route)) != 0;
            let mut retained = raw & HPET_T0_CONFIG_WRITABLE;
            if !route_ok {
                retained &= !HPET_TN_INT_ROUTE_MASK;
            }
            // Spec: HPET 1.0a — Tn_VAL_SET_CNF is W1 and not retained; it arms
            // the next comparator write to load the periodic accumulator.
            if raw & HPET_TN_VAL_SET != 0 {
                self.t0_val_set_pending = true;
            }
            self.t0_config = retained;
        }
    }

    fn write_t0_comparator_byte(&mut self, byte_index: usize, val: u8) {
        self.qword_scratch = self.t0_comparator.to_le_bytes();
        if byte_index < 8 {
            self.qword_scratch[byte_index] = val;
            let value = u64::from_le_bytes(self.qword_scratch) & HPET_COUNTER_MASK;
            self.t0_comparator = value;
            // Commit period side effects once per 32-bit low-dword store
            // (byte lanes 0..3 assembled; fire on lane 3) so multi-byte MMIO
            // writes do not snapshot a partial value under VAL_SET.
            if byte_index == 3 {
                if self.t0_val_set_pending && self.t0_config & HPET_TN_TYPE_PERIODIC != 0 {
                    // Spec: VAL_SET + comparator write → period; comparator
                    // already holds the programmed value as next match seed.
                    self.t0_periodic_period = value.max(1);
                    self.t0_val_set_pending = false;
                } else if self.t0_config & HPET_TN_TYPE_PERIODIC != 0
                    && self.t0_periodic_period == 0
                {
                    self.t0_periodic_period = value.max(1);
                }
                self.t0_oneshot_armed = true;
            }
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
        let value = self.read_qword(dword_off) as u32;
        Some(value.to_le_bytes()[lane])
    }

    /// Byte write within the claimed window.
    pub fn mmio_write_u8(&mut self, addr: u64, val: u8) -> bool {
        if !self.owns(addr) {
            return false;
        }
        let off = (addr - self.base) as u32;
        let dword_off = off & !3;
        let lane = (off & 3) as usize;
        match dword_off {
            HPET_REG_CONFIG | 0x14 => {
                let idx = (dword_off - HPET_REG_CONFIG) as usize + lane;
                self.write_config_byte(idx, val);
            }
            HPET_REG_INTR_STATUS | 0x24 => {
                let idx = (dword_off - HPET_REG_INTR_STATUS) as usize + lane;
                self.write_intr_status_byte(idx, val);
            }
            HPET_REG_MAIN_COUNTER | 0xF4 => {
                let idx = (dword_off - HPET_REG_MAIN_COUNTER) as usize + lane;
                self.write_main_counter_byte(idx, val);
            }
            HPET_REG_T0_CONFIG | 0x104 => {
                let idx = (dword_off - HPET_REG_T0_CONFIG) as usize + lane;
                self.write_t0_config_byte(idx, val);
            }
            HPET_REG_T0_COMPARATOR | 0x10C => {
                let idx = (dword_off - HPET_REG_T0_COMPARATOR) as usize + lane;
                self.write_t0_comparator_byte(idx, val);
            }
            // CAPS is RO; other offsets claimed with no side effect.
            _ => {}
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_u32(hpet: &HpetMmio, off: u32) -> u32 {
        let mut b = [0u8; 4];
        for i in 0..4u64 {
            b[i as usize] = hpet
                .mmio_read_u8(HPET_DEFAULT_BASE + u64::from(off) + i)
                .unwrap();
        }
        u32::from_le_bytes(b)
    }

    fn write_u32(hpet: &mut HpetMmio, off: u32, value: u32) {
        for (i, byte) in value.to_le_bytes().into_iter().enumerate() {
            assert!(hpet.mmio_write_u8(HPET_DEFAULT_BASE + u64::from(off) + i as u64, byte));
        }
    }

    /// Spec: HPET 1.0a — CAPS/ID readable; vendor/rev/timer count documented.
    #[test]
    fn caps_id_presence_defaults() {
        let hpet = HpetMmio::new();
        assert!(hpet.owns(HPET_DEFAULT_BASE));
        assert!(!hpet.owns(HPET_DEFAULT_BASE + HPET_WINDOW_SIZE));
        assert_eq!(read_u32(&hpet, HPET_REG_CAPS_ID), HPET_CAPS_ID_VALUE as u32);
        assert_eq!(read_u32(&hpet, 0x04), (HPET_CAPS_ID_VALUE >> 32) as u32);
        assert_eq!(HPET_CAPS_ID_VALUE as u8, HPET_REV_ID);
        assert_eq!(((HPET_CAPS_ID_VALUE >> 8) & 0x1F) as u8, HPET_NUM_TIM_CAP);
        assert_eq!((HPET_CAPS_ID_VALUE >> 16) as u16, HPET_VENDOR_ID);
        assert_eq!(hpet.main_counter(), 0);
    }

    /// Spec: HPET 1.0a — General Configuration ENABLE_CNF store/readback.
    #[test]
    fn config_enable_store_readback() {
        let mut hpet = HpetMmio::new();
        assert!(hpet.mmio_write_u8(HPET_DEFAULT_BASE + u64::from(HPET_REG_CONFIG), 0x01));
        assert_eq!(hpet.config(), HPET_CFG_ENABLE);
        assert_eq!(read_u32(&hpet, HPET_REG_CONFIG), 1);
        assert!(hpet.mmio_write_u8(HPET_DEFAULT_BASE + u64::from(HPET_REG_CONFIG), 0x03));
        assert_eq!(hpet.config(), HPET_CFG_ENABLE);
    }

    #[test]
    fn main_counter_writable_32bit() {
        let mut hpet = HpetMmio::new();
        write_u32(&mut hpet, HPET_REG_MAIN_COUNTER, 0x1234_5678);
        assert_eq!(hpet.main_counter(), 0x1234_5678);
        assert_eq!(read_u32(&hpet, HPET_REG_MAIN_COUNTER), 0x1234_5678);
    }

    #[test]
    fn reset_clears_config() {
        let mut hpet = HpetMmio::new();
        assert!(hpet.mmio_write_u8(HPET_DEFAULT_BASE + u64::from(HPET_REG_CONFIG), 0x01));
        hpet.reset();
        assert_eq!(hpet, HpetMmio::new());
    }

    /// Spec: HPET 1.0a — Timer 0 config RO caps + INT_ENB / comparator.
    #[test]
    fn timer0_config_caps_and_comparator_store() {
        let mut hpet = HpetMmio::new();
        let cfg = hpet.t0_config();
        assert_ne!(cfg & HPET_TN_PER_INT_CAP, 0);
        assert_eq!((cfg >> 32) as u32, HPET_T0_INT_ROUTE_CAP);
        assert_eq!(read_u32(&hpet, 0x104), HPET_T0_INT_ROUTE_CAP);

        // Enable interrupts + route to IRQ2 (advertised).
        write_u32(
            &mut hpet,
            HPET_REG_T0_CONFIG,
            (HPET_TN_INT_ENB | (2 << HPET_TN_INT_ROUTE_SHIFT)) as u32,
        );
        assert_ne!(hpet.t0_config() & HPET_TN_INT_ENB, 0);
        assert_eq!(hpet.t0_int_route(), 2);

        write_u32(&mut hpet, HPET_REG_T0_COMPARATOR, 100);
        assert_eq!(hpet.t0_comparator(), 100);
        assert_eq!(read_u32(&hpet, HPET_REG_T0_COMPARATOR), 100);
    }

    /// Spec: HPET 1.0a — one-shot comparator fires INT_STS / irq_line when enabled.
    #[test]
    fn timer0_oneshot_raises_stub_irq_on_advance() {
        let mut hpet = HpetMmio::new();
        write_u32(&mut hpet, HPET_REG_CONFIG, 1);
        write_u32(
            &mut hpet,
            HPET_REG_T0_CONFIG,
            (HPET_TN_INT_ENB | HPET_TN_INT_TYPE | (2 << HPET_TN_INT_ROUTE_SHIFT)) as u32,
        );
        write_u32(&mut hpet, HPET_REG_T0_COMPARATOR, 50);
        assert!(!hpet.irq_line());

        assert!(hpet.advance_main_counter(50));
        assert_eq!(hpet.main_counter(), 50);
        assert_eq!(hpet.intr_status() & 1, 1);
        assert!(hpet.irq_line());

        // One-shot does not re-fire on further advances without rewriting CMP.
        assert!(!hpet.advance_main_counter(100));
        assert!(hpet.irq_line());

        // Write-1-to-clear status deasserts level IRQ.
        write_u32(&mut hpet, HPET_REG_INTR_STATUS, 1);
        assert_eq!(hpet.intr_status() & 1, 0);
        assert!(!hpet.irq_line());
    }

    /// Spec: HPET 1.0a — periodic type re-arms comparator after fire.
    #[test]
    fn timer0_periodic_rearms_comparator() {
        let mut hpet = HpetMmio::new();
        write_u32(&mut hpet, HPET_REG_CONFIG, 1);
        // VAL_SET + periodic + INT_ENB, then comparator write loads period.
        write_u32(
            &mut hpet,
            HPET_REG_T0_CONFIG,
            (HPET_TN_INT_ENB | HPET_TN_TYPE_PERIODIC | HPET_TN_VAL_SET) as u32,
        );
        write_u32(&mut hpet, HPET_REG_T0_COMPARATOR, 10);
        assert_eq!(hpet.t0_periodic_period(), 10);
        assert_eq!(hpet.t0_comparator(), 10);
        // VAL_SET is not retained in the visible config.
        assert_eq!(hpet.t0_config() & HPET_TN_VAL_SET, 0);

        assert!(hpet.advance_main_counter(10));
        assert!(hpet.irq_line());
        assert_eq!(hpet.t0_comparator(), 20);

        write_u32(&mut hpet, HPET_REG_INTR_STATUS, 1);
        assert!(hpet.advance_main_counter(10));
        assert!(hpet.irq_line());
        assert_eq!(hpet.t0_comparator(), 30);
    }

    /// Spec: HPET 1.0a — after period is set, a normal comparator write changes next match.
    #[test]
    fn timer0_periodic_val_set_then_next_match_write() {
        let mut hpet = HpetMmio::new();
        write_u32(&mut hpet, HPET_REG_CONFIG, 1);
        write_u32(
            &mut hpet,
            HPET_REG_T0_CONFIG,
            (HPET_TN_INT_ENB | HPET_TN_TYPE_PERIODIC | HPET_TN_VAL_SET) as u32,
        );
        write_u32(&mut hpet, HPET_REG_T0_COMPARATOR, 5); // period = 5
                                                         // Without VAL_SET, rewrite comparator to first match at 20.
        write_u32(&mut hpet, HPET_REG_T0_COMPARATOR, 20);
        assert_eq!(hpet.t0_periodic_period(), 5);
        assert_eq!(hpet.t0_comparator(), 20);

        assert!(!hpet.advance_main_counter(19));
        assert!(hpet.advance_main_counter(1));
        assert_eq!(hpet.t0_comparator(), 25); // 20 main + period 5 after fire at 20
    }

    #[test]
    fn irq_gated_by_global_and_timer_enable() {
        let mut hpet = HpetMmio::new();
        write_u32(&mut hpet, HPET_REG_CONFIG, 1);
        write_u32(&mut hpet, HPET_REG_T0_CONFIG, HPET_TN_INT_ENB as u32);
        write_u32(&mut hpet, HPET_REG_T0_COMPARATOR, 1);
        assert!(hpet.advance_main_counter(1));
        assert_eq!(hpet.intr_status() & 1, 1);
        assert!(hpet.irq_line());
        // Clearing Tn_INT_ENB gates irq_line; status bit remains until W1C.
        write_u32(&mut hpet, HPET_REG_T0_CONFIG, 0);
        assert!(!hpet.irq_line());
        assert_eq!(hpet.intr_status() & 1, 1);
        // ENABLE_CNF=0 halts further advances.
        write_u32(&mut hpet, HPET_REG_CONFIG, 0);
        write_u32(&mut hpet, HPET_REG_T0_CONFIG, HPET_TN_INT_ENB as u32);
        write_u32(&mut hpet, HPET_REG_T0_COMPARATOR, 100);
        assert!(!hpet.advance_main_counter(100));
        assert_eq!(hpet.main_counter(), 1);
    }

    /// Spec: HPET 1.0a — `Tn_INT_ROUTE_CNF` must be advertised; else default GSI 2.
    #[test]
    fn ioapic_gsi_defaults_to_irq2_when_route_unset() {
        let mut hpet = HpetMmio::new();
        assert_eq!(hpet.t0_int_route(), 0);
        assert_eq!(hpet.ioapic_gsi(), HPET_DEFAULT_IOAPIC_GSI);
        write_u32(
            &mut hpet,
            HPET_REG_T0_CONFIG,
            (HPET_TN_INT_ENB | (2 << HPET_TN_INT_ROUTE_SHIFT)) as u32,
        );
        assert_eq!(hpet.ioapic_gsi(), 2);
    }
}
