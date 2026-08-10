//! Host-side IBM BIOS INT 13h hard-disk subset (AH=00/02/03/08 + 41h/42h/43h/48h),
//! floppy subset (AH=00/02/03/08/15, `DL=00h`), and CD/El Torito subset
//! (AH=41h/42h/48h/4Bh, `DL=E0h`).
//!
//! Closest approach in-tree to SeaBIOS disk services: a **host** dispatcher that
//! applies classic INT 13h register conventions against the primary IDE image,
//! attached FDC media, or ATAPI CD medium, mirroring
//! [`crate::mbr::Machine::load_mbr_to_7c00`]'s host-side media path. This is
//! **not** a guest IVT BIOS and not CHS translation modes.
//!
//! Spec: IBM PC BIOS INT 13h Disk Services (AH=00h reset, AH=02h read sectors,
//! AH=03h write sectors, AH=08h get drive parameters, AH=15h get disk type);
//! IBM/Microsoft INT 13h Extensions / RBIL (AH=41h check extensions, AH=42h
//! extended read, AH=43h extended write, AH=48h extended drive parameters);
//! El Torito 1.0 / RBIL AH=4Bh AL=00h bootable CD-ROM status packet.
//! ATA IDENTIFY obsolete geometry 16 heads / 63 sectors-per-track (matches
//! `IdePrimary` IDENTIFY words 3/6). Floppy uses fixed 1.44MB geometry (80/2/18)
//! via `Fdc82077::read_sector` / `Fdc82077::write_sector`. CD uses Mode-1
//! 2048-byte LBAs from the attached ATAPI image.

use crate::{Machine, MachineError};
use devices::{FDC_1440_CYLINDERS, FDC_1440_HEADS, FDC_1440_SECTORS_PER_TRACK, FDC_SECTOR_SIZE};
use firmware_interface::EL_TORITO_SECTOR_BYTES;
use x86_core::CpuState;

/// First floppy (`DL`).
pub const INT13_DRIVE_FD0: u8 = 0x00;
/// First hard disk (IBM BIOS `DL`).
pub const INT13_DRIVE_HD0: u8 = 0x80;
/// First CD-ROM / El Torito no-emulation drive number commonly assigned by BIOS.
pub const INT13_DRIVE_CD0: u8 = 0xE0;

/// AH=00h — reset disk system.
pub const INT13_AH_RESET: u8 = 0x00;
/// AH=02h — read disk sectors into `ES:BX`.
pub const INT13_AH_READ: u8 = 0x02;
/// AH=03h — write disk sectors from `ES:BX`.
pub const INT13_AH_WRITE: u8 = 0x03;
/// AH=08h — get drive parameters.
pub const INT13_AH_GET_DRIVE_PARAMS: u8 = 0x08;
/// AH=15h — get disk type / media sense (AT and later).
pub const INT13_AH_GET_DISK_TYPE: u8 = 0x15;
/// AH=41h — check extensions present.
pub const INT13_AH_CHECK_EXTENSIONS: u8 = 0x41;
/// AH=42h — extended read sectors (Disk Address Packet).
pub const INT13_AH_EXT_READ: u8 = 0x42;
/// AH=43h — extended write sectors (Disk Address Packet).
pub const INT13_AH_EXT_WRITE: u8 = 0x43;
/// AH=48h — extended get drive parameters.
pub const INT13_AH_EXT_GET_PARAMS: u8 = 0x48;
/// AH=4Bh — bootable CD-ROM (El Torito) get status / terminate emulation.
pub const INT13_AH_CDROM_EMULATION: u8 = 0x4B;
/// AH=4Bh AL=00h — get status (fill specification packet at `DS:SI`).
pub const INT13_CD_AL_GET_STATUS: u8 = 0x00;
/// El Torito / RBIL specification packet size (`13h` = 19 bytes).
pub const INT13_CD_SPEC_PACKET_SIZE: u8 = 0x13;

/// Magic `BX` input for AH=41h.
pub const INT13_EXT_MAGIC_IN: u16 = 0x55AA;
/// Magic `BX` output for AH=41h success.
pub const INT13_EXT_MAGIC_OUT: u16 = 0xAA55;
/// Major version returned in `AH` on AH=41h success (IBM/MS INT 13h Extensions).
pub const INT13_EXT_VERSION: u8 = 0x01;
/// `CX` bit 0 — packet-structure device access supported (AH=42h/43h here).
pub const INT13_EXT_CX_PACKET: u16 = 0x0001;
/// `CX` bit 2 — Enhanced Disk Drive support (AH=48h subset here).
pub const INT13_EXT_CX_EDD: u16 = 0x0004;
/// Subset advertised by AH=41h: packet access + EDD params (not locking).
pub const INT13_EXT_CX_SUPPORTED: u16 = INT13_EXT_CX_PACKET | INT13_EXT_CX_EDD;
/// Minimum Disk Address Packet size (16 bytes).
pub const INT13_DAP_SIZE_MIN: u8 = 0x10;
/// Minimum AH=48h result buffer size (Phoenix EDD v1.x / RBIL).
pub const INT13_EDD_PARAMS_SIZE_MIN: u16 = 0x1A;
/// Information flags: geometry fields are valid.
pub const INT13_EDD_INFO_GEOMETRY_VALID: u16 = 0x0002;

/// Success status in `AH` with `CF` clear.
pub const INT13_STATUS_OK: u8 = 0x00;
/// Invalid command / unsupported function / bad drive.
pub const INT13_STATUS_INVALID: u8 = 0x01;
/// Write protected (floppy media WP pin).
pub const INT13_STATUS_WRITE_PROTECTED: u8 = 0x03;
/// Sector not found / address beyond media.
pub const INT13_STATUS_SECTOR_NOT_FOUND: u8 = 0x04;
/// Drive not ready (no attached IDE / floppy image).
pub const INT13_STATUS_TIMEOUT: u8 = 0x80;

/// AH=15h type: no such drive (CF clear).
pub const INT13_DISK_TYPE_NONE: u8 = 0x00;
/// AH=15h type: floppy without change-line support.
pub const INT13_DISK_TYPE_FLOPPY: u8 = 0x01;
/// AH=15h type: floppy with change-line support (82077AA DIR DSKCHG).
pub const INT13_DISK_TYPE_FLOPPY_CHANGE_LINE: u8 = 0x02;
/// AH=15h type: hard disk.
pub const INT13_DISK_TYPE_HARD: u8 = 0x03;

/// AH=08h floppy `BL` — 1.44 MB 3½″ (CMOS/RBIL type `04h`).
pub const INT13_FLOPPY_TYPE_1440: u8 = 0x04;
/// Max cylinder for 1.44MB geometry (`FDC_1440_CYLINDERS - 1`).
pub const INT13_FLOPPY_MAX_CYLINDER: u8 = FDC_1440_CYLINDERS - 1;
/// Max head for 1.44MB geometry (`FDC_1440_HEADS - 1`).
pub const INT13_FLOPPY_MAX_HEAD: u8 = FDC_1440_HEADS - 1;
/// Sectors per track for 1.44MB geometry.
pub const INT13_FLOPPY_SPT: u8 = FDC_1440_SECTORS_PER_TRACK;

/// Heads matching IDE IDENTIFY obsolete word 3.
pub const INT13_HD_HEADS: u16 = 16;
/// Sectors per track matching IDE IDENTIFY obsolete word 6.
pub const INT13_HD_SPT: u16 = 63;

