//! Firmware interface stubs (fw_cfg / ACPI arrive later).
//!
//! BIOS ROM placement helpers map a legacy BIOS image the way a classic PC
//! exposes SeaBIOS: at the top of the 4 GiB physical space and aliased in the
//! last 128 KiB below 1 MiB (64 KiB images land at `0xF0000`).

#![forbid(unsafe_code)]

mod el_torito;

pub use el_torito::{
    parse_el_torito, ElToritoError, ElToritoInfo, EL_TORITO_BOOTABLE, EL_TORITO_BOOT_SYSTEM_ID,
    EL_TORITO_KEY_55, EL_TORITO_KEY_AA, EL_TORITO_PLATFORM_X86, EL_TORITO_SECTOR_BYTES,
    EL_TORITO_VALIDATION_HEADER_ID, ISO9660_PVD_LBA, ISO9660_STANDARD_ID, ISO9660_VD_BOOT_RECORD,
    ISO9660_VD_TERMINATOR,
};

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

/// Base of the legacy option-ROM (expansion ROM) region.
///
/// Spec: BIOS Boot Specification / IBM PC memory map — the BIOS scans
/// `0xC0000`-`0xDFFFF` for expansion ROMs on 2 KiB boundaries.
pub const OPTION_ROM_REGION_BASE: u64 = 0x000C_0000;

/// End (exclusive) of the legacy option-ROM region.
pub const OPTION_ROM_REGION_END: u64 = 0x000E_0000;

/// Boundary the option-ROM scan steps by (2 KiB).
pub const OPTION_ROM_SCAN_STEP: u64 = 2 * 1024;

/// Conventional base of the video BIOS.
pub const VGA_OPTION_ROM_BASE: u64 = 0x000C_0000;

/// Expansion ROM signature at offsets 0-1.
pub const OPTION_ROM_SIGNATURE: [u8; 2] = [0x55, 0xAA];

/// Unit of the initialization-size byte at offset 2.
pub const OPTION_ROM_BLOCK_SIZE: usize = 512;

/// Smallest header a size and checksum can be read from.
pub const OPTION_ROM_HEADER_LEN: usize = 3;

/// Errors from [`prepare_option_rom`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionRomError {
    /// Shorter than the signature plus size byte.
    TooSmall,
    /// Offsets 0-1 are not `0x55 0xAA`.
    BadSignature,
    /// The initialization-size byte is zero.
    ZeroSize,
    /// The declared size runs past the supplied image.
    SizeExceedsImage,
    /// The byte-wise sum over the declared size is not zero.
    BadChecksum,
    /// The base is not on a 2 KiB boundary.
    Misaligned,
    /// The image does not fit inside `0xC0000`-`0xDFFFF`.
    OutsideRegion,
}

impl core::fmt::Display for OptionRomError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooSmall => write!(f, "option ROM shorter than its header"),
            Self::BadSignature => write!(f, "option ROM signature is not 0x55AA"),
            Self::ZeroSize => write!(f, "option ROM initialization size is zero"),
            Self::SizeExceedsImage => write!(f, "option ROM size byte exceeds the image"),
            Self::BadChecksum => write!(f, "option ROM checksum is not zero"),
            Self::Misaligned => write!(
                f,
                "option ROM base is not {OPTION_ROM_SCAN_STEP}-byte aligned"
            ),
            Self::OutsideRegion => write!(
                f,
                "option ROM does not fit in {OPTION_ROM_REGION_BASE:#X}-{:#X}",
                OPTION_ROM_REGION_END - 1
            ),
        }
    }
}

impl std::error::Error for OptionRomError {}

