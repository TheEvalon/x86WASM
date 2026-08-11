//! Host FAT12 subset helpers for FreeDOS-path measure (R15).
//!
//! Parses a classic BPB from an active-partition VBR and walks the root
//! directory for `KERNEL.SYS` / `COMMAND.COM` (8.3 names). Used to advance
//! FreeDOS next-gap past [`crate::guest_boot::FreedosNextGap::ExecutedVbrMissingCommand`].
//!
//! Honesty: locating a directory name is **not** a FreeDOS prompt, kernel
//! load, or SeaBIOS INT 13h success.
//!
//! Spec: Microsoft FAT (FAT12) BPB / root directory; FreeDOS `KERNEL.SYS`;
//! OSDev FAT; `docs/boot-r15-freedos-next.md`.

use crate::boot_media::{
    classify_int19_boot_image, Int19BootMediaClass, MBR_PART0_OFF, MBR_PART_BOOTABLE,
    MBR_PART_TYPE_FAT12,
};
use crate::mbr::{MBR_SECTOR_SIZE, MBR_SIGNATURE_HI, MBR_SIGNATURE_LO};
use crate::Machine;

/// Bytes per FAT directory entry.
pub const FAT_DIRENT_SIZE: usize = 32;
/// 8.3 name field length (padded with spaces).
pub const FAT_NAME83_LEN: usize = 11;
/// FreeDOS kernel 8.3 directory name (`KERNEL  SYS`).
pub const FAT12_NAME_KERNEL_SYS: &[u8; 11] = b"KERNEL  SYS";
/// DOS shell 8.3 directory name (`COMMAND COM`).
pub const FAT12_NAME_COMMAND_COM: &[u8; 11] = b"COMMAND COM";
/// Attribute: volume label (skip when scanning for files).
pub const FAT_ATTR_VOLUME: u8 = 0x08;
/// Attribute: long-name (VFAT) — skip.
pub const FAT_ATTR_LFN: u8 = 0x0F;
/// End-of-directory marker (first name byte `0x00`).
pub const FAT_DIRENT_END: u8 = 0x00;
/// Deleted entry marker (first name byte `0xE5`).
pub const FAT_DIRENT_DELETED: u8 = 0xE5;

/// Classic BIOS Parameter Block fields needed for FAT12 root walk (from VBR).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fat12Bpb {
    pub bytes_per_sector: u16,
    pub sectors_per_cluster: u8,
    pub reserved_sectors: u16,
    pub fat_count: u8,
    pub root_entry_count: u16,
    pub sectors_per_fat: u16,
}

impl Fat12Bpb {
    /// First sector of the root directory relative to the volume start (VBR LBA).
    pub fn root_dir_lba_offset(self) -> u32 {
        u32::from(self.reserved_sectors)
            + u32::from(self.fat_count) * u32::from(self.sectors_per_fat)
    }

    /// Root directory size in sectors (rounded up).
    pub fn root_dir_sectors(self) -> u32 {
        let bytes = u32::from(self.root_entry_count) * FAT_DIRENT_SIZE as u32;
        let bps = u32::from(self.bytes_per_sector.max(1));
        bytes.div_ceil(bps)
    }
}

/// One short-name file found in the FAT12 root.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fat12DirEntry {
    /// 8.3 name as stored (11 bytes, space-padded).
    pub name83: [u8; 11],
    pub attr: u8,
    pub first_cluster: u16,
    pub size: u32,
}

impl Fat12DirEntry {
    pub fn is_kernel_sys(self) -> bool {
        &self.name83 == FAT12_NAME_KERNEL_SYS
    }

    pub fn is_command_com(self) -> bool {
        &self.name83 == FAT12_NAME_COMMAND_COM
    }
}

/// Host locate result for FreeDOS kernel / shell names on a FAT12 volume.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fat12KernelLocate {
    /// No IDE image / too short.
    NoMedia,
    /// Not an INT19-candidate FAT12 active partition.
    NotFat12Partition,
    /// VBR present but BPB fields unusable.
    BadBpb,
    /// Root walked; neither `KERNEL.SYS` nor `COMMAND.COM` present.
    RootMissingKernel,
    /// `KERNEL.SYS` directory entry present (size/cluster recorded).
    KernelSysPresent { entry: Fat12DirEntry },
    /// Only `COMMAND.COM` present (unusual for FreeDOS cold boot; still a name).
    CommandComPresent { entry: Fat12DirEntry },
}

