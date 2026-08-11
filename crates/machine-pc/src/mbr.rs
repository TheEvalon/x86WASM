//! Hard-disk / floppy boot-sector handoff helpers (MBR → `0x7C00`).
//!
//! Isolated from MachineBus / port wiring so parallel fw_cfg / port-92 slices
//! can merge without fighting this file.
//!
//! Spec: IBM PC BIOS INT 19h / OSDev "Boot Sequence" — load first sector to
//! physical `0x7C00`, require `0x55AA` signature, jump `CS:IP = 0000:7C00`.
//! R14 also models the classic MBR→active-partition VBR chain as a **host**
//! handoff (`load_active_vbr_to_7c00`) — not a claim of SeaBIOS INT 19h success.

use crate::boot_media::{MBR_PART0_OFF, MBR_PART_BOOTABLE};
use crate::{Machine, MachineError};
use x86_core::SegmentReg;

/// Physical load address for the classic PC boot sector.
pub const MBR_PHYS_ADDR: u64 = 0x7C00;

/// Boot sector size (ATA / floppy sector).
pub const MBR_SECTOR_SIZE: usize = 512;

/// Valid MBR / VBR signature bytes at offsets 510–511 (little-endian `0xAA55`).
pub const MBR_SIGNATURE_LO: u8 = 0x55;
pub const MBR_SIGNATURE_HI: u8 = 0xAA;

/// Active partition entry discovered in a classic MBR.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActivePartition {
    /// Partition table slot `0..3`.
    pub index: u8,
    /// Partition system ID (byte 4 of the entry).
    pub part_type: u8,
    /// Starting LBA (little-endian dword at entry + 8).
    pub start_lba: u32,
    /// Sector count (little-endian dword at entry + 12).
    pub sector_count: u32,
}

/// Scan MBR bytes for the first active (`80h`) partition entry.
///
/// Spec: IBM PC / OSDev MBR — boot indicator `80h` at offset `0x1BE + 16*i`.
/// Returns `None` when signature is wrong or no active entry exists.
pub fn find_active_partition(mbr: &[u8]) -> Option<ActivePartition> {
    if mbr.len() < MBR_SECTOR_SIZE {
        return None;
    }
    if mbr[510] != MBR_SIGNATURE_LO || mbr[511] != MBR_SIGNATURE_HI {
        return None;
    }
    for i in 0..4u8 {
        let off = MBR_PART0_OFF + usize::from(i) * 16;
        if mbr[off] != MBR_PART_BOOTABLE {
            continue;
        }
        let part_type = mbr[off + 4];
        let start_lba =
            u32::from_le_bytes([mbr[off + 8], mbr[off + 9], mbr[off + 10], mbr[off + 11]]);
        let sector_count =
            u32::from_le_bytes([mbr[off + 12], mbr[off + 13], mbr[off + 14], mbr[off + 15]]);
        return Some(ActivePartition {
            index: i,
            part_type,
            start_lba,
            sector_count,
        });
    }
    None
}

impl Machine {
    /// Attach a raw disk image to the primary IDE master.
    ///
    /// Wraps [`devices::IdePrimary::attach_image`]. Image should be a multiple
    /// of 512 bytes for ATA LBA semantics; short images are still attached.
    ///
    /// Also records the capacity and re-derives the firmware configuration, so
    /// the CMOS fixed-disk bytes (`12h`, `19h`, `1Bh`-`23h`) describe the disk
    /// that is actually there. Attaching straight to [`Machine::ide`] still
    /// answers IDENTIFY but leaves CMOS reporting no fixed disk; this is the
    /// supported path.
    pub fn attach_ide_image(&mut self, image: Vec<u8>) {
        self.record_ide_disk_capacity(image.len());
        self.ide.attach_image(image);
        self.sync_firmware_configuration();
    }

    /// Construct a machine with a primary IDE image already attached.
    ///
    /// Wraps [`Self::new`] + [`Self::attach_ide_image`].
    pub fn with_ide(ram_size: usize, image: Vec<u8>) -> Self {
        let mut m = Self::new(ram_size);
        m.attach_ide_image(image);
        m
    }

