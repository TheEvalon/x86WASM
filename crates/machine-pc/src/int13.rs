//! Host-side IBM BIOS INT 13h hard-disk subset (AH=02 read at minimum).
//!
//! Closest approach in-tree to SeaBIOS disk services: a **host** dispatcher that
//! applies classic INT 13h register conventions against the primary IDE image,
//! mirroring [`crate::mbr::Machine::load_mbr_to_7c00`]'s host-side media path.
//! This is **not** a guest IVT BIOS, not CHS translation modes, and not floppy
//! INT 13h (see [`crate::mbr`] for floppy → `0x7C00` handoff).
//!
//! Spec: IBM PC BIOS INT 13h Disk Services (AH=00h reset, AH=02h read sectors,
//! AH=08h get drive parameters); ATA IDENTIFY obsolete geometry 16 heads / 63
//! sectors-per-track (matches `IdePrimary` IDENTIFY words 3/6).

use crate::{Machine, MachineError};
use x86_core::CpuState;

/// First hard disk (IBM BIOS `DL`).
pub const INT13_DRIVE_HD0: u8 = 0x80;

/// AH=00h — reset disk system.
pub const INT13_AH_RESET: u8 = 0x00;
/// AH=02h — read disk sectors into `ES:BX`.
pub const INT13_AH_READ: u8 = 0x02;
/// AH=08h — get drive parameters.
pub const INT13_AH_GET_DRIVE_PARAMS: u8 = 0x08;

/// Success status in `AH` with `CF` clear.
pub const INT13_STATUS_OK: u8 = 0x00;
/// Invalid command / unsupported function / bad drive.
pub const INT13_STATUS_INVALID: u8 = 0x01;
/// Sector not found / address beyond media.
pub const INT13_STATUS_SECTOR_NOT_FOUND: u8 = 0x04;
/// Drive not ready (no attached IDE image).
pub const INT13_STATUS_TIMEOUT: u8 = 0x80;

/// Heads matching IDE IDENTIFY obsolete word 3.
pub const INT13_HD_HEADS: u16 = 16;
/// Sectors per track matching IDE IDENTIFY obsolete word 6.
pub const INT13_HD_SPT: u16 = 63;

/// ATA / BIOS sector size.
pub const INT13_SECTOR_SIZE: usize = 512;

impl Machine {
    /// Host-side INT 13h hard-disk dispatch using current CPU registers.
    ///
    /// Reads `AH` for the function and `DL` for the drive. Only
    /// [`INT13_DRIVE_HD0`] (`0x80`) is accepted. Sets `AH` / `CF` (and
    /// function-specific outputs) per IBM BIOS INT 13h conventions.
    ///
    /// Spec: IBM PC BIOS INT 13h — host service only; guest `INT 13h` still
    /// requires a real IVT handler (SeaBIOS) or an explicit call into this API.
    pub fn service_int13_hd(&mut self) {
        let dl = self.cpu.gpr_u8_low(CpuState::RDX);
        if dl != INT13_DRIVE_HD0 {
            self.int13_fail(INT13_STATUS_INVALID);
            return;
        }
        match self.cpu.ah() {
            INT13_AH_RESET => self.int13_hd_reset(),
            INT13_AH_READ => self.int13_hd_read_from_regs(),
            INT13_AH_GET_DRIVE_PARAMS => self.int13_hd_get_params(),
            _ => self.int13_fail(INT13_STATUS_INVALID),
        }
    }