/// Validate a PC-compatible expansion ROM and place it in the legacy region.
///
/// Spec: PCI Firmware Specification / BIOS Boot Specification, PC-compatible
/// expansion ROM header — byte 0-1 signature `0x55 0xAA`, byte 2 the
/// initialization size in 512-byte blocks, byte 3 onwards the entry point; the
/// byte-wise sum over the initialization size must be zero modulo 256. The
/// BIOS scans `0xC0000`-`0xDFFFF` on 2 KiB boundaries for that signature, so
/// the base must be aligned and the declared image must fit in the region.
///
/// Only the declared initialization size is mapped; any trailing bytes in the
/// supplied file are ignored. Runtime size, the PnP expansion header at offset
/// `0x1A`, and BEV/BCV boot entries are not parsed.
pub fn prepare_option_rom(phys_base: u64, data: &[u8]) -> Result<RomImage, OptionRomError> {
    if data.len() < OPTION_ROM_HEADER_LEN {
        return Err(OptionRomError::TooSmall);
    }
    if data[0] != OPTION_ROM_SIGNATURE[0] || data[1] != OPTION_ROM_SIGNATURE[1] {
        return Err(OptionRomError::BadSignature);
    }
    let blocks = usize::from(data[2]);
    if blocks == 0 {
        return Err(OptionRomError::ZeroSize);
    }
    let len = blocks * OPTION_ROM_BLOCK_SIZE;
    if len > data.len() {
        return Err(OptionRomError::SizeExceedsImage);
    }
    let checksum = data[..len].iter().fold(0u8, |acc, b| acc.wrapping_add(*b));
    if checksum != 0 {
        return Err(OptionRomError::BadChecksum);
    }
    if !phys_base.is_multiple_of(OPTION_ROM_SCAN_STEP) {
        return Err(OptionRomError::Misaligned);
    }
    let end = phys_base
        .checked_add(len as u64)
        .ok_or(OptionRomError::OutsideRegion)?;
    if phys_base < OPTION_ROM_REGION_BASE || end > OPTION_ROM_REGION_END {
        return Err(OptionRomError::OutsideRegion);
    }

    Ok(RomImage::new(phys_base, data[..len].to_vec()))
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

    /// Build a valid PC-compatible expansion ROM of `blocks` 512-byte blocks.
    ///
    /// Spec: PCI Firmware Specification / BIOS Boot Specification expansion ROM
    /// header — `0x55 0xAA`, size in 512-byte blocks at offset 2, entry point
    /// from offset 3, byte-wise checksum over the declared size equal to zero.
    fn synthetic_option_rom(blocks: u8) -> Vec<u8> {
        let mut rom = vec![0u8; usize::from(blocks) * OPTION_ROM_BLOCK_SIZE];
        rom[0] = OPTION_ROM_SIGNATURE[0];
        rom[1] = OPTION_ROM_SIGNATURE[1];
        rom[2] = blocks;
        rom[3] = 0xCB; // RETF entry point stub
        let sum = rom.iter().fold(0u8, |acc, b| acc.wrapping_add(*b));
        let last = rom.len() - 1;
        rom[last] = rom[last].wrapping_sub(sum);
        rom
    }

    #[test]
    fn prepare_option_rom_accepts_a_valid_vga_style_image() {
        let rom = synthetic_option_rom(4);
        let image = prepare_option_rom(VGA_OPTION_ROM_BASE, &rom).expect("valid option ROM");
        assert_eq!(image.phys_base, 0x000C_0000);
        assert_eq!(image.data.len(), 4 * OPTION_ROM_BLOCK_SIZE);
        assert_eq!(image.data[0], 0x55);
        assert_eq!(image.data[1], 0xAA);
    }

    /// Only the declared initialization size is mapped; trailing padding in the
    /// supplied file is not.
    #[test]
    fn prepare_option_rom_maps_only_the_declared_size() {
        let mut rom = synthetic_option_rom(2);
        rom.extend_from_slice(&[0xEE; 512]);
        let image = prepare_option_rom(VGA_OPTION_ROM_BASE, &rom).expect("valid option ROM");
        assert_eq!(image.data.len(), 2 * OPTION_ROM_BLOCK_SIZE);
        assert!(!image.data.contains(&0xEE));
    }

    #[test]
    fn prepare_option_rom_rejects_malformed_images() {
        assert_eq!(
            prepare_option_rom(VGA_OPTION_ROM_BASE, &[0x55, 0xAA]),
            Err(OptionRomError::TooSmall)
        );

        let mut bad_sig = synthetic_option_rom(1);
        bad_sig[0] = 0x54;
        assert_eq!(
            prepare_option_rom(VGA_OPTION_ROM_BASE, &bad_sig),
            Err(OptionRomError::BadSignature)
        );

        let mut zero_size = synthetic_option_rom(1);
        zero_size[2] = 0;
        assert_eq!(
            prepare_option_rom(VGA_OPTION_ROM_BASE, &zero_size),
            Err(OptionRomError::ZeroSize)
        );

        let mut oversized = synthetic_option_rom(1);
        oversized[2] = 4;
        assert_eq!(
            prepare_option_rom(VGA_OPTION_ROM_BASE, &oversized),
            Err(OptionRomError::SizeExceedsImage)
        );

        let mut bad_sum = synthetic_option_rom(1);
        bad_sum[4] = bad_sum[4].wrapping_add(1);
        assert_eq!(
            prepare_option_rom(VGA_OPTION_ROM_BASE, &bad_sum),
            Err(OptionRomError::BadChecksum)
        );
    }

    /// Spec: legacy expansion ROMs are scanned on 2 KiB boundaries within
    /// `0xC0000`-`0xDFFFF`.
    #[test]
    fn prepare_option_rom_rejects_bad_placement() {
        let rom = synthetic_option_rom(4);
        assert_eq!(
            prepare_option_rom(0x000C_0400, &rom),
            Err(OptionRomError::Misaligned)
        );
        assert_eq!(
            prepare_option_rom(0x000B_F800, &rom),
            Err(OptionRomError::OutsideRegion)
        );
        // Aligned, but the declared size runs past the end of the region.
        let last = OPTION_ROM_REGION_END - OPTION_ROM_SCAN_STEP;
        assert_eq!(
            prepare_option_rom(last, &synthetic_option_rom(6)),
            Err(OptionRomError::OutsideRegion)
        );
        // The last aligned slot that still fits is accepted.
        assert!(prepare_option_rom(last, &rom).is_ok());
    }

    #[test]
    fn prepare_bios_rom_rejects_empty_and_too_large() {
        assert_eq!(prepare_bios_rom(&[]), Err(BiosRomError::Empty));
        let big = vec![0u8; BIOS_ROM_MAX_SIZE + 1];
        assert_eq!(prepare_bios_rom(&big), Err(BiosRomError::TooLarge));
    }
}