    /// Load LBA 0 from attached boot media into phys [`MBR_PHYS_ADDR`] and set
    /// `CS:IP` to `0000:7C00` for a real-mode boot handoff.
    ///
    /// Prefers primary IDE (`IdePrimary` image LBA 0) when present with at least
    /// 512 bytes. Falls back to floppy CHS `(0,0,1)` when IDE has no usable
    /// sector. Requires a classic `0x55AA` signature at bytes 510–511.
    ///
    /// Spec: IBM PC BIOS INT 19h / OSDev Boot Sequence — MBR at `0000:7C00`.
    /// Does **not** run SeaBIOS POST or INT 13h; host-side media→RAM copy only.
    /// For an explicit floppy-first handoff even when IDE is attached, use
    /// [`Self::load_floppy_boot_to_7c00`].
    pub fn load_mbr_to_7c00(&mut self) -> Result<(), MachineError> {
        let sector = self
            .read_boot_sector_ide_prefer()?
            .ok_or(MachineError::NoBootMedia)?;
        self.install_boot_sector_at_7c00(&sector)
    }

    /// Load the **active partition VBR** into phys [`MBR_PHYS_ADDR`] and set
    /// `CS:IP = 0000:7C00`.
    ///
    /// Host-side model of the classic MBR→VBR chain: parse LBA0 for the first
    /// active (`80h`) partition, copy that partition's first sector (VBR) to
    /// `0x7C00`, require `0x55AA`. IDE primary only (floppy has no MBR table).
    ///
    /// Spec: IBM PC BIOS / OSDev Boot Sequence — after MBR, the active partition
    /// boot sector is loaded to `0000:7C00`. This is **not** SeaBIOS INT 19h
    /// success and does not execute MBR code.
    pub fn load_active_vbr_to_7c00(&mut self) -> Result<ActivePartition, MachineError> {
        if !self.ide.present || self.ide.image.len() < MBR_SECTOR_SIZE {
            return Err(MachineError::NoBootMedia);
        }
        let mbr = &self.ide.image[..MBR_SECTOR_SIZE];
        if mbr[510] != MBR_SIGNATURE_LO || mbr[511] != MBR_SIGNATURE_HI {
            return Err(MachineError::InvalidMbrSignature);
        }
        let part = find_active_partition(mbr).ok_or(MachineError::NoActivePartition)?;
        let sector = self
            .read_ide_lba_sector(part.start_lba)?
            .ok_or(MachineError::IncompletePartitionSector)?;
        self.install_boot_sector_at_7c00(&sector)?;
        Ok(part)
    }

    /// Load floppy CHS `(0,0,1)` into phys [`MBR_PHYS_ADDR`] and set
    /// `CS:IP = 0000:7C00`, **ignoring** any attached IDE image.
    ///
    /// Spec: IBM PC BIOS INT 19h floppy boot path — first floppy sector to
    /// `0000:7C00` with `0x55AA`. Host-side only (not INT 13h AH=02h on `DL=00h`).
    pub fn load_floppy_boot_to_7c00(&mut self) -> Result<(), MachineError> {
        let sector = self
            .read_floppy_boot_sector()?
            .ok_or(MachineError::NoBootMedia)?;
        self.install_boot_sector_at_7c00(&sector)
    }

    fn install_boot_sector_at_7c00(
        &mut self,
        sector: &[u8; MBR_SECTOR_SIZE],
    ) -> Result<(), MachineError> {
        let need = (MBR_PHYS_ADDR as usize)
            .checked_add(MBR_SECTOR_SIZE)
            .ok_or(MachineError::MbrRamTooSmall)?;
        if self.mem.ram_len() < need {
            return Err(MachineError::MbrRamTooSmall);
        }

        if sector[510] != MBR_SIGNATURE_LO || sector[511] != MBR_SIGNATURE_HI {
            return Err(MachineError::InvalidMbrSignature);
        }

        for (i, byte) in sector.iter().enumerate() {
            self.mem
                .write_u8(MBR_PHYS_ADDR + i as u64, *byte)
                .map_err(|_| MachineError::MbrRamTooSmall)?;
        }

        // Spec: IBM PC BIOS — far jump to 0000:7C00 after loading the sector.
        self.cpu.cs = SegmentReg::real_mode_code(0x0000);
        self.cpu.set_ip16(0x7C00);
        Ok(())
    }