    /// Read `count` sectors from primary IDE starting at packed INT 13h CHS
    /// into physical `dest`, without touching CPU registers.
    ///
    /// Spec: IBM PC BIOS INT 13h AH=02h addressing — cylinder in `CH`+`CL[7:6]`,
    /// sector in `CL[5:0]` (1-based), head in `DH`.
    pub fn int13_hd_read_chs_to_phys(
        &mut self,
        cylinder: u16,
        head: u8,
        sector: u8,
        count: u8,
        dest: u64,
    ) -> Result<u8, u8> {
        if !self.ide.present || self.ide.image.is_empty() {
            return Err(INT13_STATUS_TIMEOUT);
        }
        if count == 0 || sector == 0 || u16::from(head) >= INT13_HD_HEADS {
            return Err(INT13_STATUS_INVALID);
        }
        if u16::from(sector) > INT13_HD_SPT {
            return Err(INT13_STATUS_SECTOR_NOT_FOUND);
        }

        let Some(start_lba) = chs_to_lba(cylinder, head, sector) else {
            return Err(INT13_STATUS_SECTOR_NOT_FOUND);
        };
        let total = (self.ide.image.len() / INT13_SECTOR_SIZE) as u64;
        let need = u64::from(count);
        if start_lba.checked_add(need).is_none_or(|end| end > total) {
            return Err(INT13_STATUS_SECTOR_NOT_FOUND);
        }

        let byte_off = (start_lba as usize).saturating_mul(INT13_SECTOR_SIZE);
        let bytes = usize::from(count).saturating_mul(INT13_SECTOR_SIZE);
        let end = dest.checked_add(bytes as u64).ok_or(INT13_STATUS_INVALID)?;
        if end > self.mem.ram_len() as u64 {
            return Err(INT13_STATUS_INVALID);
        }

        for i in 0..bytes {
            let b = self.ide.image[byte_off + i];
            self.mem
                .write_u8(dest + i as u64, b)
                .map_err(|_| INT13_STATUS_INVALID)?;
        }
        Ok(count)
    }
}

impl Machine {
    fn int13_ok_al(&mut self, al: u8) {
        self.cpu.set_ah(INT13_STATUS_OK);
        self.cpu.set_al(al);
        self.cpu.set_cf(false);
    }

    fn int13_fail(&mut self, status: u8) {
        self.cpu.set_ah(status);
        self.cpu.set_cf(true);
    }

    fn int13_hd_reset(&mut self) {
        if !self.ide.present {
            self.int13_fail(INT13_STATUS_TIMEOUT);
            return;
        }
        self.int13_ok_al(0);
    }

    fn int13_hd_read_from_regs(&mut self) {
        let al = self.cpu.al();
        let cx = self.cpu.gpr_u16(CpuState::RCX);
        let dh = self.cpu.gpr_u8(4 + CpuState::RDX); // DH via high-byte index
        let bx = self.cpu.gpr_u16(CpuState::RBX);
        let (cylinder, sector) = unpack_cx(cx);
        let dest = self.cpu.es.base.wrapping_add(u64::from(bx));

        match self.int13_hd_read_chs_to_phys(cylinder, dh, sector, al, dest) {
            Ok(n) => self.int13_ok_al(n),
            Err(status) => {
                self.cpu.set_al(0);
                self.int13_fail(status);
            }
        }
    }

    fn int13_hd_get_params(&mut self) {
        if !self.ide.present {
            self.int13_fail(INT13_STATUS_TIMEOUT);
            return;
        }
        let total = (self.ide.image.len() / INT13_SECTOR_SIZE) as u64;
        // Maximum cylinder addressable with this fixed geometry (0-based).
        let max_cyl = if total == 0 {
            0u16
        } else {
            let spc = u64::from(INT13_HD_HEADS) * u64::from(INT13_HD_SPT);
            ((total.saturating_sub(1)) / spc).min(u64::from(u16::MAX)) as u16
        };
        // AH=08h: CX = max cylinder/sector packed; DH = max head; DL = drive count.
        self.cpu
            .set_gpr_u16(CpuState::RCX, pack_cx(max_cyl, INT13_HD_SPT as u8));
        self.cpu
            .set_gpr_u8(4 + CpuState::RDX, (INT13_HD_HEADS - 1) as u8);
        self.cpu.set_gpr_u8_low(CpuState::RDX, 1); // one HD
        self.cpu.set_ah(INT13_STATUS_OK);
        self.cpu.set_cf(false);
    }
}

