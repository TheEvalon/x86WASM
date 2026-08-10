//! Host-side IBM BIOS INT 13h hard-disk subset (AH=00/02/03/08 + 41h/42h/43h)
//! and floppy subset (AH=00/02/03, `DL=00h`).
//!
//! Closest approach in-tree to SeaBIOS disk services: a **host** dispatcher that
//! applies classic INT 13h register conventions against the primary IDE image
//! or attached FDC media, mirroring [`crate::mbr::Machine::load_mbr_to_7c00`]'s
//! host-side media path. This is **not** a guest IVT BIOS and not CHS
//! translation modes.
//!
//! Spec: IBM PC BIOS INT 13h Disk Services (AH=00h reset, AH=02h read sectors,
//! AH=03h write sectors, AH=08h get drive parameters); IBM/Microsoft INT 13h
//! Extensions / RBIL (AH=41h check extensions, AH=42h extended read, AH=43h
//! extended write). ATA IDENTIFY obsolete geometry 16 heads / 63
//! sectors-per-track (matches `IdePrimary` IDENTIFY words 3/6). Floppy uses
//! fixed 1.44MB geometry (80/2/18) via `Fdc82077::read_sector` /
//! `Fdc82077::write_sector`.

use crate::{Machine, MachineError};
use devices::{
    FDC_1440_CYLINDERS, FDC_1440_HEADS, FDC_1440_SECTORS_PER_TRACK, FDC_SECTOR_SIZE,
};
use x86_core::CpuState;

/// First floppy (`DL`).
pub const INT13_DRIVE_FD0: u8 = 0x00;
/// First hard disk (IBM BIOS `DL`).
pub const INT13_DRIVE_HD0: u8 = 0x80;

/// AH=00h — reset disk system.
pub const INT13_AH_RESET: u8 = 0x00;
/// AH=02h — read disk sectors into `ES:BX`.
pub const INT13_AH_READ: u8 = 0x02;
/// AH=03h — write disk sectors from `ES:BX`.
pub const INT13_AH_WRITE: u8 = 0x03;
/// AH=08h — get drive parameters.
pub const INT13_AH_GET_DRIVE_PARAMS: u8 = 0x08;
/// AH=41h — check extensions present.
pub const INT13_AH_CHECK_EXTENSIONS: u8 = 0x41;
/// AH=42h — extended read sectors (Disk Address Packet).
pub const INT13_AH_EXT_READ: u8 = 0x42;
/// AH=43h — extended write sectors (Disk Address Packet).
pub const INT13_AH_EXT_WRITE: u8 = 0x43;

/// Magic `BX` input for AH=41h.
pub const INT13_EXT_MAGIC_IN: u16 = 0x55AA;
/// Magic `BX` output for AH=41h success.
pub const INT13_EXT_MAGIC_OUT: u16 = 0xAA55;
/// Major version returned in `AH` on AH=41h success (IBM/MS INT 13h Extensions).
pub const INT13_EXT_VERSION: u8 = 0x01;
/// `CX` bit 0 — packet-structure device access supported (AH=42h/43h here).
pub const INT13_EXT_CX_PACKET: u16 = 0x0001;
/// Minimum Disk Address Packet size (16 bytes).
pub const INT13_DAP_SIZE_MIN: u8 = 0x10;

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
            INT13_AH_WRITE => self.int13_hd_write_from_regs(),
            INT13_AH_GET_DRIVE_PARAMS => self.int13_hd_get_params(),
            INT13_AH_CHECK_EXTENSIONS => self.int13_hd_check_extensions(),
            INT13_AH_EXT_READ => self.int13_hd_ext_read_from_regs(),
            INT13_AH_EXT_WRITE => self.int13_hd_ext_write_from_regs(),
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
        // Bit 0 only: packet access (AH=42h/43h). Removable lock / EDD are out.
        self.cpu.set_gpr_u16(CpuState::RCX, INT13_EXT_CX_PACKET);
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
    /// Supports AH=00h reset, AH=02h read, AH=03h write against attached FDC
    /// 1.44MB media. Spec: IBM PC BIOS INT 13h floppy disk services; geometry
    /// matches [`FDC_1440_CYLINDERS`] / [`FDC_1440_HEADS`] /
    /// [`FDC_1440_SECTORS_PER_TRACK`].
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
            _ => self.int13_fail(INT13_STATUS_INVALID),
        }
    }

    /// Route INT 13h by `DL`: floppy `00h` or hard disk `80h`.
    pub fn service_int13(&mut self) {
        match self.cpu.gpr_u8_low(CpuState::RDX) {
            INT13_DRIVE_FD0 => self.service_int13_floppy(),
            INT13_DRIVE_HD0 => self.service_int13_hd(),
            _ => self.int13_fail(INT13_STATUS_INVALID),
        }
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

    /// Spec: IBM/MS INT 13h Extensions AH=41h — magic BX handshake + packet bit.
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
        assert_eq!(
            m.cpu.gpr_u16(CpuState::RCX) & INT13_EXT_CX_PACKET,
            INT13_EXT_CX_PACKET
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
        assert!(m.ide.image[..INT13_SECTOR_SIZE]
            .iter()
            .all(|&b| b == 0x5A));
    }

    /// Spec: AH=43h then AH=42h round-trips the same LBA buffer.
    #[test]
    fn int13_ah43_then_ah42_round_trip() {
        let mut m = Machine::with_ide(64 * 1024, synthetic_disk(4));
        for i in 0..INT13_SECTOR_SIZE {
            m.mem
                .write_u8(0xA000 + i as u64, (i & 0xFF) as u8)
                .unwrap();
        }
        setup_int13_hd_ext_write(&mut m, 0x5500, 1, 1, 0x0000, 0xA000);
        m.service_int13_hd();
        assert!(!cf(&m.cpu));

        setup_int13_hd_ext_read(&mut m, 0x5600, 1, 1, 0x0000, 0xB000);
        m.service_int13_hd();
        assert!(!cf(&m.cpu));
        for i in 0..INT13_SECTOR_SIZE {
            assert_eq!(
                m.mem.read_u8(0xB000 + i as u64).unwrap(),
                (i & 0xFF) as u8
            );
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
        assert_eq!(m.mem.read_u8(0x8000 + INT13_SECTOR_SIZE as u64).unwrap(), 0xB2);
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
            m.mem
                .write_u8(0xA000 + i as u64, (i & 0xFF) as u8)
                .unwrap();
        }
        setup_int13_floppy_write(&mut m.cpu, 0, 0, 2, 1, 0x0000, 0xA000);
        m.service_int13();
        assert!(!cf(&m.cpu));
        setup_int13_floppy_read(&mut m.cpu, 0, 0, 2, 1, 0x0000, 0xB000);
        m.service_int13();
        assert!(!cf(&m.cpu));
        for i in 0..INT13_SECTOR_SIZE {
            assert_eq!(
                m.mem.read_u8(0xB000 + i as u64).unwrap(),
                (i & 0xFF) as u8
            );
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
}
