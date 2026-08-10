//! Host helpers: minimal INT 19h-bootable HD/floppy images for SeaBIOS.
//!
//! After CF9, POST without media reboot-loops at `F000:9842` (`boot_fail`).
//! These helpers attach a synthetic signed MBR (active partition) or 1.44MB
//! floppy VBR so firmware INT 19h has a candidate — **not** a FreeDOS prompt.
//!
//! Spec: IBM PC BIOS INT 19h / OSDev Boot Sequence; classic MBR partition
//! table (boot indicator `80h`); floppy boot sector `0x55AA`.
//! See `docs/boot-r13-int19-bootable-media.md`.

use crate::mbr::{MBR_SECTOR_SIZE, MBR_SIGNATURE_HI, MBR_SIGNATURE_LO};
use crate::Machine;
use devices::FDC_1440_IMAGE_SIZE;

/// Classic first MBR partition-table entry offset.
pub const MBR_PART0_OFF: usize = 0x1BE;
/// Active / bootable partition indicator (IBM / SeaBIOS).
pub const MBR_PART_BOOTABLE: u8 = 0x80;
/// FAT12 system ID (common floppy/small-partition type).
pub const MBR_PART_TYPE_FAT12: u8 = 0x01;

/// How a raw image looks to an INT 19h / SeaBIOS boot scan (host classify).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Int19BootMediaClass {
    /// Shorter than one sector.
    TooShort,
    /// Sector 0 lacks `0x55AA`.
    MissingSignature,
    /// HD-sized image with signature but no active (`80h`) partition.
    HdSignatureOnly,
    /// HD image with an active partition entry (INT 19h HDD candidate).
    HdActivePartition { part_lba: u32, part_type: u8 },
    /// Exact 1.44MB floppy with signed boot sector.
    FloppyBootSector,
}

impl Int19BootMediaClass {
    pub fn tag(&self) -> &'static str {
        match self {
            Self::TooShort => "too-short",
            Self::MissingSignature => "missing-signature",
            Self::HdSignatureOnly => "hd-signature-only",
            Self::HdActivePartition { .. } => "hd-active-partition",
            Self::FloppyBootSector => "floppy-boot-sector",
        }
    }

    /// True when SeaBIOS INT 19h would treat this as bootable media (not no-media).
    pub fn is_int19_candidate(&self) -> bool {
        matches!(
            self,
            Self::HdActivePartition { .. } | Self::FloppyBootSector
        )
    }
}

impl std::fmt::Display for Int19BootMediaClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HdActivePartition {
                part_lba,
                part_type,
            } => write!(
                f,
                "hd-active-partition:lba={part_lba} type={part_type:#04x}"
            ),
            other => f.write_str(other.tag()),
        }
    }
}

/// Classify a raw disk/floppy buffer for INT 19h boot candidacy.
///
/// Spec: IBM BIOS — boot sector signature at 510–511; HDD scan uses active
/// partition indicator `80h` at MBR entry 0..3. Does not execute boot code.
pub fn classify_int19_boot_image(image: &[u8]) -> Int19BootMediaClass {
    if image.len() < MBR_SECTOR_SIZE {
        return Int19BootMediaClass::TooShort;
    }
    if image[510] != MBR_SIGNATURE_LO || image[511] != MBR_SIGNATURE_HI {
        return Int19BootMediaClass::MissingSignature;
    }
    if image.len() == FDC_1440_IMAGE_SIZE {
        return Int19BootMediaClass::FloppyBootSector;
    }
    for i in 0..4 {
        let off = MBR_PART0_OFF + i * 16;
        if image[off] == MBR_PART_BOOTABLE {
            let part_lba = u32::from_le_bytes([
                image[off + 8],
                image[off + 9],
                image[off + 10],
                image[off + 11],
            ]);
            let part_type = image[off + 4];
            return Int19BootMediaClass::HdActivePartition {
                part_lba,
                part_type,
            };
        }
    }
    Int19BootMediaClass::HdSignatureOnly
}

fn write_part0_active(mbr: &mut [u8], part_lba: u32, part_sectors: u32, part_type: u8) {
    let off = MBR_PART0_OFF;
    mbr[off] = MBR_PART_BOOTABLE;
    // CHS start: C/H/S = 0/1/1 (decorative; LBA is authoritative for host helpers).
    mbr[off + 1] = 0x01; // head
    mbr[off + 2] = 0x01; // sector 1
    mbr[off + 3] = 0x00; // cyl 0
    mbr[off + 4] = part_type;
    mbr[off + 5] = 0x01;
    mbr[off + 6] = 0x01;
    mbr[off + 7] = 0x00;
    mbr[off + 8..off + 12].copy_from_slice(&part_lba.to_le_bytes());
    mbr[off + 12..off + 16].copy_from_slice(&part_sectors.to_le_bytes());
}

fn hlt_sector() -> [u8; MBR_SECTOR_SIZE] {
    let mut s = [0x90u8; MBR_SECTOR_SIZE];
    s[0] = 0xF4; // HLT
    s[510] = MBR_SIGNATURE_LO;
    s[511] = MBR_SIGNATURE_HI;
    s
}