/// Pack cylinder + 1-based sector into INT 13h `CX`.
pub fn pack_cx(cylinder: u16, sector: u8) -> u16 {
    let cyl_hi = (cylinder >> 8) & 0x03;
    let cyl_lo = cylinder & 0xFF;
    let sec = u16::from(sector & 0x3F);
    (cyl_lo << 8) | (cyl_hi << 6) | sec
}

/// Unpack INT 13h `CX` into (cylinder, sector).
pub fn unpack_cx(cx: u16) -> (u16, u8) {
    let sector = (cx & 0x3F) as u8;
    let cyl_lo = (cx >> 8) & 0xFF;
    let cyl_hi = (cx >> 6) & 0x03;
    let cylinder = (cyl_hi << 8) | cyl_lo;
    (cylinder, sector)
}

/// CHS → LBA using fixed 16/63 geometry (sector is 1-based).
pub fn chs_to_lba(cylinder: u16, head: u8, sector: u8) -> Option<u64> {
    if sector == 0 || u16::from(head) >= INT13_HD_HEADS || u16::from(sector) > INT13_HD_SPT {
        return None;
    }
    let lba = (u64::from(cylinder) * u64::from(INT13_HD_HEADS) + u64::from(head))
        * u64::from(INT13_HD_SPT)
        + u64::from(sector - 1);
    Some(lba)
}

/// Convenience: set up INT 13h AH=02h registers for a hard-disk read.
pub fn setup_int13_hd_read(
    cpu: &mut CpuState,
    cylinder: u16,
    head: u8,
    sector: u8,
    count: u8,
    es: u16,
    bx: u16,
) {
    cpu.set_ah(INT13_AH_READ);
    cpu.set_al(count);
    cpu.set_gpr_u16(CpuState::RCX, pack_cx(cylinder, sector));
    cpu.set_gpr_u8_low(CpuState::RDX, INT13_DRIVE_HD0);
    cpu.set_gpr_u8(4 + CpuState::RDX, head);
    cpu.set_gpr_u16(CpuState::RBX, bx);
    cpu.es = x86_core::SegmentReg::real_mode(es);
}