impl Fat12KernelLocate {
    pub fn tag(&self) -> &'static str {
        match self {
            Self::NoMedia => "no-media",
            Self::NotFat12Partition => "not-fat12-partition",
            Self::BadBpb => "bad-bpb",
            Self::RootMissingKernel => "root-missing-kernel",
            Self::KernelSysPresent { .. } => "kernel-sys-present",
            Self::CommandComPresent { .. } => "command-com-present",
        }
    }

    /// True when a FreeDOS-relevant 8.3 name was found in the root.
    pub fn name_found(&self) -> bool {
        matches!(
            self,
            Self::KernelSysPresent { .. } | Self::CommandComPresent { .. }
        )
    }
}

impl std::fmt::Display for Fat12KernelLocate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::KernelSysPresent { entry } => write!(
                f,
                "kernel-sys-present:cluster={} size={}",
                entry.first_cluster, entry.size
            ),
            Self::CommandComPresent { entry } => write!(
                f,
                "command-com-present:cluster={} size={}",
                entry.first_cluster, entry.size
            ),
            other => f.write_str(other.tag()),
        }
    }
}

/// Parse a FAT12 BPB from a 512-byte VBR (ECMA-style / MS-DOS layout).
///
/// Spec: FAT BPB at offsets `0x0B`.. — requires `bytes_per_sector==512`,
/// `fat_count>=1`, `sectors_per_fat>=1`, `root_entry_count>=1`.
pub fn parse_fat12_bpb(vbr: &[u8]) -> Option<Fat12Bpb> {
    if vbr.len() < MBR_SECTOR_SIZE {
        return None;
    }
    if vbr[510] != MBR_SIGNATURE_LO || vbr[511] != MBR_SIGNATURE_HI {
        return None;
    }
    let bytes_per_sector = u16::from_le_bytes([vbr[0x0B], vbr[0x0C]]);
    let sectors_per_cluster = vbr[0x0D];
    let reserved_sectors = u16::from_le_bytes([vbr[0x0E], vbr[0x0F]]);
    let fat_count = vbr[0x10];
    let root_entry_count = u16::from_le_bytes([vbr[0x11], vbr[0x12]]);
    let sectors_per_fat = u16::from_le_bytes([vbr[0x16], vbr[0x17]]);
    if bytes_per_sector != 512
        || sectors_per_cluster == 0
        || reserved_sectors == 0
        || fat_count == 0
        || root_entry_count == 0
        || sectors_per_fat == 0
    {
        return None;
    }
    Some(Fat12Bpb {
        bytes_per_sector,
        sectors_per_cluster,
        reserved_sectors,
        fat_count,
        root_entry_count,
        sectors_per_fat,
    })
}

fn parse_dirent(raw: &[u8]) -> Option<Fat12DirEntry> {
    if raw.len() < FAT_DIRENT_SIZE {
        return None;
    }
    let first = raw[0];
    if first == FAT_DIRENT_END {
        return None;
    }
    if first == FAT_DIRENT_DELETED {
        return Some(Fat12DirEntry {
            name83: [0xE5; 11],
            attr: raw[11],
            first_cluster: 0,
            size: 0,
        });
    }
    let mut name83 = [0u8; 11];
    name83.copy_from_slice(&raw[0..11]);
    let attr = raw[11];
    if attr == FAT_ATTR_LFN || (attr & FAT_ATTR_VOLUME) != 0 {
        return Some(Fat12DirEntry {
            name83,
            attr,
            first_cluster: 0,
            size: 0,
        });
    }
    let first_cluster = u16::from_le_bytes([raw[26], raw[27]]);
    let size = u32::from_le_bytes([raw[28], raw[29], raw[30], raw[31]]);
    Some(Fat12DirEntry {
        name83,
        attr,
        first_cluster,
        size,
    })
}