/// ATA / BIOS sector size.
pub const INT13_SECTOR_SIZE: usize = 512;
/// CD Mode-1 user-data / El Torito logical block size (AH=42h/48h on `DL=E0h`).
pub const INT13_CD_SECTOR_SIZE: usize = EL_TORITO_SECTOR_BYTES;

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
            INT13_AH_WRITE => self.int13_hd_write_from_regs(),
            INT13_AH_GET_DRIVE_PARAMS => self.int13_hd_get_params(),
            INT13_AH_CHECK_EXTENSIONS => self.int13_hd_check_extensions(),
            INT13_AH_EXT_READ => self.int13_hd_ext_read_from_regs(),
            INT13_AH_EXT_WRITE => self.int13_hd_ext_write_from_regs(),
            INT13_AH_EXT_GET_PARAMS => self.int13_hd_ext_get_params(),
            _ => self.int13_fail(INT13_STATUS_INVALID),
        }
    }

    /// Read `count` 512-byte sectors from primary IDE starting at absolute LBA
    /// into physical `dest`.
    ///
    /// Spec: IBM/MS INT 13h Extensions AH=42h — LBA addressing; host helper
    /// without touching CPU registers.
    pub fn int13_hd_read_lba_to_phys(
        &mut self,
        lba: u64,
        count: u16,
        dest: u64,
    ) -> Result<u16, u8> {
        if !self.ide.present || self.ide.image.is_empty() {
            return Err(INT13_STATUS_TIMEOUT);
        }
        if count == 0 {
            return Err(INT13_STATUS_INVALID);
        }
        let total = (self.ide.image.len() / INT13_SECTOR_SIZE) as u64;
        let need = u64::from(count);
        if lba.checked_add(need).is_none_or(|end| end > total) {
            return Err(INT13_STATUS_SECTOR_NOT_FOUND);
        }
        let byte_off = (lba as usize).saturating_mul(INT13_SECTOR_SIZE);
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

    /// Write `count` 512-byte sectors from physical `src` into primary IDE
    /// starting at absolute LBA.
    ///
    /// Spec: IBM/MS INT 13h Extensions / RBIL AH=43h — LBA addressing; host
    /// helper without touching CPU registers. Verify-after-write (`AL` bit 0)
    /// is ignored (write-only; no post-write readback).
    pub fn int13_hd_write_lba_from_phys(
        &mut self,
        lba: u64,
        count: u16,
        src: u64,
    ) -> Result<u16, u8> {
        if !self.ide.present || self.ide.image.is_empty() {
            return Err(INT13_STATUS_TIMEOUT);
        }
        if count == 0 {
            return Err(INT13_STATUS_INVALID);
        }
        let total = (self.ide.image.len() / INT13_SECTOR_SIZE) as u64;
        let need = u64::from(count);
        if lba.checked_add(need).is_none_or(|end| end > total) {
            return Err(INT13_STATUS_SECTOR_NOT_FOUND);
        }
        let byte_off = (lba as usize).saturating_mul(INT13_SECTOR_SIZE);
        let bytes = usize::from(count).saturating_mul(INT13_SECTOR_SIZE);
        let end = src.checked_add(bytes as u64).ok_or(INT13_STATUS_INVALID)?;
        if end > self.mem.ram_len() as u64 {
            return Err(INT13_STATUS_INVALID);
        }
        for i in 0..bytes {
            let b = self
                .mem
                .read_u8(src + i as u64)
                .map_err(|_| INT13_STATUS_INVALID)?;
            self.ide.image[byte_off + i] = b;
        }
        Ok(count)
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
        let (byte_off, bytes, dest) =
            self.int13_hd_chs_bounds(cylinder, head, sector, count, dest)?;
        for i in 0..bytes {
            let b = self.ide.image[byte_off + i];
            self.mem
                .write_u8(dest + i as u64, b)
                .map_err(|_| INT13_STATUS_INVALID)?;
        }
        Ok(count)
    }

    /// Write `count` sectors from physical `src` into primary IDE at packed
    /// INT 13h CHS, without touching CPU registers.
    ///
    /// Spec: IBM PC BIOS INT 13h AH=03h — same CHS packing as AH=02h; buffer
    /// is the source.
    pub fn int13_hd_write_chs_from_phys(
        &mut self,
        cylinder: u16,
        head: u8,
        sector: u8,
        count: u8,
        src: u64,
    ) -> Result<u8, u8> {
        let (byte_off, bytes, src) =
            self.int13_hd_chs_bounds(cylinder, head, sector, count, src)?;
        for i in 0..bytes {
            let b = self
                .mem
                .read_u8(src + i as u64)
                .map_err(|_| INT13_STATUS_INVALID)?;
            self.ide.image[byte_off + i] = b;
        }
        Ok(count)
    }

    fn int13_hd_chs_bounds(
        &self,
        cylinder: u16,
        head: u8,
        sector: u8,
        count: u8,
        phys: u64,
    ) -> Result<(usize, usize, u64), u8> {
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
        let end = phys.checked_add(bytes as u64).ok_or(INT13_STATUS_INVALID)?;
        if end > self.mem.ram_len() as u64 {
            return Err(INT13_STATUS_INVALID);
        }
        Ok((byte_off, bytes, phys))
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

    fn int13_hd_write_from_regs(&mut self) {
        let al = self.cpu.al();
        let cx = self.cpu.gpr_u16(CpuState::RCX);
        let dh = self.cpu.gpr_u8(4 + CpuState::RDX);
        let bx = self.cpu.gpr_u16(CpuState::RBX);
        let (cylinder, sector) = unpack_cx(cx);
        let src = self.cpu.es.base.wrapping_add(u64::from(bx));

        match self.int13_hd_write_chs_from_phys(cylinder, dh, sector, al, src) {
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
        // AH=08h: CX = max cylinder/sector packed; DH = max head; DL = drive count.
        // AL unused (cleared); BL = 00h for hard disks (floppy type N/A).
        let (max_cyl, heads, spt, _total) = self.int13_hd_geometry();
        self.cpu
            .set_gpr_u16(CpuState::RCX, pack_cx(max_cyl, spt as u8));
        self.cpu
            .set_gpr_u8(4 + CpuState::RDX, heads.saturating_sub(1) as u8);
        self.cpu.set_gpr_u8_low(CpuState::RDX, 1); // one HD
        self.cpu.set_al(0);
        self.cpu.set_gpr_u8_low(CpuState::RBX, 0); // BL
        self.cpu.set_ah(INT13_STATUS_OK);
        self.cpu.set_cf(false);
    }

    /// Spec: IBM/MS INT 13h Extensions AH=41h — `BX=55AAh` in, `BX=AA55h` out.
    fn int13_hd_check_extensions(&mut self) {
        if self.cpu.gpr_u16(CpuState::RBX) != INT13_EXT_MAGIC_IN {
            self.int13_fail(INT13_STATUS_INVALID);
            return;
        }
        if !self.ide.present {
            self.int13_fail(INT13_STATUS_TIMEOUT);
            return;
        }
        self.cpu.set_ah(INT13_EXT_VERSION);
        self.cpu.set_gpr_u16(CpuState::RBX, INT13_EXT_MAGIC_OUT);
        // Packet access (AH=42h/43h) + EDD params (AH=48h). Removable locking out.
        self.cpu.set_gpr_u16(CpuState::RCX, INT13_EXT_CX_SUPPORTED);
        self.cpu.set_cf(false);
    }

    /// Spec: IBM/MS INT 13h Extensions AH=42h — Disk Address Packet at `DS:SI`.
    fn int13_hd_ext_read_from_regs(&mut self) {
        let si = self.cpu.gpr_u16(CpuState::RSI);
        let dap_phys = self.cpu.ds.base.wrapping_add(u64::from(si));
        match self.int13_parse_dap(dap_phys) {
            Ok(dap) => match self.int13_hd_read_lba_to_phys(dap.lba, dap.count, dap.buf) {
                Ok(_) => {
                    self.cpu.set_ah(INT13_STATUS_OK);
                    self.cpu.set_cf(false);
                }
                Err(status) => self.int13_fail(status),
            },
            Err(status) => self.int13_fail(status),
        }
    }

    /// Spec: IBM/MS INT 13h Extensions / RBIL AH=43h — Disk Address Packet at
    /// `DS:SI`. `AL` verify flag is accepted but ignored (no post-write verify).
    fn int13_hd_ext_write_from_regs(&mut self) {
        let si = self.cpu.gpr_u16(CpuState::RSI);
        let dap_phys = self.cpu.ds.base.wrapping_add(u64::from(si));
        match self.int13_parse_dap(dap_phys) {
            Ok(dap) => match self.int13_hd_write_lba_from_phys(dap.lba, dap.count, dap.buf) {
                Ok(_) => {
                    self.cpu.set_ah(INT13_STATUS_OK);
                    self.cpu.set_cf(false);
                }
                Err(status) => self.int13_fail(status),
            },
            Err(status) => self.int13_fail(status),
        }
    }

    /// Spec: Phoenix EDD / IBM INT 13h Extensions AH=48h — result buffer at `DS:SI`.
    fn int13_hd_ext_get_params(&mut self) {
        if !self.ide.present || self.ide.image.is_empty() {
            self.int13_fail(INT13_STATUS_TIMEOUT);
            return;
        }
        let si = self.cpu.gpr_u16(CpuState::RSI);
        let buf = self.cpu.ds.base.wrapping_add(u64::from(si));
        let Ok(buf_size) = self.read_guest_u16(buf) else {
            self.int13_fail(INT13_STATUS_INVALID);
            return;
        };
        if buf_size < INT13_EDD_PARAMS_SIZE_MIN {
            self.int13_fail(INT13_STATUS_INVALID);
            return;
        }
        let (max_cyl, heads, spt, total) = self.int13_hd_geometry();
        // Phoenix: DWORD cylinder/head/spt counts; cylinder count = max+1 when media exists.
        let cyl_count = if total == 0 {
            0u32
        } else {
            u32::from(max_cyl).saturating_add(1)
        };
        if self
            .write_guest_u16(buf, INT13_EDD_PARAMS_SIZE_MIN)
            .and_then(|_| self.write_guest_u16(buf + 2, INT13_EDD_INFO_GEOMETRY_VALID))
            .and_then(|_| self.write_guest_u32(buf + 4, cyl_count))
            .and_then(|_| self.write_guest_u32(buf + 8, u32::from(heads)))
            .and_then(|_| self.write_guest_u32(buf + 12, u32::from(spt)))
            .and_then(|_| self.write_guest_u64(buf + 16, total))
            .and_then(|_| self.write_guest_u16(buf + 24, INT13_SECTOR_SIZE as u16))
            .is_err()
        {
            self.int13_fail(INT13_STATUS_INVALID);
            return;
        }
        self.cpu.set_ah(INT13_STATUS_OK);
        self.cpu.set_cf(false);
    }

    /// Fixed 16/63 geometry derived from the attached IDE image size.
    fn int13_hd_geometry(&self) -> (u16, u16, u16, u64) {
        let total = (self.ide.image.len() / INT13_SECTOR_SIZE) as u64;
        let max_cyl = if total == 0 {
            0u16
        } else {
            let spc = u64::from(INT13_HD_HEADS) * u64::from(INT13_HD_SPT);
            ((total.saturating_sub(1)) / spc).min(u64::from(u16::MAX)) as u16
        };
        (max_cyl, INT13_HD_HEADS, INT13_HD_SPT, total)
    }

    fn int13_parse_dap(&self, dap_phys: u64) -> Result<DiskAddressPacket, u8> {
        let size = self
            .mem
            .read_u8(dap_phys)
            .map_err(|_| INT13_STATUS_INVALID)?;
        if size < INT13_DAP_SIZE_MIN {
            return Err(INT13_STATUS_INVALID);
        }
        let count = self.read_guest_u16(dap_phys + 2)?;
        let buf_off = self.read_guest_u16(dap_phys + 4)?;
        let buf_seg = self.read_guest_u16(dap_phys + 6)?;
        let lba = self.read_guest_u64(dap_phys + 8)?;
        // Classic packet: transfer buffer is real-mode `seg:off`. Flat 64-bit
        // buffer form (`size >= 18h` with `FFFF:FFFF` pointer) is unsupported.
        if buf_seg == 0xFFFF && buf_off == 0xFFFF {
            return Err(INT13_STATUS_INVALID);
        }
        let buf = (u64::from(buf_seg) << 4).wrapping_add(u64::from(buf_off));
        Ok(DiskAddressPacket { count, buf, lba })
    }

    fn read_guest_u16(&self, phys: u64) -> Result<u16, u8> {
        let lo = self.mem.read_u8(phys).map_err(|_| INT13_STATUS_INVALID)?;
        let hi = self
            .mem
            .read_u8(phys + 1)
            .map_err(|_| INT13_STATUS_INVALID)?;
        Ok(u16::from(lo) | (u16::from(hi) << 8))
    }

    fn read_guest_u64(&self, phys: u64) -> Result<u64, u8> {
        let mut v = 0u64;
        for i in 0..8u64 {
            let b = self
                .mem
                .read_u8(phys + i)
                .map_err(|_| INT13_STATUS_INVALID)?;
            v |= u64::from(b) << (i * 8);
        }
        Ok(v)
    }

    fn write_guest_u16(&mut self, phys: u64, value: u16) -> Result<(), u8> {
        self.mem
            .write_u8(phys, (value & 0xFF) as u8)
            .map_err(|_| INT13_STATUS_INVALID)?;
        self.mem
            .write_u8(phys + 1, (value >> 8) as u8)
            .map_err(|_| INT13_STATUS_INVALID)?;
        Ok(())
    }

    fn write_guest_u32(&mut self, phys: u64, value: u32) -> Result<(), u8> {
        for (i, b) in value.to_le_bytes().iter().enumerate() {
            self.mem
                .write_u8(phys + i as u64, *b)
                .map_err(|_| INT13_STATUS_INVALID)?;
        }
        Ok(())
    }

    fn write_guest_u64(&mut self, phys: u64, value: u64) -> Result<(), u8> {
        for (i, b) in value.to_le_bytes().iter().enumerate() {
            self.mem
                .write_u8(phys + i as u64, *b)
                .map_err(|_| INT13_STATUS_INVALID)?;
        }
        Ok(())
    }
}

/// Parsed classic 16-byte Disk Address Packet (IBM/MS INT 13h Extensions).
struct DiskAddressPacket {
    count: u16,
    /// Physical buffer address (`seg:off`) — destination for read, source for write.
    buf: u64,
    lba: u64,
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

/// Convenience: set up INT 13h AH=03h registers for a hard-disk write.
pub fn setup_int13_hd_write(
    cpu: &mut CpuState,
    cylinder: u16,
    head: u8,
    sector: u8,
    count: u8,
    es: u16,
    bx: u16,
) {
    cpu.set_ah(INT13_AH_WRITE);
    cpu.set_al(count);
    cpu.set_gpr_u16(CpuState::RCX, pack_cx(cylinder, sector));
    cpu.set_gpr_u8_low(CpuState::RDX, INT13_DRIVE_HD0);
    cpu.set_gpr_u8(4 + CpuState::RDX, head);
    cpu.set_gpr_u16(CpuState::RBX, bx);
    cpu.es = x86_core::SegmentReg::real_mode(es);
}

/// Write a classic 16-byte Disk Address Packet at `dap_phys` (DS=0 harness).
fn write_dap(
    machine: &mut Machine,
    dap_phys: u64,
    lba: u64,
    count: u16,
    buf_seg: u16,
    buf_off: u16,
) {
    let mut dap = [0u8; 16];
    dap[0] = INT13_DAP_SIZE_MIN;
    dap[2] = (count & 0xFF) as u8;
    dap[3] = (count >> 8) as u8;
    dap[4] = (buf_off & 0xFF) as u8;
    dap[5] = (buf_off >> 8) as u8;
    dap[6] = (buf_seg & 0xFF) as u8;
    dap[7] = (buf_seg >> 8) as u8;
    for (i, b) in lba.to_le_bytes().iter().enumerate() {
        dap[8 + i] = *b;
    }
    for (i, b) in dap.iter().enumerate() {
        machine.mem.write_u8(dap_phys + i as u64, *b).unwrap();
    }
}

/// Write a classic 16-byte Disk Address Packet and set AH=42h / DS:SI / DL.
pub fn setup_int13_hd_ext_read(
    machine: &mut Machine,
    dap_phys: u64,
    lba: u64,
    count: u16,
    buf_seg: u16,
    buf_off: u16,
) {
    write_dap(machine, dap_phys, lba, count, buf_seg, buf_off);
    // Place DAP at physical address under DS:SI with DS=0 for harness simplicity.
    machine.cpu.set_ah(INT13_AH_EXT_READ);
    machine.cpu.set_gpr_u8_low(CpuState::RDX, INT13_DRIVE_HD0);
    machine.cpu.set_gpr_u16(CpuState::RSI, dap_phys as u16);
    machine.cpu.ds = x86_core::SegmentReg::real_mode(0);
}

/// Write a classic 16-byte Disk Address Packet and set AH=43h / DS:SI / DL.
///
/// Spec: RBIL INT 13h AH=43h — `AL` holds write flags; harness clears them
/// (no verify-after-write).
pub fn setup_int13_hd_ext_write(
    machine: &mut Machine,
    dap_phys: u64,
    lba: u64,
    count: u16,
    buf_seg: u16,
    buf_off: u16,
) {
    write_dap(machine, dap_phys, lba, count, buf_seg, buf_off);
    machine.cpu.set_ah(INT13_AH_EXT_WRITE);
    machine.cpu.set_al(0); // flags: no verify
    machine.cpu.set_gpr_u8_low(CpuState::RDX, INT13_DRIVE_HD0);
    machine.cpu.set_gpr_u16(CpuState::RSI, dap_phys as u16);
    machine.cpu.ds = x86_core::SegmentReg::real_mode(0);
}

/// Set up AH=48h with a result buffer of `buf_size` bytes at physical `buf_phys`.
pub fn setup_int13_hd_ext_get_params(machine: &mut Machine, buf_phys: u64, buf_size: u16) {
    machine
        .mem
        .write_u8(buf_phys, (buf_size & 0xFF) as u8)
        .unwrap();
    machine
        .mem
        .write_u8(buf_phys + 1, (buf_size >> 8) as u8)
        .unwrap();
    for i in 2..usize::from(buf_size.max(INT13_EDD_PARAMS_SIZE_MIN)) {
        machine.mem.write_u8(buf_phys + i as u64, 0).unwrap();
    }
    machine.cpu.set_ah(INT13_AH_EXT_GET_PARAMS);
    machine.cpu.set_gpr_u8_low(CpuState::RDX, INT13_DRIVE_HD0);
    machine.cpu.set_gpr_u16(CpuState::RSI, buf_phys as u16);
    machine.cpu.ds = x86_core::SegmentReg::real_mode(0);
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

    /// Host-side INT 13h floppy dispatch (`DL = 00h`) using current CPU registers.
    ///
    /// Supports AH=00h reset, AH=02h read, AH=03h write, AH=08h get-params, and
    /// AH=15h get disk type against attached FDC 1.44MB media. Spec: IBM PC BIOS
    /// INT 13h / RBIL floppy disk services; geometry matches
    /// [`FDC_1440_CYLINDERS`] / [`FDC_1440_HEADS`] /
    /// [`FDC_1440_SECTORS_PER_TRACK`]. Diskette parameter table (`ES:DI` on
    /// AH=08h) is out of scope.
    pub fn service_int13_floppy(&mut self) {
        let dl = self.cpu.gpr_u8_low(CpuState::RDX);
        if dl != INT13_DRIVE_FD0 {
            self.int13_fail(INT13_STATUS_INVALID);
            return;
        }
        match self.cpu.ah() {
            INT13_AH_RESET => self.int13_floppy_reset(),
            INT13_AH_READ => self.int13_floppy_read_from_regs(),
            INT13_AH_WRITE => self.int13_floppy_write_from_regs(),
            INT13_AH_GET_DRIVE_PARAMS => self.int13_floppy_get_params(),
            INT13_AH_GET_DISK_TYPE => self.int13_floppy_get_disk_type(),
            _ => self.int13_fail(INT13_STATUS_INVALID),
        }
    }

    /// Route INT 13h by `DL`: floppy `00h`, hard disk `80h`, or CD `E0h`.
    pub fn service_int13(&mut self) {
        match self.cpu.gpr_u8_low(CpuState::RDX) {
            INT13_DRIVE_FD0 => self.service_int13_floppy(),
            INT13_DRIVE_HD0 => self.service_int13_hd(),
            INT13_DRIVE_CD0 => self.service_int13_cd(),
            _ => self.int13_fail(INT13_STATUS_INVALID),
        }
    }

    /// Host-side INT 13h CD/El Torito dispatch (`DL = E0h`).
    ///
    /// Supports AH=41h/42h/48h against the ATAPI Mode-1 medium (2048-byte LBAs)
    /// and AH=4Bh AL=00h get-status (specification packet at `DS:SI`). Spec:
    /// IBM/MS INT 13h Extensions + El Torito 1.0 / RBIL AH=4Bh. Not SeaBIOS;
    /// terminate-emulation (`AL=01h`) and floppy/HDD emulation media remain out.
    pub fn service_int13_cd(&mut self) {
        let dl = self.cpu.gpr_u8_low(CpuState::RDX);
        if dl != INT13_DRIVE_CD0 {
            self.int13_fail(INT13_STATUS_INVALID);
            return;
        }
        match self.cpu.ah() {
            INT13_AH_CHECK_EXTENSIONS => self.int13_cd_check_extensions(),
            INT13_AH_EXT_READ => self.int13_cd_ext_read_from_regs(),
            INT13_AH_EXT_GET_PARAMS => self.int13_cd_ext_get_params(),
            INT13_AH_CDROM_EMULATION => self.int13_cd_emulation_from_regs(),
            _ => self.int13_fail(INT13_STATUS_INVALID),
        }
    }

    /// Read `count` Mode-1 (2048-byte) CD sectors from the ATAPI medium at LBA.
    ///
    /// Spec: IBM/MS INT 13h Extensions AH=42h applied to El Torito/ATAPI CD —
    /// packet `count` is in 2048-byte blocks (not 512). Host helper only.
    pub fn int13_cd_read_lba_to_phys(
        &mut self,
        lba: u64,
        count: u16,
        dest: u64,
    ) -> Result<u16, u8> {
        if !self.ide.is_atapi_cdrom() {
            return Err(INT13_STATUS_TIMEOUT);
        }
        let Some(image) = self.ide.atapi_medium_image() else {
            return Err(INT13_STATUS_TIMEOUT);
        };
        if count == 0 {
            return Err(INT13_STATUS_INVALID);
        }
        let total = (image.len() / INT13_CD_SECTOR_SIZE) as u64;
        let need = u64::from(count);
        if lba.checked_add(need).is_none_or(|end| end > total) {
            return Err(INT13_STATUS_SECTOR_NOT_FOUND);
        }
        let byte_off = (lba as usize).saturating_mul(INT13_CD_SECTOR_SIZE);
        let bytes = usize::from(count).saturating_mul(INT13_CD_SECTOR_SIZE);
        let end = dest.checked_add(bytes as u64).ok_or(INT13_STATUS_INVALID)?;
        if end > self.mem.ram_len() as u64 {
            return Err(INT13_STATUS_INVALID);
        }
        let chunk = image[byte_off..byte_off + bytes].to_vec();
        for (i, b) in chunk.iter().enumerate() {
            self.mem
                .write_u8(dest + i as u64, *b)
                .map_err(|_| INT13_STATUS_INVALID)?;
        }
        Ok(count)
    }

    fn int13_cd_has_medium(&self) -> bool {
        self.ide.is_atapi_cdrom() && self.ide.atapi_medium_image().is_some()
    }

    fn int13_cd_check_extensions(&mut self) {
        if self.cpu.gpr_u16(CpuState::RBX) != INT13_EXT_MAGIC_IN {
            self.int13_fail(INT13_STATUS_INVALID);
            return;
        }
        if !self.int13_cd_has_medium() {
            self.int13_fail(INT13_STATUS_TIMEOUT);
            return;
        }
        self.cpu.set_ah(INT13_EXT_VERSION);
        self.cpu.set_gpr_u16(CpuState::RBX, INT13_EXT_MAGIC_OUT);
        // Packet access (AH=42h) + EDD params (AH=48h). Removable locking out.
        self.cpu.set_gpr_u16(CpuState::RCX, INT13_EXT_CX_SUPPORTED);
        self.cpu.set_cf(false);
    }

    fn int13_cd_ext_read_from_regs(&mut self) {
        let si = self.cpu.gpr_u16(CpuState::RSI);
        let dap_phys = self.cpu.ds.base.wrapping_add(u64::from(si));
        match self.int13_parse_dap(dap_phys) {
            Ok(dap) => match self.int13_cd_read_lba_to_phys(dap.lba, dap.count, dap.buf) {
                Ok(_) => {
                    self.cpu.set_ah(INT13_STATUS_OK);
                    self.cpu.set_cf(false);
                }
                Err(status) => self.int13_fail(status),
            },
            Err(status) => self.int13_fail(status),
        }
    }

    fn int13_cd_ext_get_params(&mut self) {
        if !self.int13_cd_has_medium() {
            self.int13_fail(INT13_STATUS_TIMEOUT);
            return;
        }
        let total = self
            .ide
            .atapi_medium_image()
            .map(|img| (img.len() / INT13_CD_SECTOR_SIZE) as u64)
            .unwrap_or(0);
        let si = self.cpu.gpr_u16(CpuState::RSI);
        let buf = self.cpu.ds.base.wrapping_add(u64::from(si));
        let Ok(buf_size) = self.read_guest_u16(buf) else {
            self.int13_fail(INT13_STATUS_INVALID);
            return;
        };
        if buf_size < INT13_EDD_PARAMS_SIZE_MIN {
            self.int13_fail(INT13_STATUS_INVALID);
            return;
        }
        // CD: report linear LBA geometry (cyl=total, heads=1, spt=1) + 2048-byte
        // sector size. Spec: Phoenix EDD / IBM INT 13h Extensions AH=48h.
        if self
            .write_guest_u16(buf, INT13_EDD_PARAMS_SIZE_MIN)
            .and_then(|_| self.write_guest_u16(buf + 2, INT13_EDD_INFO_GEOMETRY_VALID))
            .and_then(|_| self.write_guest_u32(buf + 4, total as u32))
            .and_then(|_| self.write_guest_u32(buf + 8, 1))
            .and_then(|_| self.write_guest_u32(buf + 12, 1))
            .and_then(|_| self.write_guest_u64(buf + 16, total))
            .and_then(|_| self.write_guest_u16(buf + 24, INT13_CD_SECTOR_SIZE as u16))
            .is_err()
        {
            self.int13_fail(INT13_STATUS_INVALID);
            return;
        }
        self.cpu.set_ah(INT13_STATUS_OK);
        self.cpu.set_cf(false);
    }

    /// Spec: El Torito / RBIL INT 13h AH=4Bh — AL=00h fills a 19-byte
    /// specification packet at `DS:SI` from the attached ATAPI El Torito catalog.
    fn int13_cd_emulation_from_regs(&mut self) {
        if self.cpu.al() != INT13_CD_AL_GET_STATUS {
            // AL=01h terminate-emulation and other subfunctions are out of scope.
            self.int13_fail(INT13_STATUS_INVALID);
            return;
        }
        if !self.int13_cd_has_medium() {
            self.int13_fail(INT13_STATUS_TIMEOUT);
            return;
        }
        let info = match self.inspect_atapi_el_torito() {
            Ok(info) if info.bootable => info,
            _ => {
                self.int13_fail(INT13_STATUS_INVALID);
                return;
            }
        };
        let si = self.cpu.gpr_u16(CpuState::RSI);
        let pkt = self.cpu.ds.base.wrapping_add(u64::from(si));
        let end = pkt.wrapping_add(u64::from(INT13_CD_SPEC_PACKET_SIZE));
        if end > self.mem.ram_len() as u64 {
            self.int13_fail(INT13_STATUS_INVALID);
            return;
        }
        // El Torito specification packet (19 bytes). CHS fields stay 0 for no-emul.
        let mut buf = [0u8; INT13_CD_SPEC_PACKET_SIZE as usize];
        buf[0] = INT13_CD_SPEC_PACKET_SIZE;
        buf[1] = info.media_type;
        buf[2] = INT13_DRIVE_CD0;
        buf[3] = 0; // controller index
        buf[4..8].copy_from_slice(&info.load_rba.to_le_bytes());
        buf[8] = 0; // device specification (IDE primary master stub)
        buf[9] = 0;
        buf[10] = 0; // buffer segment unused for get-status
        buf[11] = 0;
        let load_seg = info.effective_load_segment();
        buf[12..14].copy_from_slice(&load_seg.to_le_bytes());
        buf[14..16].copy_from_slice(&info.sector_count.to_le_bytes());
        // offsets 16..18: cylinder/sector/head remain 0 for media type 00h
        for (i, b) in buf.iter().enumerate() {
            if self.mem.write_u8(pkt + i as u64, *b).is_err() {
                self.int13_fail(INT13_STATUS_INVALID);
                return;
            }
        }
        self.cpu.set_ah(INT13_STATUS_OK);
        self.cpu.set_cf(false);
    }

    /// Read `count` floppy sectors starting at CHS into physical `dest`.
    ///
    /// Spec: IBM BIOS INT 13h AH=02h floppy — consecutive sectors advance
    /// sector → head → cylinder within 1.44MB geometry.
    pub fn int13_floppy_read_chs_to_phys(
        &mut self,
        cylinder: u8,
        head: u8,
        sector: u8,
        count: u8,
        dest: u64,
    ) -> Result<u8, u8> {
        self.int13_floppy_xfer(cylinder, head, sector, count, dest, false)
    }

    /// Write `count` floppy sectors from physical `src` starting at CHS.
    ///
    /// Spec: IBM BIOS INT 13h AH=03h floppy — same CHS advance as AH=02h.
    /// Media write-protect → [`INT13_STATUS_WRITE_PROTECTED`].
    pub fn int13_floppy_write_chs_from_phys(
        &mut self,
        cylinder: u8,
        head: u8,
        sector: u8,
        count: u8,
        src: u64,
    ) -> Result<u8, u8> {
        self.int13_floppy_xfer(cylinder, head, sector, count, src, true)
    }

    fn int13_floppy_xfer(
        &mut self,
        mut cylinder: u8,
        mut head: u8,
        mut sector: u8,
        count: u8,
        mut phys: u64,
        write: bool,
    ) -> Result<u8, u8> {
        if !self.fdc.has_media() {
            return Err(INT13_STATUS_TIMEOUT);
        }
        if write && self.fdc.write_protected {
            return Err(INT13_STATUS_WRITE_PROTECTED);
        }
        if count == 0 || sector == 0 || head >= FDC_1440_HEADS {
            return Err(INT13_STATUS_INVALID);
        }
        if sector > FDC_1440_SECTORS_PER_TRACK || cylinder >= FDC_1440_CYLINDERS {
            return Err(INT13_STATUS_SECTOR_NOT_FOUND);
        }
        let bytes = usize::from(count).saturating_mul(INT13_SECTOR_SIZE);
        let end = phys.checked_add(bytes as u64).ok_or(INT13_STATUS_INVALID)?;
        if end > self.mem.ram_len() as u64 {
            return Err(INT13_STATUS_INVALID);
        }

        for i in 0..count {
            if write {
                let mut sector_buf = [0u8; FDC_SECTOR_SIZE];
                for (j, b) in sector_buf.iter_mut().enumerate() {
                    *b = self
                        .mem
                        .read_u8(phys + j as u64)
                        .map_err(|_| INT13_STATUS_INVALID)?;
                }
                if !self.fdc.write_sector(cylinder, head, sector, &sector_buf) {
                    return Err(INT13_STATUS_SECTOR_NOT_FOUND);
                }
            } else {
                let Some(sector_buf) = self.fdc.read_sector(cylinder, head, sector) else {
                    return Err(INT13_STATUS_SECTOR_NOT_FOUND);
                };
                for (j, b) in sector_buf.iter().enumerate() {
                    self.mem
                        .write_u8(phys + j as u64, *b)
                        .map_err(|_| INT13_STATUS_INVALID)?;
                }
            }
            phys = phys.wrapping_add(INT13_SECTOR_SIZE as u64);
            if i + 1 < count {
                let Some((c, h, s)) = advance_floppy_chs(cylinder, head, sector) else {
                    return Err(INT13_STATUS_SECTOR_NOT_FOUND);
                };
                cylinder = c;
                head = h;
                sector = s;
            }
        }
        Ok(count)
    }

    fn int13_floppy_reset(&mut self) {
        if !self.fdc.has_media() {
            self.int13_fail(INT13_STATUS_TIMEOUT);
            return;
        }
        self.int13_ok_al(0);
    }

    fn int13_floppy_read_from_regs(&mut self) {
        let al = self.cpu.al();
        let cx = self.cpu.gpr_u16(CpuState::RCX);
        let dh = self.cpu.gpr_u8(4 + CpuState::RDX);
        let bx = self.cpu.gpr_u16(CpuState::RBX);
        let (cylinder, sector) = unpack_cx(cx);
        let dest = self.cpu.es.base.wrapping_add(u64::from(bx));
        if cylinder > u16::from(u8::MAX) {
            self.cpu.set_al(0);
            self.int13_fail(INT13_STATUS_SECTOR_NOT_FOUND);
            return;
        }
        match self.int13_floppy_read_chs_to_phys(cylinder as u8, dh, sector, al, dest) {
            Ok(n) => self.int13_ok_al(n),
            Err(status) => {
                self.cpu.set_al(0);
                self.int13_fail(status);
            }
        }
    }

    fn int13_floppy_write_from_regs(&mut self) {
        let al = self.cpu.al();
        let cx = self.cpu.gpr_u16(CpuState::RCX);
        let dh = self.cpu.gpr_u8(4 + CpuState::RDX);
        let bx = self.cpu.gpr_u16(CpuState::RBX);
        let (cylinder, sector) = unpack_cx(cx);
        let src = self.cpu.es.base.wrapping_add(u64::from(bx));
        if cylinder > u16::from(u8::MAX) {
            self.cpu.set_al(0);
            self.int13_fail(INT13_STATUS_SECTOR_NOT_FOUND);
            return;
        }
        match self.int13_floppy_write_chs_from_phys(cylinder as u8, dh, sector, al, src) {
            Ok(n) => self.int13_ok_al(n),
            Err(status) => {
                self.cpu.set_al(0);
                self.int13_fail(status);
            }
        }
    }

    /// Spec: IBM BIOS / RBIL INT 13h AH=08h floppy — return 1.44MB max CHS and
    /// drive type `BL=04h`. No media → [`INT13_STATUS_TIMEOUT`]. `ES:DI` diskette
    /// parameter table is not filled (out of scope).
    fn int13_floppy_get_params(&mut self) {
        if !self.fdc.has_media() {
            self.int13_fail(INT13_STATUS_TIMEOUT);
            return;
        }
        self.cpu.set_gpr_u16(
            CpuState::RCX,
            pack_cx(u16::from(INT13_FLOPPY_MAX_CYLINDER), INT13_FLOPPY_SPT),
        );
        self.cpu
            .set_gpr_u8(4 + CpuState::RDX, INT13_FLOPPY_MAX_HEAD);
        self.cpu.set_gpr_u8_low(CpuState::RDX, 1); // one floppy drive
        self.cpu
            .set_gpr_u8_low(CpuState::RBX, INT13_FLOPPY_TYPE_1440); // BL
        self.cpu.set_al(0);
        self.cpu.set_ah(INT13_STATUS_OK);
        self.cpu.set_cf(false);
    }

    /// Spec: IBM BIOS / RBIL INT 13h AH=15h — disk type in `AH` with CF clear.
    /// Attached 1.44MB media reports change-line support (`02h`); no media →
    /// type `00h` (no such drive).
    fn int13_floppy_get_disk_type(&mut self) {
        if !self.fdc.has_media() {
            self.cpu.set_ah(INT13_DISK_TYPE_NONE);
            self.cpu.set_cf(false);
            return;
        }
        // 82077AA DIR exposes DSKCHG — change-line capable floppy.
        self.cpu.set_ah(INT13_DISK_TYPE_FLOPPY_CHANGE_LINE);
        self.cpu.set_cf(false);
    }
}

/// Advance floppy CHS by one sector within 1.44MB geometry.
fn advance_floppy_chs(cylinder: u8, head: u8, sector: u8) -> Option<(u8, u8, u8)> {
    let mut sector = sector.checked_add(1)?;
    let mut head = head;
    let mut cylinder = cylinder;
    if sector > FDC_1440_SECTORS_PER_TRACK {
        sector = 1;
        head = head.checked_add(1)?;
        if head >= FDC_1440_HEADS {
            head = 0;
            cylinder = cylinder.checked_add(1)?;
            if cylinder >= FDC_1440_CYLINDERS {
                return None;
            }
        }
    }
    Some((cylinder, head, sector))
}

/// Convenience: set up INT 13h AH=02h registers for a floppy read.
pub fn setup_int13_floppy_read(
    cpu: &mut CpuState,
    cylinder: u8,
    head: u8,
    sector: u8,
    count: u8,
    es: u16,
    bx: u16,
) {
    cpu.set_ah(INT13_AH_READ);
    cpu.set_al(count);
    cpu.set_gpr_u16(CpuState::RCX, pack_cx(u16::from(cylinder), sector));
    cpu.set_gpr_u8_low(CpuState::RDX, INT13_DRIVE_FD0);
    cpu.set_gpr_u8(4 + CpuState::RDX, head);
    cpu.set_gpr_u16(CpuState::RBX, bx);
    cpu.es = x86_core::SegmentReg::real_mode(es);
}

/// Convenience: set up INT 13h AH=03h registers for a floppy write.
pub fn setup_int13_floppy_write(
    cpu: &mut CpuState,
    cylinder: u8,
    head: u8,
    sector: u8,
    count: u8,
    es: u16,
    bx: u16,
) {
    cpu.set_ah(INT13_AH_WRITE);
    cpu.set_al(count);
    cpu.set_gpr_u16(CpuState::RCX, pack_cx(u16::from(cylinder), sector));
    cpu.set_gpr_u8_low(CpuState::RDX, INT13_DRIVE_FD0);
    cpu.set_gpr_u8(4 + CpuState::RDX, head);
    cpu.set_gpr_u16(CpuState::RBX, bx);
    cpu.es = x86_core::SegmentReg::real_mode(es);
}

/// Convenience: set up INT 13h AH=08h registers for floppy get-params.
pub fn setup_int13_floppy_get_params(cpu: &mut CpuState) {
    cpu.set_ah(INT13_AH_GET_DRIVE_PARAMS);
    cpu.set_al(0);
    cpu.set_gpr_u8_low(CpuState::RDX, INT13_DRIVE_FD0);
}

/// Convenience: set up INT 13h AH=15h registers for floppy get disk type.
pub fn setup_int13_floppy_get_disk_type(cpu: &mut CpuState) {
    cpu.set_ah(INT13_AH_GET_DISK_TYPE);
    cpu.set_al(0);
    cpu.set_gpr_u8_low(CpuState::RDX, INT13_DRIVE_FD0);
}

/// Convenience: set up INT 13h AH=4Bh AL=00h CD get-status (`DS:SI` packet).
pub fn setup_int13_cd_get_status(cpu: &mut CpuState, ds: u16, si: u16) {
    cpu.set_ah(INT13_AH_CDROM_EMULATION);
    cpu.set_al(INT13_CD_AL_GET_STATUS);
    cpu.set_gpr_u8_low(CpuState::RDX, INT13_DRIVE_CD0);
    cpu.set_gpr_u16(CpuState::RSI, si);
    cpu.ds = x86_core::SegmentReg::real_mode(ds);
}

/// Write a DAP and set AH=42h / DS:SI / DL=`E0h` for CD extended read.
pub fn setup_int13_cd_ext_read(
    machine: &mut Machine,
    dap_phys: u64,
    lba: u64,
    count: u16,
    buf_seg: u16,
    buf_off: u16,
) {
    write_dap(machine, dap_phys, lba, count, buf_seg, buf_off);
    machine.cpu.set_ah(INT13_AH_EXT_READ);
    machine.cpu.set_gpr_u8_low(CpuState::RDX, INT13_DRIVE_CD0);
    machine.cpu.set_gpr_u16(CpuState::RSI, dap_phys as u16);
    machine.cpu.ds = x86_core::SegmentReg::real_mode(0);
}

/// Set up AH=48h for CD with a result buffer at physical `buf_phys`.
pub fn setup_int13_cd_ext_get_params(machine: &mut Machine, buf_phys: u64, buf_size: u16) {
    machine
        .mem
        .write_u8(buf_phys, (buf_size & 0xFF) as u8)
        .unwrap();
    machine
        .mem
        .write_u8(buf_phys + 1, (buf_size >> 8) as u8)
        .unwrap();
    for i in 2..usize::from(buf_size.max(INT13_EDD_PARAMS_SIZE_MIN)) {
        machine.mem.write_u8(buf_phys + i as u64, 0).unwrap();
    }
    machine.cpu.set_ah(INT13_AH_EXT_GET_PARAMS);
    machine.cpu.set_gpr_u8_low(CpuState::RDX, INT13_DRIVE_CD0);
    machine.cpu.set_gpr_u16(CpuState::RSI, buf_phys as u16);
    machine.cpu.ds = x86_core::SegmentReg::real_mode(0);
}

/// Convenience: set up INT 13h AH=41h for CD extensions check.
pub fn setup_int13_cd_check_extensions(cpu: &mut CpuState) {
    cpu.set_ah(INT13_AH_CHECK_EXTENSIONS);
    cpu.set_gpr_u16(CpuState::RBX, INT13_EXT_MAGIC_IN);
    cpu.set_gpr_u8_low(CpuState::RDX, INT13_DRIVE_CD0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mbr::{MBR_PHYS_ADDR, MBR_SECTOR_SIZE, MBR_SIGNATURE_HI, MBR_SIGNATURE_LO};
    use devices::FDC_1440_IMAGE_SIZE;

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
        assert_eq!(m.cpu.al(), 0);
        assert_eq!(m.cpu.gpr_u8_low(CpuState::RBX), 0);
    }

    /// Spec: unsupported AH → invalid function.
    #[test]
    fn int13_unsupported_ah_fails() {
        let mut m = Machine::with_ide(64 * 1024, synthetic_disk(1));
        m.cpu.set_ah(0x05); // Format track — out of scope
        m.cpu.set_gpr_u8_low(CpuState::RDX, INT13_DRIVE_HD0);
        m.service_int13_hd();
        assert!(cf(&m.cpu));
        assert_eq!(m.cpu.ah(), INT13_STATUS_INVALID);
    }

    /// Spec: IBM BIOS INT 13h AH=03h — write ES:BX to CHS (0,0,1) / LBA0.
    #[test]
    fn int13_ah03_writes_lba0_from_es_bx() {
        let mut m = Machine::with_ide(64 * 1024, synthetic_disk(4));
        for i in 0..INT13_SECTOR_SIZE {
            m.mem.write_u8(0x9000 + i as u64, 0x5A).unwrap();
        }
        m.mem.write_u8(0x9000, 0xE9).unwrap();
        m.mem.write_u8(0x9000 + 510, MBR_SIGNATURE_LO).unwrap();
        m.mem.write_u8(0x9000 + 511, MBR_SIGNATURE_HI).unwrap();
        setup_int13_hd_write(&mut m.cpu, 0, 0, 1, 1, 0x0000, 0x9000);
        m.service_int13_hd();

        assert!(!cf(&m.cpu), "CF clear on success");
        assert_eq!(m.cpu.ah(), INT13_STATUS_OK);
        assert_eq!(m.cpu.al(), 1);
        assert_eq!(m.ide.image[0], 0xE9);
        assert_eq!(m.ide.image[1], 0x5A);
        assert_eq!(m.ide.image[510], MBR_SIGNATURE_LO);
        assert_eq!(m.ide.image[511], MBR_SIGNATURE_HI);
    }

    /// Spec: multi-sector AH=03h writes consecutive LBAs.
    #[test]
    fn int13_ah03_writes_two_sectors() {
        let mut m = Machine::with_ide(64 * 1024, synthetic_disk(4));
        for i in 0..(2 * INT13_SECTOR_SIZE) {
            m.mem.write_u8(0xA000 + i as u64, 0x11).unwrap();
        }
        m.mem.write_u8(0xA000, 0xAA).unwrap();
        m.mem
            .write_u8(0xA000 + INT13_SECTOR_SIZE as u64, 0xBB)
            .unwrap();
        setup_int13_hd_write(&mut m.cpu, 0, 0, 1, 2, 0x0000, 0xA000);
        m.service_int13_hd();
        assert!(!cf(&m.cpu));
        assert_eq!(m.cpu.al(), 2);
        assert_eq!(m.ide.image[0], 0xAA);
        assert_eq!(m.ide.image[INT13_SECTOR_SIZE], 0xBB);
    }

    /// Spec: AH=03h then AH=02h round-trips the same buffer.
    #[test]
    fn int13_ah03_then_ah02_round_trip() {
        let mut m = Machine::with_ide(64 * 1024, synthetic_disk(4));
        for i in 0..INT13_SECTOR_SIZE {
            m.mem.write_u8(0xB000 + i as u64, (i & 0xFF) as u8).unwrap();
        }
        setup_int13_hd_write(&mut m.cpu, 0, 0, 2, 1, 0x0000, 0xB000);
        m.service_int13_hd();
        assert!(!cf(&m.cpu));

        setup_int13_hd_read(&mut m.cpu, 0, 0, 2, 1, 0x0000, 0xC000);
        m.service_int13_hd();
        assert!(!cf(&m.cpu));
        for i in 0..INT13_SECTOR_SIZE {
            assert_eq!(m.mem.read_u8(0xC000 + i as u64).unwrap(), (i & 0xFF) as u8);
        }
    }

    /// Spec: AH=03h OOB / no media mirror AH=02h status codes.
    #[test]
    fn int13_ah03_rejects_oob_and_no_media() {
        let mut m = Machine::with_ide(64 * 1024, synthetic_disk(2));
        assert_eq!(
            m.int13_hd_write_chs_from_phys(0, 0, 1, 8, 0x7000),
            Err(INT13_STATUS_SECTOR_NOT_FOUND)
        );
        let mut bare = Machine::new(64 * 1024);
        setup_int13_hd_write(&mut bare.cpu, 0, 0, 1, 1, 0x0000, 0x7000);
        bare.service_int13_hd();
        assert!(cf(&bare.cpu));
        assert_eq!(bare.cpu.ah(), INT13_STATUS_TIMEOUT);
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

    /// Spec: IBM/MS INT 13h Extensions AH=41h — magic BX handshake + packet/EDD bits.
    #[test]
    fn int13_ah41_reports_extensions() {
        let mut m = Machine::with_ide(64 * 1024, synthetic_disk(2));
        m.cpu.set_ah(INT13_AH_CHECK_EXTENSIONS);
        m.cpu.set_gpr_u16(CpuState::RBX, INT13_EXT_MAGIC_IN);
        m.cpu.set_gpr_u8_low(CpuState::RDX, INT13_DRIVE_HD0);
        m.service_int13_hd();
        assert!(!cf(&m.cpu));
        assert_eq!(m.cpu.ah(), INT13_EXT_VERSION);
        assert_eq!(m.cpu.gpr_u16(CpuState::RBX), INT13_EXT_MAGIC_OUT);
        assert_eq!(m.cpu.gpr_u16(CpuState::RCX), INT13_EXT_CX_SUPPORTED);
        assert_eq!(
            m.cpu.gpr_u16(CpuState::RCX) & INT13_EXT_CX_PACKET,
            INT13_EXT_CX_PACKET
        );
        assert_eq!(
            m.cpu.gpr_u16(CpuState::RCX) & INT13_EXT_CX_EDD,
            INT13_EXT_CX_EDD
        );
    }

    /// Spec: AH=41h requires BX=55AAh.
    #[test]
    fn int13_ah41_rejects_bad_magic() {
        let mut m = Machine::with_ide(64 * 1024, synthetic_disk(1));
        m.cpu.set_ah(INT13_AH_CHECK_EXTENSIONS);
        m.cpu.set_gpr_u16(CpuState::RBX, 0x1234);
        m.cpu.set_gpr_u8_low(CpuState::RDX, INT13_DRIVE_HD0);
        m.service_int13_hd();
        assert!(cf(&m.cpu));
        assert_eq!(m.cpu.ah(), INT13_STATUS_INVALID);
    }

    /// Spec: AH=42h reads LBA via Disk Address Packet into seg:off.
    #[test]
    fn int13_ah42_ext_read_lba0() {
        let mut m = Machine::with_ide(64 * 1024, synthetic_disk(4));
        setup_int13_hd_ext_read(&mut m, 0x5000, 0, 1, 0x0000, 0x7C00);
        m.service_int13_hd();
        assert!(!cf(&m.cpu));
        assert_eq!(m.cpu.ah(), INT13_STATUS_OK);
        assert_eq!(m.mem.read_u8(MBR_PHYS_ADDR).unwrap(), 0xF4);
        assert_eq!(m.mem.read_u8(MBR_PHYS_ADDR + 510).unwrap(), 0x55);
    }

    /// Spec: AH=42h multi-block LBA read.
    #[test]
    fn int13_ah42_ext_read_two_blocks() {
        let mut m = Machine::with_ide(64 * 1024, synthetic_disk(4));
        setup_int13_hd_ext_read(&mut m, 0x5100, 0, 2, 0x0000, 0x8000);
        m.service_int13_hd();
        assert!(!cf(&m.cpu));
        assert_eq!(m.mem.read_u8(0x8000).unwrap(), 0xF4);
        assert_eq!(m.mem.read_u8(0x8200).unwrap(), 0xA5);
    }

    /// Spec: AH=42h OOB LBA → sector not found; tiny DAP → invalid.
    #[test]
    fn int13_ah42_rejects_oob_and_short_dap() {
        let mut m = Machine::with_ide(64 * 1024, synthetic_disk(2));
        setup_int13_hd_ext_read(&mut m, 0x5200, 8, 1, 0x0000, 0x7000);
        m.service_int13_hd();
        assert!(cf(&m.cpu));
        assert_eq!(m.cpu.ah(), INT13_STATUS_SECTOR_NOT_FOUND);

        m.mem.write_u8(0x5300, 0x08).unwrap(); // size < 10h
        m.cpu.set_ah(INT13_AH_EXT_READ);
        m.cpu.set_gpr_u8_low(CpuState::RDX, INT13_DRIVE_HD0);
        m.cpu.set_gpr_u16(CpuState::RSI, 0x5300);
        m.cpu.ds = x86_core::SegmentReg::real_mode(0);
        m.service_int13_hd();
        assert!(cf(&m.cpu));
        assert_eq!(m.cpu.ah(), INT13_STATUS_INVALID);
    }

    /// Spec: RBIL INT 13h AH=43h — DAP LBA write from seg:off into IDE image.
    #[test]
    fn int13_ah43_ext_write_lba0() {
        let mut m = Machine::with_ide(64 * 1024, synthetic_disk(2));
        for i in 0..INT13_SECTOR_SIZE {
            m.mem.write_u8(0x9000 + i as u64, 0x5A).unwrap();
        }
        setup_int13_hd_ext_write(&mut m, 0x5400, 0, 1, 0x0000, 0x9000);
        m.service_int13_hd();
        assert!(!cf(&m.cpu));
        assert_eq!(m.cpu.ah(), INT13_STATUS_OK);
        assert!(m.ide.image[..INT13_SECTOR_SIZE].iter().all(|&b| b == 0x5A));
    }

    /// Spec: AH=43h then AH=42h round-trips the same LBA buffer.
    #[test]
    fn int13_ah43_then_ah42_round_trip() {
        let mut m = Machine::with_ide(64 * 1024, synthetic_disk(4));
        for i in 0..INT13_SECTOR_SIZE {
            m.mem.write_u8(0xA000 + i as u64, (i & 0xFF) as u8).unwrap();
        }
        setup_int13_hd_ext_write(&mut m, 0x5500, 1, 1, 0x0000, 0xA000);
        m.service_int13_hd();
        assert!(!cf(&m.cpu));

        setup_int13_hd_ext_read(&mut m, 0x5600, 1, 1, 0x0000, 0xB000);
        m.service_int13_hd();
        assert!(!cf(&m.cpu));
        for i in 0..INT13_SECTOR_SIZE {
            assert_eq!(m.mem.read_u8(0xB000 + i as u64).unwrap(), (i & 0xFF) as u8);
        }
    }

    /// Spec: AH=43h multi-block LBA write.
    #[test]
    fn int13_ah43_ext_write_multi() {
        let mut m = Machine::with_ide(64 * 1024, synthetic_disk(4));
        for i in 0..(2 * INT13_SECTOR_SIZE) {
            m.mem.write_u8(0xC000 + i as u64, 0xE7).unwrap();
        }
        setup_int13_hd_ext_write(&mut m, 0x5700, 0, 2, 0x0000, 0xC000);
        m.service_int13_hd();
        assert!(!cf(&m.cpu));
        assert!(m.ide.image[..2 * INT13_SECTOR_SIZE]
            .iter()
            .all(|&b| b == 0xE7));
    }

    /// Spec: AH=43h OOB LBA → sector not found; no media → timeout.
    #[test]
    fn int13_ah43_ext_write_errors() {
        let mut m = Machine::with_ide(64 * 1024, synthetic_disk(2));
        setup_int13_hd_ext_write(&mut m, 0x5800, 8, 1, 0x0000, 0x7000);
        m.service_int13_hd();
        assert!(cf(&m.cpu));
        assert_eq!(m.cpu.ah(), INT13_STATUS_SECTOR_NOT_FOUND);

        let mut bare = Machine::new(64 * 1024);
        setup_int13_hd_ext_write(&mut bare, 0x5900, 0, 1, 0x0000, 0x7000);
        bare.service_int13_hd();
        assert!(cf(&bare.cpu));
        assert_eq!(bare.cpu.ah(), INT13_STATUS_TIMEOUT);
    }

    fn synthetic_floppy_boot() -> Vec<u8> {
        let mut img = vec![0u8; FDC_1440_IMAGE_SIZE];
        img[0] = 0xF4;
        img[510] = MBR_SIGNATURE_LO;
        img[511] = MBR_SIGNATURE_HI;
        // Sector 2 (CHS 0,0,2) marker
        img[INT13_SECTOR_SIZE] = 0xB2;
        img[INT13_SECTOR_SIZE + 1] = 0x2B;
        img
    }

    /// Spec: IBM BIOS INT 13h floppy AH=02h — CHS (0,0,1) via FDC media.
    #[test]
    fn int13_floppy_ah02_reads_boot_sector() {
        let mut m = Machine::with_floppy(64 * 1024, synthetic_floppy_boot()).expect("floppy");
        setup_int13_floppy_read(&mut m.cpu, 0, 0, 1, 1, 0x0000, 0x7C00);
        m.service_int13_floppy();
        assert!(!cf(&m.cpu));
        assert_eq!(m.cpu.ah(), INT13_STATUS_OK);
        assert_eq!(m.cpu.al(), 1);
        assert_eq!(m.mem.read_u8(MBR_PHYS_ADDR).unwrap(), 0xF4);
        assert_eq!(m.mem.read_u8(MBR_PHYS_ADDR + 510).unwrap(), 0x55);
    }

    /// Spec: multi-sector floppy AH=02h advances consecutive CHS.
    #[test]
    fn int13_floppy_ah02_reads_two_sectors() {
        let mut m = Machine::with_floppy(64 * 1024, synthetic_floppy_boot()).expect("floppy");
        setup_int13_floppy_read(&mut m.cpu, 0, 0, 1, 2, 0x0000, 0x8000);
        m.service_int13_floppy();
        assert!(!cf(&m.cpu));
        assert_eq!(m.mem.read_u8(0x8000).unwrap(), 0xF4);
        assert_eq!(
            m.mem.read_u8(0x8000 + INT13_SECTOR_SIZE as u64).unwrap(),
            0xB2
        );
        assert_eq!(
            m.mem
                .read_u8(0x8000 + INT13_SECTOR_SIZE as u64 + 1)
                .unwrap(),
            0x2B
        );
    }

    /// Spec: floppy AH=03h writes ES:BX into FDC image at CHS.
    #[test]
    fn int13_floppy_ah03_writes_sector() {
        let mut m = Machine::with_floppy(64 * 1024, synthetic_floppy_boot()).expect("floppy");
        for i in 0..INT13_SECTOR_SIZE {
            m.mem.write_u8(0x9000 + i as u64, 0x3C).unwrap();
        }
        setup_int13_floppy_write(&mut m.cpu, 0, 0, 1, 1, 0x0000, 0x9000);
        m.service_int13_floppy();
        assert!(!cf(&m.cpu));
        let sector = m.fdc.read_sector(0, 0, 1).expect("sector");
        assert!(sector.iter().all(|&b| b == 0x3C));
    }

    /// Spec: floppy AH=03h then AH=02h round-trip; WP → AH=03h.
    #[test]
    fn int13_floppy_write_read_and_wp() {
        let mut m = Machine::with_floppy(64 * 1024, synthetic_floppy_boot()).expect("floppy");
        for i in 0..INT13_SECTOR_SIZE {
            m.mem.write_u8(0xA000 + i as u64, (i & 0xFF) as u8).unwrap();
        }
        setup_int13_floppy_write(&mut m.cpu, 0, 0, 2, 1, 0x0000, 0xA000);
        m.service_int13();
        assert!(!cf(&m.cpu));
        setup_int13_floppy_read(&mut m.cpu, 0, 0, 2, 1, 0x0000, 0xB000);
        m.service_int13();
        assert!(!cf(&m.cpu));
        for i in 0..INT13_SECTOR_SIZE {
            assert_eq!(m.mem.read_u8(0xB000 + i as u64).unwrap(), (i & 0xFF) as u8);
        }

        m.fdc.set_write_protected(true);
        setup_int13_floppy_write(&mut m.cpu, 0, 0, 1, 1, 0x0000, 0xA000);
        m.service_int13_floppy();
        assert!(cf(&m.cpu));
        assert_eq!(m.cpu.ah(), INT13_STATUS_WRITE_PROTECTED);
    }

    /// Spec: no floppy media → timeout; HD DL rejected by floppy service.
    #[test]
    fn int13_floppy_errors() {
        let mut bare = Machine::new(64 * 1024);
        setup_int13_floppy_read(&mut bare.cpu, 0, 0, 1, 1, 0x0000, 0x7C00);
        bare.service_int13_floppy();
        assert!(cf(&bare.cpu));
        assert_eq!(bare.cpu.ah(), INT13_STATUS_TIMEOUT);

        let mut m = Machine::with_floppy(64 * 1024, synthetic_floppy_boot()).expect("floppy");
        setup_int13_floppy_read(&mut m.cpu, 0, 0, 1, 1, 0x0000, 0x7C00);
        m.cpu.set_gpr_u8_low(CpuState::RDX, INT13_DRIVE_HD0);
        m.service_int13_floppy();
        assert!(cf(&m.cpu));
        assert_eq!(m.cpu.ah(), INT13_STATUS_INVALID);
    }

    /// Spec: IBM/RBIL INT 13h AH=08h floppy — 1.44MB max CHS + `BL=04h`.
    #[test]
    fn int13_floppy_ah08_returns_1440_geometry() {
        let mut m = Machine::with_floppy(64 * 1024, synthetic_floppy_boot()).expect("floppy");
        setup_int13_floppy_get_params(&mut m.cpu);
        m.service_int13_floppy();
        assert!(!cf(&m.cpu));
        assert_eq!(m.cpu.ah(), INT13_STATUS_OK);
        assert_eq!(m.cpu.gpr_u8_low(CpuState::RBX), INT13_FLOPPY_TYPE_1440);
        let (cyl, spt) = unpack_cx(m.cpu.gpr_u16(CpuState::RCX));
        assert_eq!(cyl, u16::from(INT13_FLOPPY_MAX_CYLINDER));
        assert_eq!(spt, INT13_FLOPPY_SPT);
        assert_eq!(m.cpu.gpr_u8(4 + CpuState::RDX), INT13_FLOPPY_MAX_HEAD);
        assert_eq!(m.cpu.gpr_u8_low(CpuState::RDX), 1);
    }

    /// Spec: AH=08h with no floppy media → timeout (honest CF/AH).
    #[test]
    fn int13_floppy_ah08_no_media_timeout() {
        let mut bare = Machine::new(64 * 1024);
        setup_int13_floppy_get_params(&mut bare.cpu);
        bare.service_int13();
        assert!(cf(&bare.cpu));
        assert_eq!(bare.cpu.ah(), INT13_STATUS_TIMEOUT);
    }

    /// Spec: IBM/RBIL INT 13h AH=15h — change-line floppy when media attached.
    #[test]
    fn int13_floppy_ah15_reports_change_line_type() {
        let mut m = Machine::with_floppy(64 * 1024, synthetic_floppy_boot()).expect("floppy");
        setup_int13_floppy_get_disk_type(&mut m.cpu);
        m.service_int13();
        assert!(!cf(&m.cpu));
        assert_eq!(m.cpu.ah(), INT13_DISK_TYPE_FLOPPY_CHANGE_LINE);
    }

    /// Spec: AH=15h with no media → type `00h` (no such drive), CF clear.
    #[test]
    fn int13_floppy_ah15_no_media_type_none() {
        let mut bare = Machine::new(64 * 1024);
        setup_int13_floppy_get_disk_type(&mut bare.cpu);
        bare.service_int13_floppy();
        assert!(!cf(&bare.cpu));
        assert_eq!(bare.cpu.ah(), INT13_DISK_TYPE_NONE);
    }

    /// Spec: Phoenix EDD AH=48h — geometry + total sectors from IDE image.
    #[test]
    fn int13_ah48_returns_edd_params() {
        let sectors = 16 * 63 * 2;
        let mut m = Machine::with_ide(64 * 1024, synthetic_disk(sectors));
        setup_int13_hd_ext_get_params(&mut m, 0x6000, INT13_EDD_PARAMS_SIZE_MIN);
        m.service_int13_hd();
        assert!(!cf(&m.cpu));
        assert_eq!(m.cpu.ah(), INT13_STATUS_OK);
        assert_eq!(m.read_guest_u16(0x6000).unwrap(), INT13_EDD_PARAMS_SIZE_MIN);
        assert_eq!(
            m.read_guest_u16(0x6002).unwrap(),
            INT13_EDD_INFO_GEOMETRY_VALID
        );
        // Two cylinders of 16*63 → cyl_count = 2.
        assert_eq!(
            u32::from(m.mem.read_u8(0x6004).unwrap())
                | (u32::from(m.mem.read_u8(0x6005).unwrap()) << 8)
                | (u32::from(m.mem.read_u8(0x6006).unwrap()) << 16)
                | (u32::from(m.mem.read_u8(0x6007).unwrap()) << 24),
            2
        );
        assert_eq!(
            u32::from(m.mem.read_u8(0x6008).unwrap())
                | (u32::from(m.mem.read_u8(0x6009).unwrap()) << 8),
            u32::from(INT13_HD_HEADS)
        );
        assert_eq!(
            u32::from(m.mem.read_u8(0x600C).unwrap())
                | (u32::from(m.mem.read_u8(0x600D).unwrap()) << 8),
            u32::from(INT13_HD_SPT)
        );
        assert_eq!(m.read_guest_u64(0x6010).unwrap(), sectors as u64);
        assert_eq!(m.read_guest_u16(0x6018).unwrap(), INT13_SECTOR_SIZE as u16);
    }

    /// Spec: AH=48h rejects short buffers and missing media.
    #[test]
    fn int13_ah48_rejects_short_buffer_and_no_media() {
        let mut m = Machine::with_ide(64 * 1024, synthetic_disk(2));
        setup_int13_hd_ext_get_params(&mut m, 0x6100, 0x10);
        m.service_int13_hd();
        assert!(cf(&m.cpu));
        assert_eq!(m.cpu.ah(), INT13_STATUS_INVALID);

        let mut bare = Machine::new(64 * 1024);
        setup_int13_hd_ext_get_params(&mut bare, 0x6100, INT13_EDD_PARAMS_SIZE_MIN);
        bare.service_int13_hd();
        assert!(cf(&bare.cpu));
        assert_eq!(bare.cpu.ah(), INT13_STATUS_TIMEOUT);
    }

    fn write_iso_sector(img: &mut [u8], lba: u32, data: &[u8]) {
        let start = lba as usize * firmware_interface::EL_TORITO_SECTOR_BYTES;
        img[start..start + data.len()].copy_from_slice(data);
    }

    /// Minimal bootable El Torito ISO (no-emul) for AH=4Bh tests.
    fn synthetic_eltorito_iso() -> Vec<u8> {
        use firmware_interface::{
            EL_TORITO_BOOTABLE, EL_TORITO_BOOT_SYSTEM_ID, EL_TORITO_KEY_55, EL_TORITO_KEY_AA,
            EL_TORITO_MEDIA_NO_EMUL, EL_TORITO_PLATFORM_X86, EL_TORITO_SECTOR_BYTES,
            EL_TORITO_VALIDATION_HEADER_ID, ISO9660_STANDARD_ID, ISO9660_VD_BOOT_RECORD,
            ISO9660_VD_TERMINATOR,
        };
        let mut img = vec![0u8; 32 * EL_TORITO_SECTOR_BYTES];
        let mut pvd = vec![0u8; EL_TORITO_SECTOR_BYTES];
        pvd[0] = 1;
        pvd[1..6].copy_from_slice(ISO9660_STANDARD_ID);
        pvd[6] = 1;
        write_iso_sector(&mut img, 16, &pvd);

        let mut br = vec![0u8; EL_TORITO_SECTOR_BYTES];
        br[0] = ISO9660_VD_BOOT_RECORD;
        br[1..6].copy_from_slice(ISO9660_STANDARD_ID);
        br[6] = 1;
        br[7..7 + EL_TORITO_BOOT_SYSTEM_ID.len()].copy_from_slice(EL_TORITO_BOOT_SYSTEM_ID);
        let catalog_lba = 20u32;
        br[0x47..0x4B].copy_from_slice(&catalog_lba.to_le_bytes());
        write_iso_sector(&mut img, 17, &br);

        let mut term = vec![0u8; EL_TORITO_SECTOR_BYTES];
        term[0] = ISO9660_VD_TERMINATOR;
        term[1..6].copy_from_slice(ISO9660_STANDARD_ID);
        term[6] = 1;
        write_iso_sector(&mut img, 18, &term);

        let mut cat = vec![0u8; EL_TORITO_SECTOR_BYTES];
        let mut validation = [0u8; 32];
        validation[0] = EL_TORITO_VALIDATION_HEADER_ID;
        validation[1] = EL_TORITO_PLATFORM_X86;
        validation[30] = EL_TORITO_KEY_55;
        validation[31] = EL_TORITO_KEY_AA;
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
        cat[33] = EL_TORITO_MEDIA_NO_EMUL;
        cat[38..40].copy_from_slice(&4u16.to_le_bytes());
        cat[40..44].copy_from_slice(&24u32.to_le_bytes());
        write_iso_sector(&mut img, catalog_lba, &cat);

        let mut boot = vec![0x90u8; EL_TORITO_SECTOR_BYTES];
        boot[0] = 0xF4;
        write_iso_sector(&mut img, 24, &boot);
        img
    }

    /// Spec: El Torito / RBIL INT 13h AH=4Bh AL=00h — fill CD status packet.
    #[test]
    fn int13_cd_ah4b_get_status_packet() {
        let mut m = Machine::new(64 * 1024);
        m.attach_atapi_cdrom_image(synthetic_eltorito_iso());
        setup_int13_cd_get_status(&mut m.cpu, 0x0000, 0x5000);
        m.service_int13();
        assert!(!cf(&m.cpu));
        assert_eq!(m.cpu.ah(), INT13_STATUS_OK);
        assert_eq!(m.mem.read_u8(0x5000).unwrap(), INT13_CD_SPEC_PACKET_SIZE);
        assert_eq!(
            m.mem.read_u8(0x5001).unwrap(),
            firmware_interface::EL_TORITO_MEDIA_NO_EMUL
        );
        assert_eq!(m.mem.read_u8(0x5002).unwrap(), INT13_DRIVE_CD0);
        let rba = u32::from(m.mem.read_u8(0x5004).unwrap())
            | (u32::from(m.mem.read_u8(0x5005).unwrap()) << 8)
            | (u32::from(m.mem.read_u8(0x5006).unwrap()) << 16)
            | (u32::from(m.mem.read_u8(0x5007).unwrap()) << 24);
        assert_eq!(rba, 24);
        let load_seg = u16::from(m.mem.read_u8(0x500C).unwrap())
            | (u16::from(m.mem.read_u8(0x500D).unwrap()) << 8);
        assert_eq!(load_seg, firmware_interface::EL_TORITO_DEFAULT_LOAD_SEGMENT);
        let sectors = u16::from(m.mem.read_u8(0x500E).unwrap())
            | (u16::from(m.mem.read_u8(0x500F).unwrap()) << 8);
        assert_eq!(sectors, 4);
    }

    /// Spec: AH=4Bh with no ATAPI medium → timeout; terminate AL rejected.
    #[test]
    fn int13_cd_ah4b_errors() {
        let mut bare = Machine::new(64 * 1024);
        setup_int13_cd_get_status(&mut bare.cpu, 0x0000, 0x5000);
        bare.service_int13_cd();
        assert!(cf(&bare.cpu));
        assert_eq!(bare.cpu.ah(), INT13_STATUS_TIMEOUT);

        let mut m = Machine::new(64 * 1024);
        m.attach_atapi_cdrom_image(synthetic_eltorito_iso());
        setup_int13_cd_get_status(&mut m.cpu, 0x0000, 0x5000);
        m.cpu.set_al(0x01); // terminate-emulation — unsupported
        m.service_int13_cd();
        assert!(cf(&m.cpu));
        assert_eq!(m.cpu.ah(), INT13_STATUS_INVALID);
    }

    /// Spec: IBM/MS INT 13h Extensions AH=41h on CD `DL=E0h`.
    #[test]
    fn int13_cd_ah41_extensions_present() {
        let mut m = Machine::new(64 * 1024);
        m.attach_atapi_cdrom_image(synthetic_eltorito_iso());
        setup_int13_cd_check_extensions(&mut m.cpu);
        m.service_int13();
        assert!(!cf(&m.cpu));
        assert_eq!(m.cpu.ah(), INT13_EXT_VERSION);
        assert_eq!(m.cpu.gpr_u16(CpuState::RBX), INT13_EXT_MAGIC_OUT);
        assert_eq!(m.cpu.gpr_u16(CpuState::RCX), INT13_EXT_CX_SUPPORTED);
    }

    /// Spec: AH=42h CD DAP read — 2048-byte Mode-1 LBA from ATAPI medium.
    #[test]
    fn int13_cd_ah42_reads_mode1_lba() {
        let mut m = Machine::new(128 * 1024);
        m.attach_atapi_cdrom_image(synthetic_eltorito_iso());
        // LBA 24 holds the El Torito boot image (HLT at offset 0).
        setup_int13_cd_ext_read(&mut m, 0x4000, 24, 1, 0x0000, 0x8000);
        m.service_int13();
        assert!(!cf(&m.cpu), "CF clear on CD AH=42h");
        assert_eq!(m.cpu.ah(), INT13_STATUS_OK);
        assert_eq!(m.mem.read_u8(0x8000).unwrap(), 0xF4);
        // OOB LBA → sector not found.
        setup_int13_cd_ext_read(&mut m, 0x4000, 0xFFFF, 1, 0x0000, 0x8000);
        m.service_int13_cd();
        assert!(cf(&m.cpu));
        assert_eq!(m.cpu.ah(), INT13_STATUS_SECTOR_NOT_FOUND);
    }

    /// Spec: Phoenix EDD AH=48h on CD — total blocks + sector size 2048.
    #[test]
    fn int13_cd_ah48_returns_edd_params() {
        let iso = synthetic_eltorito_iso();
        let blocks = (iso.len() / INT13_CD_SECTOR_SIZE) as u64;
        let mut m = Machine::new(64 * 1024);
        m.attach_atapi_cdrom_image(iso);
        setup_int13_cd_ext_get_params(&mut m, 0x6000, INT13_EDD_PARAMS_SIZE_MIN);
        m.service_int13();
        assert!(!cf(&m.cpu));
        assert_eq!(m.cpu.ah(), INT13_STATUS_OK);
        assert_eq!(m.read_guest_u16(0x6000).unwrap(), INT13_EDD_PARAMS_SIZE_MIN);
        assert_eq!(m.read_guest_u64(0x6010).unwrap(), blocks);
        assert_eq!(
            m.read_guest_u16(0x6018).unwrap(),
            INT13_CD_SECTOR_SIZE as u16
        );
        // No medium → timeout.
        let mut bare = Machine::new(64 * 1024);
        setup_int13_cd_ext_get_params(&mut bare, 0x6000, INT13_EDD_PARAMS_SIZE_MIN);
        bare.service_int13_cd();
        assert!(cf(&bare.cpu));
        assert_eq!(bare.cpu.ah(), INT13_STATUS_TIMEOUT);
    }
}