impl Machine {
    /// Install a real-mode IVT entry for vector `0x13` that points at `handler`.
    ///
    /// Does **not** install a BIOS body — only the far pointer. Host harnesses
    /// that want disk reads must call [`Self::service_int13_hd`] explicitly
    /// (or use SeaBIOS). Spec: IBM PC IVT — `0x13 * 4` holds `offset:segment`.
    pub fn install_int13_ivt_pointer(
        &mut self,
        handler_seg: u16,
        handler_off: u16,
    ) -> Result<(), MachineError> {
        let base = 0x13u64 * 4;
        self.mem
            .write_u8(base, (handler_off & 0xFF) as u8)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        self.mem
            .write_u8(base + 1, (handler_off >> 8) as u8)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        self.mem
            .write_u8(base + 2, (handler_seg & 0xFF) as u8)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        self.mem
            .write_u8(base + 3, (handler_seg >> 8) as u8)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mbr::{MBR_PHYS_ADDR, MBR_SECTOR_SIZE, MBR_SIGNATURE_HI, MBR_SIGNATURE_LO};

    fn synthetic_disk(sectors: usize) -> Vec<u8> {
        let mut img = vec![0u8; sectors * INT13_SECTOR_SIZE];
        // LBA0 = boot-ish pattern
        img[0] = 0xF4;
        img[510] = MBR_SIGNATURE_LO;
        img[511] = MBR_SIGNATURE_HI;
        // LBA1 marker
        if sectors > 1 {
            img[INT13_SECTOR_SIZE] = 0xA5;
            img[INT13_SECTOR_SIZE + 1] = 0x5A;
        }
        img
    }

    fn cf(cpu: &CpuState) -> bool {
        cpu.rflags & 1 != 0
    }

    /// Spec: IBM BIOS INT 13h AH=02h — CHS (0,0,1) reads LBA0 into ES:BX.
    #[test]
    fn int13_ah02_reads_lba0_into_es_bx() {
        let mut m = Machine::with_ide(64 * 1024, synthetic_disk(4));
        setup_int13_hd_read(&mut m.cpu, 0, 0, 1, 1, 0x0000, 0x7C00);
        m.service_int13_hd();

        assert!(!cf(&m.cpu), "CF clear on success");
        assert_eq!(m.cpu.ah(), INT13_STATUS_OK);
        assert_eq!(m.cpu.al(), 1);
        assert_eq!(m.mem.read_u8(MBR_PHYS_ADDR).unwrap(), 0xF4);
        assert_eq!(m.mem.read_u8(MBR_PHYS_ADDR + 510).unwrap(), 0x55);
        assert_eq!(m.mem.read_u8(MBR_PHYS_ADDR + 511).unwrap(), 0xAA);
    }

    /// Spec: multi-sector AH=02h copies consecutive LBAs.
    #[test]
    fn int13_ah02_reads_two_sectors() {
        let mut m = Machine::with_ide(64 * 1024, synthetic_disk(4));
        setup_int13_hd_read(&mut m.cpu, 0, 0, 1, 2, 0x0000, 0x8000);
        m.service_int13_hd();
        assert!(!cf(&m.cpu));
        assert_eq!(m.cpu.al(), 2);
        assert_eq!(m.mem.read_u8(0x8000).unwrap(), 0xF4);
        assert_eq!(m.mem.read_u8(0x8200).unwrap(), 0xA5);
        assert_eq!(m.mem.read_u8(0x8201).unwrap(), 0x5A);
    }

    /// Spec: sector 0 is invalid (BIOS sectors are 1-based); OOB LBA → not found.
    #[test]
    fn int13_ah02_rejects_sector_zero_and_oob_lba() {
        let mut m = Machine::with_ide(64 * 1024, synthetic_disk(4));
        setup_int13_hd_read(&mut m.cpu, 0, 0, 0, 1, 0x0000, 0x7C00);
        // pack_cx masks sector to 6 bits; force CL sector field to 0.
        m.cpu.set_gpr_u16(CpuState::RCX, pack_cx(0, 0));
        m.service_int13_hd();
        assert!(cf(&m.cpu));
        assert_eq!(m.cpu.ah(), INT13_STATUS_INVALID);

        assert_eq!(
            m.int13_hd_read_chs_to_phys(0, 0, 64, 1, 0x7C00),
            Err(INT13_STATUS_SECTOR_NOT_FOUND)
        );
        // Past end of a 4-sector image.
        assert_eq!(
            m.int13_hd_read_chs_to_phys(0, 0, 1, 8, 0x7C00),
            Err(INT13_STATUS_SECTOR_NOT_FOUND)
        );
    }

    /// Spec: no media → timeout / not ready (`AH=80h`).
    #[test]
    fn int13_ah02_no_media_sets_timeout() {
        let mut m = Machine::new(64 * 1024);
        setup_int13_hd_read(&mut m.cpu, 0, 0, 1, 1, 0x0000, 0x7C00);
        m.service_int13_hd();
        assert!(cf(&m.cpu));
        assert_eq!(m.cpu.ah(), INT13_STATUS_TIMEOUT);
    }

    /// Spec: non-HD drive number rejected (`DL != 80h`).
    #[test]
    fn int13_rejects_floppy_drive_number() {
        let mut m = Machine::with_ide(64 * 1024, synthetic_disk(2));
        setup_int13_hd_read(&mut m.cpu, 0, 0, 1, 1, 0x0000, 0x7C00);
        m.cpu.set_gpr_u8_low(CpuState::RDX, 0x00);
        m.service_int13_hd();
        assert!(cf(&m.cpu));
        assert_eq!(m.cpu.ah(), INT13_STATUS_INVALID);
    }

    /// Spec: AH=00h reset succeeds when IDE present.
    #[test]
    fn int13_ah00_reset_ok() {
        let mut m = Machine::with_ide(64 * 1024, synthetic_disk(1));
        m.cpu.set_ah(INT13_AH_RESET);
        m.cpu.set_gpr_u8_low(CpuState::RDX, INT13_DRIVE_HD0);
        m.service_int13_hd();
        assert!(!cf(&m.cpu));
        assert_eq!(m.cpu.ah(), INT13_STATUS_OK);
    }

    /// Spec: AH=08h returns fixed 16-head / 63-spt geometry.
    #[test]
    fn int13_ah08_returns_geometry() {
        let mut m = Machine::with_ide(64 * 1024, synthetic_disk(16 * 63 * 2));
        m.cpu.set_ah(INT13_AH_GET_DRIVE_PARAMS);
        m.cpu.set_gpr_u8_low(CpuState::RDX, INT13_DRIVE_HD0);
        m.service_int13_hd();
        assert!(!cf(&m.cpu));
        let (cyl, sec) = unpack_cx(m.cpu.gpr_u16(CpuState::RCX));
        assert_eq!(sec, INT13_HD_SPT as u8);
        assert!(cyl >= 1);
        assert_eq!(m.cpu.gpr_u8(4 + CpuState::RDX), (INT13_HD_HEADS - 1) as u8);
        assert_eq!(m.cpu.gpr_u8_low(CpuState::RDX), 1);
    }

    /// Spec: unsupported AH → invalid function.
    #[test]
    fn int13_unsupported_ah_fails() {
        let mut m = Machine::with_ide(64 * 1024, synthetic_disk(1));
        m.cpu.set_ah(0x42); // IBM/MS extensions — out of scope
        m.cpu.set_gpr_u8_low(CpuState::RDX, INT13_DRIVE_HD0);
        m.service_int13_hd();
        assert!(cf(&m.cpu));
        assert_eq!(m.cpu.ah(), INT13_STATUS_INVALID);
    }

    #[test]
    fn int13_ivt_pointer_install() {
        let mut m = Machine::new(64 * 1024);
        m.install_int13_ivt_pointer(0xF000, 0xE000).unwrap();
        assert_eq!(m.mem.read_u8(0x4C).unwrap(), 0x00);
        assert_eq!(m.mem.read_u8(0x4D).unwrap(), 0xE0);
        assert_eq!(m.mem.read_u8(0x4E).unwrap(), 0x00);
        assert_eq!(m.mem.read_u8(0x4F).unwrap(), 0xF0);
    }

    #[test]
    fn pack_unpack_cx_round_trip() {
        for cyl in [0u16, 1, 255, 256, 1023] {
            for sec in [1u8, 18, 63] {
                let cx = pack_cx(cyl, sec);
                let (c2, s2) = unpack_cx(cx);
                assert_eq!((c2, s2), (cyl, sec), "cyl={cyl} sec={sec}");
            }
        }
    }

    /// Host read helper feeds the same bytes `load_mbr_to_7c00` would use.
    #[test]
    fn int13_read_matches_mbr_lba0() {
        let img = synthetic_disk(2);
        let mut m = Machine::with_ide(64 * 1024, img.clone());
        m.int13_hd_read_chs_to_phys(0, 0, 1, 1, MBR_PHYS_ADDR)
            .expect("read");
        let mut m2 = Machine::with_ide(64 * 1024, img);
        m2.load_mbr_to_7c00().unwrap();
        for i in 0..MBR_SECTOR_SIZE {
            assert_eq!(
                m.mem.read_u8(MBR_PHYS_ADDR + i as u64).unwrap(),
                m2.mem.read_u8(MBR_PHYS_ADDR + i as u64).unwrap()
            );
        }
    }
}