/// Walk a root-directory byte buffer for `KERNEL.SYS` then `COMMAND.COM`.
pub fn find_freedos_names_in_root(root: &[u8]) -> Fat12KernelLocate {
    let mut command: Option<Fat12DirEntry> = None;
    let mut saw_end = false;
    let mut any_file = false;
    for chunk in root.chunks_exact(FAT_DIRENT_SIZE) {
        if saw_end {
            break;
        }
        let first = chunk[0];
        if first == FAT_DIRENT_END {
            saw_end = true;
            break;
        }
        if first == FAT_DIRENT_DELETED {
            continue;
        }
        let Some(ent) = parse_dirent(chunk) else {
            continue;
        };
        if ent.attr == FAT_ATTR_LFN || (ent.attr & FAT_ATTR_VOLUME) != 0 {
            continue;
        }
        any_file = true;
        if ent.is_kernel_sys() {
            return Fat12KernelLocate::KernelSysPresent { entry: ent };
        }
        if ent.is_command_com() && command.is_none() {
            command = Some(ent);
        }
    }
    if let Some(entry) = command {
        return Fat12KernelLocate::CommandComPresent { entry };
    }
    if any_file || saw_end || root.len() >= FAT_DIRENT_SIZE {
        Fat12KernelLocate::RootMissingKernel
    } else {
        Fat12KernelLocate::BadBpb
    }
}

/// Locate FreeDOS kernel/shell names on an IDE image with an active FAT12 partition.
///
/// Host-only: reads VBR BPB + root directory sectors. Does not load clusters.
pub fn locate_freedos_kernel_on_image(image: &[u8]) -> Fat12KernelLocate {
    match classify_int19_boot_image(image) {
        Int19BootMediaClass::TooShort => Fat12KernelLocate::NoMedia,
        Int19BootMediaClass::HdActivePartition {
            part_lba,
            part_type,
        } if part_type == MBR_PART_TYPE_FAT12 || part_type == 0x04 || part_type == 0x06 => {
            let vbr_off = (part_lba as usize).saturating_mul(MBR_SECTOR_SIZE);
            if vbr_off + MBR_SECTOR_SIZE > image.len() {
                return Fat12KernelLocate::BadBpb;
            }
            let vbr = &image[vbr_off..vbr_off + MBR_SECTOR_SIZE];
            let Some(bpb) = parse_fat12_bpb(vbr) else {
                return Fat12KernelLocate::BadBpb;
            };
            let root_lba = part_lba.saturating_add(bpb.root_dir_lba_offset());
            let root_secs = bpb.root_dir_sectors() as usize;
            let root_off = (root_lba as usize).saturating_mul(MBR_SECTOR_SIZE);
            let root_len = root_secs.saturating_mul(MBR_SECTOR_SIZE);
            if root_off + root_len > image.len() {
                return Fat12KernelLocate::BadBpb;
            }
            find_freedos_names_in_root(&image[root_off..root_off + root_len])
        }
        Int19BootMediaClass::HdActivePartition { .. }
        | Int19BootMediaClass::MissingSignature
        | Int19BootMediaClass::HdSignatureOnly
        | Int19BootMediaClass::FloppyBootSector => Fat12KernelLocate::NotFat12Partition,
    }
}

/// Locate FreeDOS names on the machine's primary IDE image.
pub fn locate_freedos_kernel_on_machine(machine: &Machine) -> Fat12KernelLocate {
    if !machine.ide.present || machine.ide.image.is_empty() {
        return Fat12KernelLocate::NoMedia;
    }
    locate_freedos_kernel_on_image(&machine.ide.image)
}

fn write_part0_active(mbr: &mut [u8], part_lba: u32, part_sectors: u32, part_type: u8) {
    let off = MBR_PART0_OFF;
    mbr[off] = MBR_PART_BOOTABLE;
    mbr[off + 1] = 0x01;
    mbr[off + 2] = 0x01;
    mbr[off + 3] = 0x00;
    mbr[off + 4] = part_type;
    mbr[off + 5] = 0x01;
    mbr[off + 6] = 0x01;
    mbr[off + 7] = 0x00;
    mbr[off + 8..off + 12].copy_from_slice(&part_lba.to_le_bytes());
    mbr[off + 12..off + 16].copy_from_slice(&part_sectors.to_le_bytes());
}

