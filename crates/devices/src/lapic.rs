//! Local APIC MMIO identity stub (default base `0xFEE0_0000`).
//!
//! Spec: Intel SDM Vol. 3A Chapter 10 "Advanced Programmable Interrupt
//! Controller (APIC)":
//! - §10.4.4 — default physical base `FEE0_0000H` (4 KiB window)
//! - §10.4.6 / §10.4.8 — Local APIC ID Register at offset `20H` (APIC ID in
//!   bits 31:24); Local APIC Version Register at offset `30H` (version in
//!   bits 7:0; Max LVT Entry in bits 23:16)
//!
//! Presence stub for firmware probes. CPUID leaf 1 EDX bit 9 (`APIC`) stays
//! clear in the interpreter — this window must not advertise a usable APIC.

/// Default Local APIC physical base (SDM Vol. 3A §10.4.4).
pub const LAPIC_DEFAULT_BASE: u64 = 0xFEE0_0000;

/// Local APIC MMIO window size (4 KiB).
pub const LAPIC_WINDOW_SIZE: u64 = 0x1000;

/// Local APIC ID Register offset.
pub const LAPIC_REG_ID: u32 = 0x20;

/// Local APIC Version Register offset.
pub const LAPIC_REG_VERSION: u32 = 0x30;

/// Version field (bits 7:0) — `0x14` is in the integrated local-APIC range
/// documented for P6 / Pentium 4 class (SDM Vol. 3A §10.4.8).
pub const LAPIC_VERSION_ID: u8 = 0x14;

/// Max LVT Entry (bits 23:16). Value `3` → LVT entries 0..=3. Timer / LINT /
/// thermal / perfmon delivery are **not** modeled.
pub const LAPIC_MAX_LVT_ENTRY: u8 = 3;

/// Composed Version register value (RO).
pub const LAPIC_VERSION_VALUE: u32 =
    (LAPIC_VERSION_ID as u32) | ((LAPIC_MAX_LVT_ENTRY as u32) << 16);

/// Local APIC presence MMIO: ID store/readback (bits 31:24), Version RO.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalApicMmio {
    base: u64,
    /// APIC ID in bits 31:24 of the ID register (SDM §10.4.6).
    apic_id: u8,
    /// Scratch for byte-lane assembly of ID writes.
    id_scratch: [u8; 4],
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
            id_scratch: [0; 4],
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

    pub fn owns(&self, addr: u64) -> bool {
        (self.base..self.base.saturating_add(LAPIC_WINDOW_SIZE)).contains(&addr)
    }

    fn id_value(&self) -> u32 {
        u32::from(self.apic_id) << 24
    }

    /// Byte read within the claimed window, or `None` if unclaimed.
    pub fn mmio_read_u8(&self, addr: u64) -> Option<u8> {
        if !self.owns(addr) {
            return None;
        }
        let off = (addr - self.base) as u32;
        let dword_off = off & !3;
        let lane = (off & 3) as usize;
        let value = match dword_off {
            LAPIC_REG_ID => self.id_value(),
            LAPIC_REG_VERSION => LAPIC_VERSION_VALUE,
            // Unimplemented offsets: zeros (EOI/SVR/LVT/ICR not modeled).
            _ => 0,
        };
        Some(value.to_le_bytes()[lane])
    }

    /// Byte write within the claimed window.
    ///
    /// Spec: SDM §10.4.6 — software may program the Local APIC ID field
    /// (bits 31:24). Other offsets accept the write (claimed) with no side
    /// effect in this stub.
    pub fn mmio_write_u8(&mut self, addr: u64, val: u8) -> bool {
        if !self.owns(addr) {
            return false;
        }
        let off = (addr - self.base) as u32;
        let dword_off = off & !3;
        let lane = (off & 3) as usize;
        if dword_off == LAPIC_REG_ID {
            self.id_scratch = self.id_value().to_le_bytes();
            self.id_scratch[lane] = val;
            // Only bits 31:24 are architecturally the APIC ID; lower bytes
            // are reserved / ignored on many implementations.
            self.apic_id = self.id_scratch[3];
        }
        true
    }
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
    }

    /// Spec: SDM §10.4.6 — ID bits 31:24 are writable.
    #[test]
    fn id_bits_31_24_store_readback() {
        let mut lapic = LocalApicMmio::new();
        assert!(lapic.mmio_write_u8(LAPIC_DEFAULT_BASE + u64::from(LAPIC_REG_ID) + 3, 0x02));
        assert_eq!(lapic.apic_id(), 0x02);
        assert_eq!(read_u32(&lapic, LAPIC_REG_ID), 0x0200_0000);
        // Version remains RO.
        assert_eq!(read_u32(&lapic, LAPIC_REG_VERSION), LAPIC_VERSION_VALUE);
    }

    #[test]
    fn unimplemented_offsets_read_zero_writes_claimed() {
        let mut lapic = LocalApicMmio::new();
        assert_eq!(lapic.mmio_read_u8(LAPIC_DEFAULT_BASE + 0xB0), Some(0)); // EOI
        assert!(lapic.mmio_write_u8(LAPIC_DEFAULT_BASE + 0xB0, 0x00));
    }

    #[test]
    fn reset_restores_id_zero() {
        let mut lapic = LocalApicMmio::new();
        assert!(lapic.mmio_write_u8(LAPIC_DEFAULT_BASE + u64::from(LAPIC_REG_ID) + 3, 0x07));
        lapic.reset();
        assert_eq!(lapic, LocalApicMmio::new());
    }
}
