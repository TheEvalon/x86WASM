//! PIIX3/PIIX4 X-Bus Chip Select (XBCS) at ISA-bridge config `4Eh`.
//!
//! Spec: Intel 82371AB (PIIX4) datasheet §4.1.9 — XBCS at offset `4E–4Fh`,
//! default `03h`, R/W. Bit 2 is **BIOSCS# Write Protect Enable**:
//! - `1` = BIOSCS# asserted for BIOS read **and** write cycles in decoded
//!   regions
//! - `0` = BIOSCS# asserted for BIOS **read** cycles only (writes not claimed
//!   by the ROM chip-select)
//!
//! This model owns the low byte (`4Eh`) that SeaBIOS/firmware program for
//! BIOS write protection. High-byte bits (APIC, 1M extended BIOS, …) are
//! out of scope and read as zero / ignored on write.
//!
//! Even when write-protect is lifted, a mapped ROM window still stores
//! nothing (mask ROM / unsequenced flash); see
//! `docs/machine-r4-write-semantics.md`. XBCS controls the *decode* story.

/// PIIX ISA bridge configuration offset of the XBCS low byte.
pub const XBCS_CONFIG_OFFSET: u8 = 0x4E;

/// Reset default for XBCS low byte (`4Eh`).
///
/// Spec: Intel 82371AB §4.1.9 — default value of `4E–4Fh` is `03h`.
pub const XBCS_DEFAULT: u8 = 0x03;

/// Bit 2: BIOSCS# Write Protect Enable (1 = writes also assert BIOSCS#).
pub const XBCS_BIOS_WRITE_PROTECT_ENABLE: u8 = 1 << 2;

/// Bit 6: Lower BIOS Enable (`E0000–EFFFF` / alias).
pub const XBCS_LOWER_BIOS_ENABLE: u8 = 1 << 6;

/// Bit 7: Extended BIOS Enable (`FFF80000–FFFDFFFF`).
pub const XBCS_EXTENDED_BIOS_ENABLE: u8 = 1 << 7;

/// Writable bits modeled in this slice (low byte only).
const XBCS_WRITABLE_MASK: u8 =
    XBCS_BIOS_WRITE_PROTECT_ENABLE | XBCS_LOWER_BIOS_ENABLE | XBCS_EXTENDED_BIOS_ENABLE | 0x03;

/// X-Bus Chip Select register (ISA function config `4Eh`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Xbcs {
    value: u8,
}

impl Default for Xbcs {
    fn default() -> Self {
        Self::new()
    }
}

impl Xbcs {
    pub fn new() -> Self {
        Self {
            value: XBCS_DEFAULT,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn value(&self) -> u8 {
        self.value
    }

    /// Store a guest write, clearing unsupported high bits in this slice.
    pub fn write(&mut self, raw: u8) {
        self.value = raw & XBCS_WRITABLE_MASK;
    }

    /// Spec: bit 2 = 0 → BIOSCS# is not asserted for BIOS write cycles.
    pub fn bios_write_protect_enabled(&self) -> bool {
        self.value & XBCS_BIOS_WRITE_PROTECT_ENABLE == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_matches_piix_datasheet_03h() {
        let x = Xbcs::new();
        assert_eq!(x.value(), XBCS_DEFAULT);
        assert!(x.bios_write_protect_enabled());
    }

    #[test]
    fn bit2_lifts_write_protect() {
        let mut x = Xbcs::new();
        x.write(XBCS_DEFAULT | XBCS_BIOS_WRITE_PROTECT_ENABLE);
        assert!(!x.bios_write_protect_enabled());
        assert_eq!(
            x.value() & XBCS_BIOS_WRITE_PROTECT_ENABLE,
            XBCS_BIOS_WRITE_PROTECT_ENABLE
        );
    }

    #[test]
    fn unsupported_bits_masked_on_write() {
        let mut x = Xbcs::new();
        x.write(0xFF);
        assert_eq!(x.value(), XBCS_WRITABLE_MASK);
    }
}