fn write_fat12_bpb(vbr: &mut [u8], bpb: Fat12Bpb, total_sectors: u16) {
    // jmp short + nop
    vbr[0] = 0xEB;
    vbr[1] = 0x3C;
    vbr[2] = 0x90;
    // OEM name is 8 bytes at offset 0x03 (MS-DOS FAT BPB).
    vbr[3..11].copy_from_slice(b"MSDOS5.0");
    vbr[0x0B..0x0D].copy_from_slice(&bpb.bytes_per_sector.to_le_bytes());
    vbr[0x0D] = bpb.sectors_per_cluster;
    vbr[0x0E..0x10].copy_from_slice(&bpb.reserved_sectors.to_le_bytes());
    vbr[0x10] = bpb.fat_count;
    vbr[0x11..0x13].copy_from_slice(&bpb.root_entry_count.to_le_bytes());
    vbr[0x13..0x15].copy_from_slice(&total_sectors.to_le_bytes());
    vbr[0x15] = 0xF8; // media
    vbr[0x16..0x18].copy_from_slice(&bpb.sectors_per_fat.to_le_bytes());
    vbr[0x18..0x1A].copy_from_slice(&63u16.to_le_bytes()); // spt decorative
    vbr[0x1A..0x1C].copy_from_slice(&16u16.to_le_bytes()); // heads decorative
    vbr[510] = MBR_SIGNATURE_LO;
    vbr[511] = MBR_SIGNATURE_HI;
}

fn write_dirent(root: &mut [u8], index: usize, name83: &[u8; 11], cluster: u16, size: u32) {
    let off = index * FAT_DIRENT_SIZE;
    root[off..off + 11].copy_from_slice(name83);
    root[off + 11] = 0x20; // archive
    root[off + 26..off + 28].copy_from_slice(&cluster.to_le_bytes());
    root[off + 28..off + 32].copy_from_slice(&size.to_le_bytes());
}

