//! El Torito boot-catalog detection (host-side validation only).
//!
//! Spec: "El Torito" Bootable CD-ROM Format Specification Version 1.0 —
//! Boot Record Volume Descriptor, Validation Entry (`55h`/`AAh` key bytes),
//! and Initial/Default Entry. Does **not** emulate INT 13h CD boot.

/// ISO 9660 / Mode-1 logical block size used by El Torito pointers.
pub const EL_TORITO_SECTOR_BYTES: usize = 2048;

/// ISO 9660 Primary Volume Descriptor absolute sector.
pub const ISO9660_PVD_LBA: u32 = 16;

/// Volume Descriptor Type: Boot Record.
pub const ISO9660_VD_BOOT_RECORD: u8 = 0;
/// Volume Descriptor Type: Set Terminator.
pub const ISO9660_VD_TERMINATOR: u8 = 255;

/// Standard identifier in every ISO 9660 volume descriptor.
pub const ISO9660_STANDARD_ID: &[u8; 5] = b"CD001";

/// Boot System Identifier in an El Torito Boot Record (space-padded).
pub const EL_TORITO_BOOT_SYSTEM_ID: &[u8; 23] = b"EL TORITO SPECIFICATION";

/// Validation Entry header ID (must be `01h`).
pub const EL_TORITO_VALIDATION_HEADER_ID: u8 = 0x01;
/// Validation Entry key bytes (offsets 30–31). Spec figure 2: `55h`, `AAh`.
pub const EL_TORITO_KEY_55: u8 = 0x55;
pub const EL_TORITO_KEY_AA: u8 = 0xAA;
/// Initial/Default Entry boot indicator — bootable.
pub const EL_TORITO_BOOTABLE: u8 = 0x88;
/// Boot media type `00h` — no emulation. Spec: El Torito Figure 3.
pub const EL_TORITO_MEDIA_NO_EMUL: u8 = 0x00;
/// Default load segment when the catalog stores `0000h`. Spec: El Torito.
pub const EL_TORITO_DEFAULT_LOAD_SEGMENT: u16 = 0x07C0;
/// Physical address for [`EL_TORITO_DEFAULT_LOAD_SEGMENT`].
pub const EL_TORITO_DEFAULT_LOAD_PHYS: u64 = (EL_TORITO_DEFAULT_LOAD_SEGMENT as u64) << 4;

/// Platform ID `00h` — 80x86. Spec Validation Entry.
pub const EL_TORITO_PLATFORM_X86: u8 = 0x00;

/// Host-side summary of a validated El Torito catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElToritoInfo {
    /// Absolute sector of the Boot Record Volume Descriptor.
    pub boot_record_lba: u32,
    /// Absolute sector of the Boot Catalog.
    pub catalog_lba: u32,
    /// Platform ID from the Validation Entry (`00h` = 80x86).
    pub platform_id: u8,
    /// True when the Initial/Default Entry boot indicator is `88h`.
    pub bootable: bool,
    /// Boot media type from the Initial/Default Entry.
    pub media_type: u8,
    /// Load segment from the Initial/Default Entry (`0000h` → use default).
    pub load_segment: u16,
    /// Load RBA (absolute sector of the boot image).
    pub load_rba: u32,
    /// Sector count field (512-byte virtual sectors per El Torito).
    pub sector_count: u16,
}

impl ElToritoInfo {
    /// Effective real-mode load segment (`0000h` resolves to [`EL_TORITO_DEFAULT_LOAD_SEGMENT`]).
    pub fn effective_load_segment(&self) -> u16 {
        if self.load_segment == 0 {
            EL_TORITO_DEFAULT_LOAD_SEGMENT
        } else {
            self.load_segment
        }
    }

    /// Physical load address = effective segment × 16.
    pub fn load_phys(&self) -> u64 {
        u64::from(self.effective_load_segment()) << 4
    }

    /// Bytes the BIOS would transfer (`sector_count` × 512).
    pub fn load_byte_len(&self) -> Option<usize> {
        usize::from(self.sector_count).checked_mul(512)
    }
}

/// Errors from [`parse_el_torito`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElToritoError {
    /// Image shorter than one ISO sector, or an LBA is out of range.
    Truncated,
    /// No El Torito Boot Record Volume Descriptor was found.
    NoBootRecord,
    /// Boot Catalog sector is missing or truncated.
    TruncatedCatalog,
    /// Validation Entry header ID is not `01h`.
    BadValidationHeader,
    /// Validation Entry key bytes are not `55h`/`AAh`.
    BadValidationKey,
    /// Sum of Validation Entry words is not zero.
    BadValidationChecksum,
}

