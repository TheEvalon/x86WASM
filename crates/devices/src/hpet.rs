//! HPET MMIO capability stub (classic base `0xFED0_0000`).
//!
//! Spec: IA-PC HPET (High Precision Event Timers) Specification, Revision 1.0a:
//! - General Capabilities and ID Register at offset `00h` (64-bit RO)
//! - General Configuration Register at offset `10h` (ENABLE_CNF bit 0)
//! - Main Counter Register at offset `F0h`
//!
//! Presence stub for firmware probes. Main counter stays at zero (no freerun).
//! Comparator interrupts, MSI, and ACPI table mapping are out of scope.

/// Classic HPET MMIO base (PC firmware convention / ACPI GAS address).
pub const HPET_DEFAULT_BASE: u64 = 0xFED0_0000;

/// Claimed HPET MMIO window (1 KiB minimum register block).
pub const HPET_WINDOW_SIZE: u64 = 0x400;

/// General Capabilities and ID Register offset.
pub const HPET_REG_CAPS_ID: u32 = 0x00;

/// General Configuration Register offset.
pub const HPET_REG_CONFIG: u32 = 0x10;

/// Main Counter Register offset.
pub const HPET_REG_MAIN_COUNTER: u32 = 0xF0;

/// ENABLE_CNF — General Configuration bit 0.
pub const HPET_CFG_ENABLE: u64 = 1 << 0;

/// Revision ID (CAPS bits 7:0).
pub const HPET_REV_ID: u8 = 0x01;

/// Number of timers minus one (CAPS bits 12:8). `0` → one timer block in CAPS;
/// comparator programming remains unsupported.
pub const HPET_NUM_TIM_CAP: u8 = 0;

/// COUNT_SIZE_CAP clear — main counter treated as 32-bit capable in CAPS.
/// (Actual counter stays zero; freerun is not modeled.)
pub const HPET_COUNT_SIZE_CAP: u64 = 0;

/// Vendor ID (CAPS bits 31:16) — Intel PCI vendor for a PC-compatible stub.
pub const HPET_VENDOR_ID: u16 = 0x8086;

/// Counter clock period in femtoseconds (CAPS bits 63:32).
///
/// Model choice: period for a nominal 14.31818 MHz HPET
/// (`1e15 / 14_318_180 ≈ 69_841_279`). Informational only — the main counter
/// does **not** advance in this stub.
pub const HPET_COUNTER_CLK_PERIOD_FS: u32 = 69_841_279;

/// Composed 64-bit General Capabilities and ID value.
pub const HPET_CAPS_ID_VALUE: u64 = (HPET_REV_ID as u64)
    | ((HPET_NUM_TIM_CAP as u64) << 8)
    | HPET_COUNT_SIZE_CAP
    | ((HPET_VENDOR_ID as u64) << 16)
    | ((HPET_COUNTER_CLK_PERIOD_FS as u64) << 32);

/// HPET presence MMIO (CAPS RO; config store/readback; counter stuck at 0).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HpetMmio {
    base: u64,
    /// General Configuration — only `ENABLE_CNF` retained.
    config: u64,
    /// Main counter — always 0 (honesty: no freerun / step-clock advance).
    main_counter: u64,
    /// Scratch for assembling multi-byte config writes.
    config_scratch: [u8; 8],
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
            config_scratch: [0; 8],
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

    pub fn owns(&self, addr: u64) -> bool {
        (self.base..self.base.saturating_add(HPET_WINDOW_SIZE)).contains(&addr)
    }

    fn read_qword(&self, dword_off: u32) -> u64 {
        match dword_off {
            HPET_REG_CAPS_ID => HPET_CAPS_ID_VALUE,
            // High half of CAPS when reading +4.
            0x04 => HPET_CAPS_ID_VALUE >> 32,
            HPET_REG_CONFIG => self.config,
            0x14 => self.config >> 32,
            HPET_REG_MAIN_COUNTER => self.main_counter,
            0xF4 => self.main_counter >> 32,
            _ => 0,
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
                self.config_scratch = self.config.to_le_bytes();
                let idx = (dword_off - HPET_REG_CONFIG) as usize + lane;
                if idx < 8 {
                    self.config_scratch[idx] = val;
                    // Spec: only ENABLE_CNF (bit0) is retained; other config
                    // bits (LEG_RT_CNF, etc.) are dropped in this stub.
                    let raw = u64::from_le_bytes(self.config_scratch);
                    self.config = raw & HPET_CFG_ENABLE;
                }
            }
            // CAPS is RO; main counter writes accepted but ignored (stays 0).
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
        // Other bits masked off.
        assert!(hpet.mmio_write_u8(HPET_DEFAULT_BASE + u64::from(HPET_REG_CONFIG), 0x03));
        assert_eq!(hpet.config(), HPET_CFG_ENABLE);
    }

    #[test]
    fn main_counter_stays_zero() {
        let mut hpet = HpetMmio::new();
        assert!(hpet.mmio_write_u8(HPET_DEFAULT_BASE + u64::from(HPET_REG_MAIN_COUNTER), 0xFF));
        assert_eq!(hpet.main_counter(), 0);
        assert_eq!(read_u32(&hpet, HPET_REG_MAIN_COUNTER), 0);
    }

    #[test]
    fn reset_clears_config() {
        let mut hpet = HpetMmio::new();
        assert!(hpet.mmio_write_u8(HPET_DEFAULT_BASE + u64::from(HPET_REG_CONFIG), 0x01));
        hpet.reset();
        assert_eq!(hpet, HpetMmio::new());
    }
}
