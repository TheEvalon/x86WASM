//! Firmware interface stubs (fw_cfg / ACPI arrive later).
//!
//! BIOS ROM placement helpers map a legacy BIOS image the way a classic PC
//! exposes SeaBIOS: at the top of the 4 GiB physical space and aliased in the
//! last 128 KiB below 1 MiB (64 KiB images land at `0xF0000`).

#![forbid(unsafe_code)]

/// How a ROM image should be placed in physical memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RomImage {
    pub phys_base: u64,
    pub data: Vec<u8>,
}

impl RomImage {
    pub fn new(phys_base: u64, data: Vec<u8>) -> Self {
        Self { phys_base, data }
    }
}

/// End of the 32-bit physical address space (top of 4 GiB).
pub const BIOS_ROM_HIGH_END: u64 = 0x1_0000_0000;

/// End of the first mebibyte — low BIOS alias region ends here.
pub const BIOS_ROM_LOW_END: u64 = 0x0010_0000;

/// Maximum BIOS image accepted by [`prepare_bios_rom`] (typical SeaBIOS upper bound).
pub const BIOS_ROM_MAX_SIZE: usize = 256 * 1024;

/// Low-memory alias window size (classic PC `0xE0000`–`0xFFFFF`).
pub const BIOS_ROM_LOW_ALIAS_MAX: usize = 128 * 1024;

/// Dual placement for a legacy BIOS ROM (high + below-1 MiB alias).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BiosRomMap {
    /// Image right-aligned under [`BIOS_ROM_HIGH_END`].
    pub high: RomImage,
    /// Last `min(len, 128 KiB)` right-aligned under [`BIOS_ROM_LOW_END`].
    pub low: RomImage,
}

/// Errors from [`prepare_bios_rom`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BiosRomError {
    Empty,
    TooLarge,
}

impl core::fmt::Display for BiosRomError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty => write!(f, "BIOS ROM image is empty"),
            Self::TooLarge => write!(f, "BIOS ROM image exceeds {} bytes", BIOS_ROM_MAX_SIZE),
        }
    }
}

impl std::error::Error for BiosRomError {}

/// Compute classic-PC high + low alias placements for a BIOS image.
///
/// Spec / docs: SeaBIOS / classic PC memory map — firmware is mapped at the
/// top of 4 GiB; the last up to 128 KiB is also visible below 1 MiB
/// (`0xF0000` for a 64 KiB image, `0xE0000` for 128 KiB). See
/// `docs/sources.md` (Firmware) and `docs/machine-model-pc-v1.md`.
///
/// Does not parse, execute, or vendor SeaBIOS sources — placement only.
pub fn prepare_bios_rom(data: &[u8]) -> Result<BiosRomMap, BiosRomError> {
    if data.is_empty() {
        return Err(BiosRomError::Empty);
    }
    if data.len() > BIOS_ROM_MAX_SIZE {
        return Err(BiosRomError::TooLarge);
    }

    let high_base = BIOS_ROM_HIGH_END - data.len() as u64;
    let high = RomImage::new(high_base, data.to_vec());

    let low_len = data.len().min(BIOS_ROM_LOW_ALIAS_MAX);
    let low_off = data.len() - low_len;
    let low_base = BIOS_ROM_LOW_END - low_len as u64;
    let low = RomImage::new(low_base, data[low_off..].to_vec());

    Ok(BiosRomMap { high, low })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rom_image_stores_bytes() {
        let r = RomImage::new(0xFFFF_0000, vec![0xF4]);
        assert_eq!(r.data, vec![0xF4]);
    }

    /// Spec: classic PC / SeaBIOS — 64 KiB BIOS at top of 4 GiB and `0xF0000`.
    #[test]
    fn prepare_bios_rom_64k_high_and_f0000_alias() {
        let mut img = vec![0u8; 64 * 1024];
        img[0] = 0xAA;
        img[0xFFF0] = 0xF4;
        img[0xFFFF] = 0x55;

        let map = prepare_bios_rom(&img).expect("64 KiB BIOS");
        assert_eq!(map.high.phys_base, 0xFFFF_0000);
        assert_eq!(map.low.phys_base, 0x000F_0000);
        assert_eq!(map.high.data.len(), 64 * 1024);
        assert_eq!(map.low.data.len(), 64 * 1024);
        assert_eq!(map.high.data[0], 0xAA);
        assert_eq!(map.low.data[0], 0xAA);
        assert_eq!(map.high.data[0xFFF0], 0xF4);
        assert_eq!(map.low.data[0xFFF0], 0xF4);
        assert_eq!(map.high.data[0xFFFF], 0x55);
        assert_eq!(map.low.data[0xFFFF], 0x55);
    }

    /// Spec: last 128 KiB of a larger image aliases at `0xE0000`.
    #[test]
    fn prepare_bios_rom_256k_aliases_last_128k_at_e0000() {
        let mut img = vec![0u8; 256 * 1024];
        img[0] = 0x11; // high-only prefix
        img[128 * 1024] = 0x22; // start of low alias
        img[256 * 1024 - 1] = 0x33;

        let map = prepare_bios_rom(&img).expect("256 KiB BIOS");
        assert_eq!(map.high.phys_base, 0xFFFC_0000);
        assert_eq!(map.high.data.len(), 256 * 1024);
        assert_eq!(map.low.phys_base, 0x000E_0000);
        assert_eq!(map.low.data.len(), 128 * 1024);
        assert_eq!(map.low.data[0], 0x22);
        assert_eq!(map.low.data[128 * 1024 - 1], 0x33);
        assert_eq!(map.high.data[0], 0x11);
    }

    #[test]
    fn prepare_bios_rom_rejects_empty_and_too_large() {
        assert_eq!(prepare_bios_rom(&[]), Err(BiosRomError::Empty));
        let big = vec![0u8; BIOS_ROM_MAX_SIZE + 1];
        assert_eq!(prepare_bios_rom(&big), Err(BiosRomError::TooLarge));
    }
}