impl core::fmt::Display for ElToritoError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Truncated => write!(f, "ISO image truncated for El Torito parse"),
            Self::NoBootRecord => write!(f, "no El Torito Boot Record Volume Descriptor"),
            Self::TruncatedCatalog => write!(f, "El Torito Boot Catalog truncated"),
            Self::BadValidationHeader => write!(f, "El Torito Validation Entry bad header"),
            Self::BadValidationKey => write!(f, "El Torito Validation Entry missing 55AA"),
            Self::BadValidationChecksum => {
                write!(f, "El Torito Validation Entry checksum failed")
            }
        }
    }
}

impl std::error::Error for ElToritoError {}

fn sector(image: &[u8], lba: u32) -> Result<&[u8], ElToritoError> {
    let start = (lba as usize)
        .checked_mul(EL_TORITO_SECTOR_BYTES)
        .ok_or(ElToritoError::Truncated)?;
    let end = start
        .checked_add(EL_TORITO_SECTOR_BYTES)
        .ok_or(ElToritoError::Truncated)?;
    if end > image.len() {
        return Err(ElToritoError::Truncated);
    }
    Ok(&image[start..end])
}

fn is_el_torito_boot_record(sec: &[u8]) -> bool {
    if sec.len() < 72 {
        return false;
    }
    if sec[0] != ISO9660_VD_BOOT_RECORD {
        return false;
    }
    if &sec[1..6] != ISO9660_STANDARD_ID.as_slice() || sec[6] != 1 {
        return false;
    }
    // Boot System ID is 32 bytes; match the fixed prefix, allow trailing spaces/zeros.
    let id = &sec[7..7 + EL_TORITO_BOOT_SYSTEM_ID.len()];
    id == EL_TORITO_BOOT_SYSTEM_ID.as_slice()
}

fn find_boot_record(image: &[u8]) -> Result<(u32, u32), ElToritoError> {
    // Scan from the Primary Volume Descriptor sector onward.
    let mut lba = ISO9660_PVD_LBA;
    loop {
        let sec = sector(image, lba)?;
        if sec[0] == ISO9660_VD_TERMINATOR {
            return Err(ElToritoError::NoBootRecord);
        }
        if is_el_torito_boot_record(sec) {
            let catalog = u32::from_le_bytes([sec[0x47], sec[0x48], sec[0x49], sec[0x4A]]);
            return Ok((lba, catalog));
        }
        lba = lba.checked_add(1).ok_or(ElToritoError::Truncated)?;
        // Bound the scan so a corrupt image cannot walk forever.
        if lba > ISO9660_PVD_LBA + 64 {
            return Err(ElToritoError::NoBootRecord);
        }
    }
}

fn validation_checksum_ok(entry: &[u8; 32]) -> bool {
    let mut sum = 0u16;
    for chunk in entry.chunks_exact(2) {
        sum = sum.wrapping_add(u16::from_le_bytes([chunk[0], chunk[1]]));
    }
    sum == 0
}