/// Minimal INT 19h-bootable HD: signed MBR + active FAT12 partition + HLT VBR.
///
/// MBR code is HLT (host/SeaBIOS load smoke). Partition starts at LBA 1.
/// Size is 32 sectors (not a real filesystem).
pub fn synthetic_int19_bootable_hd() -> Vec<u8> {
    const SECTORS: usize = 32;
    let mut img = vec![0u8; SECTORS * MBR_SECTOR_SIZE];
    let mut mbr = hlt_sector();
    // Marker in unused MBR body for tests.
    mbr[1..5].copy_from_slice(b"INT1");
    write_part0_active(&mut mbr, 1, (SECTORS as u32) - 1, MBR_PART_TYPE_FAT12);
    img[..MBR_SECTOR_SIZE].copy_from_slice(&mbr);
    let vbr = hlt_sector();
    img[MBR_SECTOR_SIZE..2 * MBR_SECTOR_SIZE].copy_from_slice(&vbr);
    img
}

/// FreeDOS-*like* INT 19h HD: active partition + VBR prints `FD` then HLT.
///
/// Still **not** FreeDOS / COMMAND.COM. LBA1 holds payload marker.
pub fn synthetic_int19_freedos_stub_hd() -> Vec<u8> {
    const SECTORS: usize = 32;
    let mut img = vec![0u8; SECTORS * MBR_SECTOR_SIZE];
    let mut mbr = [0x90u8; MBR_SECTOR_SIZE];
    // Minimal MBR: jump to VBR is out of scope; MBR itself is HLT with marker.
    mbr[0] = 0xF4;
    mbr[1..5].copy_from_slice(b"FDST");
    write_part0_active(&mut mbr, 1, (SECTORS as u32) - 1, MBR_PART_TYPE_FAT12);
    mbr[510] = MBR_SIGNATURE_LO;
    mbr[511] = MBR_SIGNATURE_HI;
    img[..MBR_SECTOR_SIZE].copy_from_slice(&mbr);

    // VBR at LBA1: COM1 "FD" + VGA 'F' + HLT (same spirit as synthetic_freedos_like).
    let mut vbr = [0x90u8; MBR_SECTOR_SIZE];
    let code: &[u8] = &[
        0xBA, 0xF8, 0x03, // mov dx, 0x03F8
        0xB0, b'F', // mov al, 'F'
        0xEE, // out dx, al
        0xB0, b'D', // mov al, 'D'
        0xEE, // out dx, al
        0xB8, 0x00, 0xB8, // mov ax, 0xB800
        0x8E, 0xC0, // mov es, ax
        0x31, 0xFF, // xor di, di
        0xB0, b'F', // mov al, 'F'
        0xB4, 0x07, // mov ah, 0x07
        0xAB, // stosw
        0xF4, // hlt
    ];
    vbr[..code.len()].copy_from_slice(code);
    vbr[510] = MBR_SIGNATURE_LO;
    vbr[511] = MBR_SIGNATURE_HI;
    img[MBR_SECTOR_SIZE..2 * MBR_SECTOR_SIZE].copy_from_slice(&vbr);
    let marker = b"FREEDOS-STUB-PAYLOAD\0";
    img[2 * MBR_SECTOR_SIZE..2 * MBR_SECTOR_SIZE + marker.len()].copy_from_slice(marker);
    img
}

/// Minimal INT 19h-bootable 1.44MB floppy: signed CHS `(0,0,1)` HLT VBR.
pub fn synthetic_int19_bootable_floppy() -> Vec<u8> {
    let mut img = vec![0u8; FDC_1440_IMAGE_SIZE];
    let mut vbr = hlt_sector();
    vbr[1..5].copy_from_slice(b"FLOP");
    img[..MBR_SECTOR_SIZE].copy_from_slice(&vbr);
    img
}

impl Machine {
    /// Attach [`synthetic_int19_bootable_hd`] on the primary IDE master.
    ///
    /// Host helper for SeaBIOS INT 19h media — not a guest filesystem or FreeDOS.
    pub fn attach_bootable_hd_for_int19(&mut self) {
        self.attach_ide_image(synthetic_int19_bootable_hd());
    }

    /// Attach [`synthetic_int19_freedos_stub_hd`] (active partition + FreeDOS-like VBR).
    pub fn attach_freedos_stub_hd_for_int19(&mut self) {
        self.attach_ide_image(synthetic_int19_freedos_stub_hd());
    }

    /// Attach [`synthetic_int19_bootable_floppy`] to the FDC.
    pub fn attach_bootable_floppy_for_int19(&mut self) -> Result<(), &'static str> {
        self.attach_floppy_image(synthetic_int19_bootable_floppy())
    }

    /// Classify currently attached IDE image for INT 19h (empty → [`Int19BootMediaClass::TooShort`]).
    pub fn classify_attached_ide_int19(&self) -> Int19BootMediaClass {
        if !self.ide.present || self.ide.image.is_empty() {
            return Int19BootMediaClass::TooShort;
        }
        classify_int19_boot_image(&self.ide.image)
    }

    /// Classify currently attached floppy image for INT 19h.
    ///
    /// This tree only attaches exact 1.44MB images; classification uses CHS
    /// `(0,0,1)` signature via [`devices::Fdc82077::read_sector`].
    pub fn classify_attached_floppy_int19(&self) -> Int19BootMediaClass {
        if !self.fdc.has_media() {
            return Int19BootMediaClass::TooShort;
        }
        let Some(sector) = self.fdc.read_sector(0, 0, 1) else {
            return Int19BootMediaClass::TooShort;
        };
        if sector[510] != MBR_SIGNATURE_LO || sector[511] != MBR_SIGNATURE_HI {
            return Int19BootMediaClass::MissingSignature;
        }
        Int19BootMediaClass::FloppyBootSector
    }
}