/// FreeDOS-*like* INT19 HD with a minimal FAT12 volume + `KERNEL.SYS` root name.
///
/// Layout (partition at LBA 1):
/// - VBR with valid FAT12 BPB (1 reserved, 2 FATs × 1 sector, 16 root entries)
/// - Stub VBR code: print `FD` + HLT (same spirit as R14 stub)
/// - Root contains `KERNEL.SYS` (cluster 2, small size) — **not** loaded/executed
///
/// Still **not** a FreeDOS prompt.
pub fn synthetic_int19_freedos_fat12_hd() -> Vec<u8> {
    const PART_LBA: u32 = 1;
    // reserved(1) + 2*fat(1) + root(1) + data(4) = 8 sectors in partition
    const PART_SECTORS: u32 = 8;
    let total = (1 + PART_SECTORS as usize) * MBR_SECTOR_SIZE;
    let mut img = vec![0u8; total];

    let mut mbr = [0x90u8; MBR_SECTOR_SIZE];
    mbr[0] = 0xF4;
    mbr[1..5].copy_from_slice(b"FD12");
    write_part0_active(&mut mbr, PART_LBA, PART_SECTORS, MBR_PART_TYPE_FAT12);
    mbr[510] = MBR_SIGNATURE_LO;
    mbr[511] = MBR_SIGNATURE_HI;
    img[..MBR_SECTOR_SIZE].copy_from_slice(&mbr);

    let bpb = Fat12Bpb {
        bytes_per_sector: 512,
        sectors_per_cluster: 1,
        reserved_sectors: 1,
        fat_count: 2,
        root_entry_count: 16,
        sectors_per_fat: 1,
    };
    let mut vbr = [0u8; MBR_SECTOR_SIZE];
    write_fat12_bpb(&mut vbr, bpb, PART_SECTORS as u16);
    // Overlay tiny payload after BPB jump target area: COM1 "FD" + HLT.
    // Keep BPB intact (bytes 0x0B+); place code at 0x3E (after classic BPB).
    let code: &[u8] = &[
        0xBA, 0xF8, 0x03, // mov dx, 0x03F8
        0xB0, b'F', //
        0xEE, //
        0xB0, b'D', //
        0xEE, //
        0xF4, // hlt
    ];
    vbr[0x3E..0x3E + code.len()].copy_from_slice(code);
    // jmp at start already points near 0x3E (EB 3C → IP after jmp = 0x02+0x3C=0x3E).
    let vbr_off = PART_LBA as usize * MBR_SECTOR_SIZE;
    img[vbr_off..vbr_off + MBR_SECTOR_SIZE].copy_from_slice(&vbr);

    // FAT1 / FAT2 media ID + EOF for cluster 2
    let fat_off = vbr_off + MBR_SECTOR_SIZE; // reserved=1 → first FAT
                                             // FAT12: two reserved clusters + cluster2 = EOF (0xFFF)
                                             // bytes: F8 FF FF (media+EOC cluster1) then FF 0F for cluster2 EOC packed...
                                             // cluster 0 = 0xFF8, cluster 1 = 0xFFF, cluster 2 = 0xFFF
                                             // packed: F8 FF FF FF 0F
    img[fat_off] = 0xF8;
    img[fat_off + 1] = 0xFF;
    img[fat_off + 2] = 0xFF;
    img[fat_off + 3] = 0xFF;
    img[fat_off + 4] = 0x0F;
    let fat2_off = fat_off + MBR_SECTOR_SIZE;
    let fat_bytes = [
        img[fat_off],
        img[fat_off + 1],
        img[fat_off + 2],
        img[fat_off + 3],
        img[fat_off + 4],
    ];
    img[fat2_off..fat2_off + 5].copy_from_slice(&fat_bytes);

    // Root directory (1 sector for 16 entries)
    let root_off = fat2_off + MBR_SECTOR_SIZE;
    write_dirent(
        &mut img[root_off..root_off + MBR_SECTOR_SIZE],
        0,
        FAT12_NAME_KERNEL_SYS,
        2,
        12,
    );

    // Cluster 2 data: marker only (not executed).
    let data_off = root_off + MBR_SECTOR_SIZE;
    let marker = b"KERNEL.SYS\0";
    img[data_off..data_off + marker.len()].copy_from_slice(marker);
    img
}

impl Machine {
    /// Attach [`synthetic_int19_freedos_fat12_hd`] (FAT12 + `KERNEL.SYS` name).
    pub fn attach_freedos_fat12_hd_for_int19(&mut self) {
        self.attach_ide_image(synthetic_int19_freedos_fat12_hd());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bpb_from_synthetic_fat12() {
        let img = synthetic_int19_freedos_fat12_hd();
        let vbr = &img[MBR_SECTOR_SIZE..2 * MBR_SECTOR_SIZE];
        let bpb = parse_fat12_bpb(vbr).expect("bpb");
        assert_eq!(bpb.bytes_per_sector, 512);
        assert_eq!(bpb.root_entry_count, 16);
        assert_eq!(bpb.root_dir_lba_offset(), 1 + 2); // reserved + 2 fats
    }

    #[test]
    fn locate_kernel_sys_on_synthetic_fat12() {
        let img = synthetic_int19_freedos_fat12_hd();
        match locate_freedos_kernel_on_image(&img) {
            Fat12KernelLocate::KernelSysPresent { entry } => {
                assert!(entry.is_kernel_sys());
                assert_eq!(entry.first_cluster, 2);
                assert_eq!(entry.size, 12);
            }
            other => panic!("unexpected {other}"),
        }
    }

    #[test]
    fn locate_missing_on_r14_stub() {
        use crate::boot_media::synthetic_int19_freedos_stub_hd;
        let img = synthetic_int19_freedos_stub_hd();
        assert_eq!(
            locate_freedos_kernel_on_image(&img),
            Fat12KernelLocate::BadBpb
        );
    }

    #[test]
    fn machine_attach_and_locate() {
        let mut m = Machine::new(64 * 1024);
        m.attach_freedos_fat12_hd_for_int19();
        assert!(locate_freedos_kernel_on_machine(&m).name_found());
    }
}