    /// Prefer IDE LBA0; else floppy cylinder 0 / head 0 / sector 1.
    fn read_boot_sector_ide_prefer(&self) -> Result<Option<[u8; MBR_SECTOR_SIZE]>, MachineError> {
        if self.ide.present {
            if self.ide.image.len() < MBR_SECTOR_SIZE {
                return Err(MachineError::IncompleteBootSector);
            }
            let mut sector = [0u8; MBR_SECTOR_SIZE];
            sector.copy_from_slice(&self.ide.image[..MBR_SECTOR_SIZE]);
            return Ok(Some(sector));
        }

        self.read_floppy_boot_sector()
    }

    /// Floppy boot sector at CHS `(0,0,1)` when media is attached.
    fn read_floppy_boot_sector(&self) -> Result<Option<[u8; MBR_SECTOR_SIZE]>, MachineError> {
        Ok(self.fdc.read_sector(0, 0, 1))
    }

    /// Read one 512-byte IDE LBA when the image covers it.
    fn read_ide_lba_sector(&self, lba: u32) -> Result<Option<[u8; MBR_SECTOR_SIZE]>, MachineError> {
        if !self.ide.present {
            return Ok(None);
        }
        let off = (lba as usize)
            .checked_mul(MBR_SECTOR_SIZE)
            .ok_or(MachineError::IncompletePartitionSector)?;
        let end = off
            .checked_add(MBR_SECTOR_SIZE)
            .ok_or(MachineError::IncompletePartitionSector)?;
        if end > self.ide.image.len() {
            return Err(MachineError::IncompletePartitionSector);
        }
        let mut sector = [0u8; MBR_SECTOR_SIZE];
        sector.copy_from_slice(&self.ide.image[off..end]);
        Ok(Some(sector))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boot_media::{
        synthetic_int19_bootable_hd, synthetic_int19_freedos_stub_hd, MBR_PART_TYPE_FAT12,
    };
    use devices::FDC_1440_IMAGE_SIZE;

    fn synthetic_mbr(fill: u8) -> Vec<u8> {
        let mut sector = vec![fill; MBR_SECTOR_SIZE];
        sector[0] = 0xF4; // HLT — boot handoff smoke
        sector[510] = MBR_SIGNATURE_LO;
        sector[511] = MBR_SIGNATURE_HI;
        sector
    }

    /// Spec: IBM PC BIOS INT 19h — IDE LBA0 → `0x7C00`, `CS:IP = 0000:7C00`, `0x55AA`.
    #[test]
    fn load_mbr_to_7c00_from_ide_sets_cs_ip_and_memory() {
        let mbr = synthetic_mbr(0x90);
        let mut m = Machine::with_ide(64 * 1024, mbr.clone());

        m.load_mbr_to_7c00().expect("IDE MBR load");

        assert_eq!(m.cpu.cs.selector, 0x0000);
        assert_eq!(m.cpu.cs.base, 0);
        assert_eq!(m.cpu.ip16(), 0x7C00);
        for (i, expected) in mbr.iter().enumerate() {
            assert_eq!(
                m.mem.read_u8(MBR_PHYS_ADDR + i as u64).unwrap(),
                *expected,
                "byte {i}"
            );
        }
        assert_eq!(m.mem.read_u8(0x7C00 + 510).unwrap(), 0x55);
        assert_eq!(m.mem.read_u8(0x7C00 + 511).unwrap(), 0xAA);
    }

    /// Spec: IBM PC — after handoff, guest fetch at `0000:7C00` runs the sector.
    #[test]
    fn load_mbr_to_7c00_handoff_executes_hlt() {
        let mut m = Machine::with_ide(64 * 1024, synthetic_mbr(0x90));
        m.load_mbr_to_7c00().unwrap();
        assert!(!m.cpu.halted);
        m.step().expect("HLT at 7C00");
        assert!(m.cpu.halted);
    }

    /// Spec: classic MBR signature check — reject sector without `0x55AA`.
    #[test]
    fn load_mbr_to_7c00_rejects_bad_signature() {
        let mut bad = vec![0x90u8; MBR_SECTOR_SIZE];
        bad[510] = 0x00;
        bad[511] = 0x00;
        let mut m = Machine::with_ide(64 * 1024, bad);
        assert!(matches!(
            m.load_mbr_to_7c00(),
            Err(MachineError::InvalidMbrSignature)
        ));
    }

    #[test]
    fn load_mbr_to_7c00_rejects_no_media() {
        let mut m = Machine::new(64 * 1024);
        assert!(matches!(
            m.load_mbr_to_7c00(),
            Err(MachineError::NoBootMedia)
        ));
    }

    #[test]
    fn load_mbr_to_7c00_rejects_short_ide_image() {
        let mut m = Machine::new(64 * 1024);
        m.attach_ide_image(vec![0x55, 0xAA]);
        assert!(matches!(
            m.load_mbr_to_7c00(),
            Err(MachineError::IncompleteBootSector)
        ));
    }

    #[test]
    fn load_mbr_to_7c00_rejects_tiny_ram() {
        let mut m = Machine::with_ide(0x7C00, synthetic_mbr(0x00));
        assert!(matches!(
            m.load_mbr_to_7c00(),
            Err(MachineError::MbrRamTooSmall)
        ));
    }

    /// Spec: prefer primary IDE LBA0 when both IDE and floppy media are attached.
    #[test]
    fn load_mbr_to_7c00_prefers_ide_over_floppy() {
        let ide_mbr = synthetic_mbr(0x11);
        let mut floppy = vec![0x22u8; FDC_1440_IMAGE_SIZE];
        floppy[..MBR_SECTOR_SIZE].copy_from_slice(&synthetic_mbr(0x22));

        let mut m = Machine::with_ide(64 * 1024, ide_mbr.clone());
        m.attach_floppy_image(floppy).expect("floppy");
        m.load_mbr_to_7c00().unwrap();

        assert_eq!(m.mem.read_u8(0x7C00 + 1).unwrap(), 0x11);
        assert_ne!(m.mem.read_u8(0x7C00 + 1).unwrap(), 0x22);
    }

    /// Spec: IBM PC floppy boot — CHS (0,0,1) when IDE is absent.
    #[test]
    fn load_mbr_to_7c00_from_floppy_when_no_ide() {
        let mut floppy = vec![0u8; FDC_1440_IMAGE_SIZE];
        let mbr = synthetic_mbr(0x33);
        floppy[..MBR_SECTOR_SIZE].copy_from_slice(&mbr);

        let mut m = Machine::with_floppy(64 * 1024, floppy).expect("floppy");
        m.load_mbr_to_7c00().expect("floppy MBR");

        assert_eq!(m.cpu.cs.selector, 0);
        assert_eq!(m.cpu.ip16(), 0x7C00);
        assert_eq!(m.mem.read_u8(0x7C00).unwrap(), 0xF4);
        assert_eq!(m.mem.read_u8(0x7C00 + 1).unwrap(), 0x33);
        assert_eq!(m.mem.read_u8(0x7DFE).unwrap(), 0x55);
        assert_eq!(m.mem.read_u8(0x7DFF).unwrap(), 0xAA);
    }

    /// Spec: explicit floppy handoff ignores attached IDE (INT 19h floppy-first).
    #[test]
    fn load_floppy_boot_to_7c00_ignores_ide() {
        let ide_mbr = synthetic_mbr(0x11);
        let mut floppy = vec![0x22u8; FDC_1440_IMAGE_SIZE];
        floppy[..MBR_SECTOR_SIZE].copy_from_slice(&synthetic_mbr(0x44));

        let mut m = Machine::with_ide(64 * 1024, ide_mbr);
        m.attach_floppy_image(floppy).expect("floppy");
        m.load_floppy_boot_to_7c00().expect("floppy-first");

        assert_eq!(m.cpu.cs.selector, 0);
        assert_eq!(m.cpu.ip16(), 0x7C00);
        assert_eq!(m.mem.read_u8(0x7C00 + 1).unwrap(), 0x44);
        assert_ne!(m.mem.read_u8(0x7C00 + 1).unwrap(), 0x11);
        assert_eq!(m.mem.read_u8(0x7DFE).unwrap(), 0x55);
    }

    #[test]
    fn load_floppy_boot_to_7c00_rejects_no_floppy() {
        let mut m = Machine::with_ide(64 * 1024, synthetic_mbr(0x11));
        assert!(matches!(
            m.load_floppy_boot_to_7c00(),
            Err(MachineError::NoBootMedia)
        ));
    }

    #[test]
    fn load_floppy_boot_to_7c00_rejects_bad_signature() {
        let mut floppy = vec![0u8; FDC_1440_IMAGE_SIZE];
        floppy[510] = 0x00;
        floppy[511] = 0x00;
        let mut m = Machine::with_floppy(64 * 1024, floppy).expect("floppy");
        assert!(matches!(
            m.load_floppy_boot_to_7c00(),
            Err(MachineError::InvalidMbrSignature)
        ));
    }

    /// Spec: OSDev Boot Sequence — active partition VBR → `0x7C00` (host chain).
    #[test]
    fn load_active_vbr_to_7c00_from_int19_hd() {
        let mut m = Machine::with_ide(64 * 1024, synthetic_int19_bootable_hd());
        let part = m.load_active_vbr_to_7c00().expect("vbr");
        assert_eq!(part.start_lba, 1);
        assert_eq!(part.part_type, MBR_PART_TYPE_FAT12);
        assert_eq!(m.cpu.cs.selector, 0);
        assert_eq!(m.cpu.ip16(), 0x7C00);
        // VBR is HLT (not MBR marker "INT1" at offset 1).
        assert_eq!(m.mem.read_u8(0x7C00).unwrap(), 0xF4);
        assert_ne!(m.mem.read_u8(0x7C00 + 1).unwrap(), b'I');
        assert_eq!(m.mem.read_u8(0x7DFE).unwrap(), 0x55);
        m.step().expect("VBR HLT");
        assert!(m.cpu.halted);
    }

    /// Spec: FreeDOS-like stub VBR at LBA1 prints `FD` then HLT when chained.
    #[test]
    fn load_active_vbr_freedos_stub_executes_fd_banner() {
        let mut m = Machine::with_ide(64 * 1024, synthetic_int19_freedos_stub_hd());
        m.load_active_vbr_to_7c00().expect("vbr");
        for _ in 0..32 {
            if m.cpu.halted {
                break;
            }
            m.step().expect("step");
        }
        assert!(m.cpu.halted);
        assert_eq!(m.com1_text(), "FD");
    }

    #[test]
    fn load_active_vbr_rejects_signature_only_mbr() {
        let mut img = synthetic_mbr(0x90);
        img.resize(2 * MBR_SECTOR_SIZE, 0);
        let mut m = Machine::with_ide(64 * 1024, img);
        assert!(matches!(
            m.load_active_vbr_to_7c00(),
            Err(MachineError::NoActivePartition)
        ));
    }

    #[test]
    fn find_active_partition_reads_lba1() {
        let img = synthetic_int19_bootable_hd();
        let part = find_active_partition(&img).expect("active");
        assert_eq!(part.index, 0);
        assert_eq!(part.start_lba, 1);
    }
}