/// Prefer IDE classify; else floppy. Used by FreeDOS-with-media readiness.
pub fn classify_machine_int19_media(machine: &Machine) -> Int19BootMediaClass {
    let ide = machine.classify_attached_ide_int19();
    if ide.is_int19_candidate() || !matches!(ide, Int19BootMediaClass::TooShort) {
        return ide;
    }
    machine.classify_attached_floppy_int19()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spec: INT 19h HDD candidate needs `0x55AA` + active partition `80h`.
    #[test]
    fn synthetic_hd_is_int19_active_partition() {
        let img = synthetic_int19_bootable_hd();
        match classify_int19_boot_image(&img) {
            Int19BootMediaClass::HdActivePartition {
                part_lba,
                part_type,
            } => {
                assert_eq!(part_lba, 1);
                assert_eq!(part_type, MBR_PART_TYPE_FAT12);
            }
            other => panic!("unexpected {other}"),
        }
        assert!(classify_int19_boot_image(&img).is_int19_candidate());
        assert_eq!(&img[1..5], b"INT1");
        assert_eq!(img[MBR_SECTOR_SIZE], 0xF4);
        assert_eq!(img[MBR_SECTOR_SIZE + 510], MBR_SIGNATURE_LO);
    }

    /// Spec: floppy INT 19h path — 1.44MB + signature.
    #[test]
    fn synthetic_floppy_is_int19_boot_sector() {
        let img = synthetic_int19_bootable_floppy();
        assert_eq!(img.len(), FDC_1440_IMAGE_SIZE);
        assert_eq!(
            classify_int19_boot_image(&img),
            Int19BootMediaClass::FloppyBootSector
        );
    }

    #[test]
    fn signature_only_hd_is_not_int19_candidate() {
        let mut img = vec![0u8; 4 * MBR_SECTOR_SIZE];
        img[510] = MBR_SIGNATURE_LO;
        img[511] = MBR_SIGNATURE_HI;
        assert_eq!(
            classify_int19_boot_image(&img),
            Int19BootMediaClass::HdSignatureOnly
        );
        assert!(!classify_int19_boot_image(&img).is_int19_candidate());
    }

    #[test]
    fn attach_bootable_hd_helper_wires_ide() {
        let mut m = Machine::new(64 * 1024);
        m.attach_bootable_hd_for_int19();
        assert!(m.ide.present);
        assert!(m.classify_attached_ide_int19().is_int19_candidate());
        m.load_mbr_to_7c00().expect("mbr");
        assert_eq!(m.cpu.ip16(), 0x7C00);
        assert_eq!(m.mem.read_u8(0x7C00).unwrap(), 0xF4);
    }

    #[test]
    fn attach_bootable_floppy_helper_wires_fdc() {
        let mut m = Machine::new(64 * 1024);
        m.attach_bootable_floppy_for_int19().expect("floppy");
        assert_eq!(
            m.classify_attached_floppy_int19(),
            Int19BootMediaClass::FloppyBootSector
        );
        m.load_floppy_boot_to_7c00().expect("vbr");
        assert_eq!(m.mem.read_u8(0x7C00).unwrap(), 0xF4);
    }

    #[test]
    fn freedos_stub_hd_has_active_partition_and_vbr_code() {
        let img = synthetic_int19_freedos_stub_hd();
        assert!(classify_int19_boot_image(&img).is_int19_candidate());
        assert_eq!(&img[1..5], b"FDST");
        // VBR starts with mov dx,0x3F8
        assert_eq!(img[MBR_SECTOR_SIZE], 0xBA);
        assert_eq!(img[MBR_SECTOR_SIZE + 1], 0xF8);
        assert_eq!(img[MBR_SECTOR_SIZE + 2], 0x03);
    }

    #[test]
    fn missing_signature_and_short_images() {
        assert_eq!(
            classify_int19_boot_image(&[0u8; 16]),
            Int19BootMediaClass::TooShort
        );
        let mut bad = vec![0u8; MBR_SECTOR_SIZE];
        assert_eq!(
            classify_int19_boot_image(&bad),
            Int19BootMediaClass::MissingSignature
        );
        bad[510] = MBR_SIGNATURE_LO;
        bad[511] = MBR_SIGNATURE_HI;
        // Exactly one sector → not 1.44MB → HD signature only (no partition).
        assert_eq!(
            classify_int19_boot_image(&bad),
            Int19BootMediaClass::HdSignatureOnly
        );
    }
}
