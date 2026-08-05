//! Hard-disk / floppy boot-sector handoff helpers (MBR → `0x7C00`).
//!
//! Isolated from MachineBus / port wiring so parallel fw_cfg / port-92 slices
//! can merge without fighting this file.
//!
//! Spec: IBM PC BIOS INT 19h / OSDev "Boot Sequence" — load first sector to
//! physical `0x7C00`, require `0x55AA` signature, jump `CS:IP = 0000:7C00`.

use crate::{Machine, MachineError};
use x86_core::SegmentReg;

/// Physical load address for the classic PC boot sector.
pub const MBR_PHYS_ADDR: u64 = 0x7C00;

/// Boot sector size (ATA / floppy sector).
pub const MBR_SECTOR_SIZE: usize = 512;

/// Valid MBR / VBR signature bytes at offsets 510–511 (little-endian `0xAA55`).
pub const MBR_SIGNATURE_LO: u8 = 0x55;
pub const MBR_SIGNATURE_HI: u8 = 0xAA;

impl Machine {
    /// Attach a raw disk image to the primary IDE master.
    ///
    /// Wraps [`devices::IdePrimary::attach_image`]. Image should be a multiple
    /// of 512 bytes for ATA LBA semantics; short images are still attached.
    pub fn attach_ide_image(&mut self, image: Vec<u8>) {
        self.ide.attach_image(image);
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
    pub fn load_mbr_to_7c00(&mut self) -> Result<(), MachineError> {
        let need = (MBR_PHYS_ADDR as usize)
            .checked_add(MBR_SECTOR_SIZE)
            .ok_or(MachineError::MbrRamTooSmall)?;
        if self.mem.ram_len() < need {
            return Err(MachineError::MbrRamTooSmall);
        }

        let sector = self
            .read_boot_sector_lba0()?
            .ok_or(MachineError::NoBootMedia)?;

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
    fn read_boot_sector_lba0(&self) -> Result<Option<[u8; MBR_SECTOR_SIZE]>, MachineError> {
        if self.ide.present {
            if self.ide.image.len() < MBR_SECTOR_SIZE {
                return Err(MachineError::IncompleteBootSector);
            }
            let mut sector = [0u8; MBR_SECTOR_SIZE];
            sector.copy_from_slice(&self.ide.image[..MBR_SECTOR_SIZE]);
            return Ok(Some(sector));
        }

        Ok(self.fdc.read_sector(0, 0, 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn attach_ide_image_and_with_ide_helpers() {
        let mut m = Machine::new(64 * 1024);
        assert!(!m.ide.present);
        m.attach_ide_image(vec![0xAAu8; MBR_SECTOR_SIZE]);
        assert!(m.ide.present);
        assert_eq!(m.ide.image.len(), MBR_SECTOR_SIZE);

        let m2 = Machine::with_ide(64 * 1024, vec![0x55u8; MBR_SECTOR_SIZE * 2]);
        assert!(m2.ide.present);
        assert_eq!(m2.ide.image.len(), MBR_SECTOR_SIZE * 2);
    }
}