/// Parse El Torito Boot Catalog validation from a raw 2048-byte-sector ISO image.
///
/// Spec: El Torito 1.0 — locate the Boot Record, read the catalog, verify the
/// Validation Entry (`01h` header, `55h`/`AAh` keys, word checksum 0), and
/// report the Initial/Default Entry bootability fields. No INT 13h emulation.
pub fn parse_el_torito(image: &[u8]) -> Result<ElToritoInfo, ElToritoError> {
    if image.len() < EL_TORITO_SECTOR_BYTES {
        return Err(ElToritoError::Truncated);
    }
    let (boot_record_lba, catalog_lba) = find_boot_record(image)?;
    let catalog = sector(image, catalog_lba).map_err(|_| ElToritoError::TruncatedCatalog)?;
    if catalog.len() < 64 {
        return Err(ElToritoError::TruncatedCatalog);
    }

    let mut validation = [0u8; 32];
    validation.copy_from_slice(&catalog[0..32]);
    if validation[0] != EL_TORITO_VALIDATION_HEADER_ID {
        return Err(ElToritoError::BadValidationHeader);
    }
    if validation[30] != EL_TORITO_KEY_55 || validation[31] != EL_TORITO_KEY_AA {
        return Err(ElToritoError::BadValidationKey);
    }
    if !validation_checksum_ok(&validation) {
        return Err(ElToritoError::BadValidationChecksum);
    }

    let default = &catalog[32..64];
    let bootable = default[0] == EL_TORITO_BOOTABLE;
    let media_type = default[1];
    let load_segment = u16::from_le_bytes([default[2], default[3]]);
    let sector_count = u16::from_le_bytes([default[6], default[7]]);
    let load_rba = u32::from_le_bytes([default[8], default[9], default[10], default[11]]);

    Ok(ElToritoInfo {
        boot_record_lba,
        catalog_lba,
        platform_id: validation[1],
        bootable,
        media_type,
        load_segment,
        load_rba,
        sector_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank_iso(sectors: usize) -> Vec<u8> {
        vec![0u8; sectors * EL_TORITO_SECTOR_BYTES]
    }

    fn write_sector(img: &mut [u8], lba: u32, data: &[u8]) {
        let start = lba as usize * EL_TORITO_SECTOR_BYTES;
        img[start..start + data.len()].copy_from_slice(data);
    }

    fn make_bootable_iso() -> Vec<u8> {
        let mut img = blank_iso(32);
        // Primary Volume Descriptor at 16 (type 1) so the scan has a neighbor.
        let mut pvd = vec![0u8; EL_TORITO_SECTOR_BYTES];
        pvd[0] = 1;
        pvd[1..6].copy_from_slice(ISO9660_STANDARD_ID);
        pvd[6] = 1;
        write_sector(&mut img, 16, &pvd);

        // Boot Record at 17.
        let mut br = vec![0u8; EL_TORITO_SECTOR_BYTES];
        br[0] = ISO9660_VD_BOOT_RECORD;
        br[1..6].copy_from_slice(ISO9660_STANDARD_ID);
        br[6] = 1;
        br[7..7 + EL_TORITO_BOOT_SYSTEM_ID.len()].copy_from_slice(EL_TORITO_BOOT_SYSTEM_ID);
        let catalog_lba = 20u32;
        br[0x47..0x4B].copy_from_slice(&catalog_lba.to_le_bytes());
        write_sector(&mut img, 17, &br);

        // Terminator at 18.
        let mut term = vec![0u8; EL_TORITO_SECTOR_BYTES];
        term[0] = ISO9660_VD_TERMINATOR;
        term[1..6].copy_from_slice(ISO9660_STANDARD_ID);
        term[6] = 1;
        write_sector(&mut img, 18, &term);

        // Boot Catalog at 20: validation + default entry.
        let mut cat = vec![0u8; EL_TORITO_SECTOR_BYTES];
        let mut validation = [0u8; 32];
        validation[0] = EL_TORITO_VALIDATION_HEADER_ID;
        validation[1] = EL_TORITO_PLATFORM_X86;
        validation[30] = EL_TORITO_KEY_55;
        validation[31] = EL_TORITO_KEY_AA;
        // Fill checksum so the word sum is zero.
        let mut sum = 0u16;
        for i in (0..32).step_by(2) {
            if i == 28 {
                continue;
            }
            sum = sum.wrapping_add(u16::from_le_bytes([validation[i], validation[i + 1]]));
        }
        let checksum = 0u16.wrapping_sub(sum);
        validation[28..30].copy_from_slice(&checksum.to_le_bytes());
        cat[0..32].copy_from_slice(&validation);

        cat[32] = EL_TORITO_BOOTABLE;
        cat[33] = 0x00; // no emulation
        cat[38..40].copy_from_slice(&4u16.to_le_bytes()); // sector count
        cat[40..44].copy_from_slice(&24u32.to_le_bytes()); // load RBA
        write_sector(&mut img, catalog_lba, &cat);

        img
    }

    #[test]
    fn parses_bootable_catalog() {
        let img = make_bootable_iso();
        let info = parse_el_torito(&img).expect("valid El Torito");
        assert_eq!(info.boot_record_lba, 17);
        assert_eq!(info.catalog_lba, 20);
        assert_eq!(info.platform_id, EL_TORITO_PLATFORM_X86);
        assert!(info.bootable);
        assert_eq!(info.media_type, EL_TORITO_MEDIA_NO_EMUL);
        assert_eq!(info.load_segment, 0);
        assert_eq!(info.effective_load_segment(), EL_TORITO_DEFAULT_LOAD_SEGMENT);
        assert_eq!(info.load_phys(), EL_TORITO_DEFAULT_LOAD_PHYS);
        assert_eq!(info.load_rba, 24);
        assert_eq!(info.sector_count, 4);
        assert_eq!(info.load_byte_len(), Some(2048));
    }

    #[test]
    fn rejects_missing_55aa() {
        let mut img = make_bootable_iso();
        let start = 20 * EL_TORITO_SECTOR_BYTES;
        img[start + 30] = 0;
        img[start + 31] = 0;
        assert_eq!(parse_el_torito(&img), Err(ElToritoError::BadValidationKey));
    }

    #[test]
    fn rejects_image_without_boot_record() {
        let mut img = blank_iso(20);
        let mut pvd = vec![0u8; EL_TORITO_SECTOR_BYTES];
        pvd[0] = 1;
        pvd[1..6].copy_from_slice(ISO9660_STANDARD_ID);
        pvd[6] = 1;
        write_sector(&mut img, 16, &pvd);
        let mut term = vec![0u8; EL_TORITO_SECTOR_BYTES];
        term[0] = ISO9660_VD_TERMINATOR;
        term[1..6].copy_from_slice(ISO9660_STANDARD_ID);
        term[6] = 1;
        write_sector(&mut img, 17, &term);
        assert_eq!(parse_el_torito(&img), Err(ElToritoError::NoBootRecord));
    }
}
