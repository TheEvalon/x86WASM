//! Host-driven option-ROM initialization entry (map + far-call `base+3`).
//!
//! Isolated from MachineBus / port wiring so parallel device slices can merge
//! without fighting this file.
//!
//! Spec: PCI Firmware Specification / BIOS Boot Specification — PC-compatible
//! expansion ROM header (`55 AA`, size, checksum); initialization entry at
//! offset 3; classic BIOS far-calls `CS:IP = (base>>4):0003` and expects `RETF`.
//! R9 adds a POST-style scan of `0xC0000`–`0xDFFFF` on 2 KiB steps
//! (`docs/firmware-r9-option-rom-post-scan.md`).

use crate::{Machine, MachineError};
use firmware_interface::{
    option_rom_entry_cs_ip, prepare_option_rom, OPTION_ROM_BLOCK_SIZE, OPTION_ROM_HEADER_LEN,
    OPTION_ROM_REGION_BASE, OPTION_ROM_REGION_END, OPTION_ROM_SCAN_STEP, OPTION_ROM_SIGNATURE,
    VGA_OPTION_ROM_BASE,
};
use x86_core::{CpuState, SegmentReg};

/// Default real-mode stack for a host-driven option-ROM far call when `SP` is
/// too small to push a return frame.
///
/// Model choice for bring-up: `SS:SP = 0000:7C00` (classic below-boot-sector
/// scratch). Not SeaBIOS stack allocation.
pub const OPTION_ROM_INVOKE_DEFAULT_SP: u16 = 0x7C00;

/// Default resume `CS:IP` when the caller does not name one: a `HLT` planted at
/// physical `0x0500` (BIOS data area / DOS free RAM below EBDA conventions).
pub const OPTION_ROM_RESUME_PHYS: u64 = 0x0500;

/// One validated option ROM discovered by a legacy POST-style scan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OptionRomScanHit {
    /// Physical base on a 2 KiB boundary inside `0xC0000`–`0xDFFFF`.
    pub phys_base: u64,
    /// Initialization size in 512-byte blocks (header byte 2).
    pub blocks: u8,
}

impl OptionRomScanHit {
    /// Next scan address after this ROM (size rounded up to 2 KiB).
    ///
    /// Spec: classic BIOS option-ROM scan advances by the declared extent,
    /// aligned up to the 2 KiB scan step.
    pub fn next_scan_base(self) -> u64 {
        let bytes = u64::from(self.blocks) * OPTION_ROM_BLOCK_SIZE as u64;
        let aligned = bytes.div_ceil(OPTION_ROM_SCAN_STEP) * OPTION_ROM_SCAN_STEP;
        self.phys_base.saturating_add(aligned)
    }
}

impl Machine {
    /// Map a validated expansion ROM, then far-call its entry at offset 3.
    ///
    /// Combines [`Self::map_option_rom`] with [`Self::invoke_option_rom_entry`].
    /// `resume_cs` / `resume_ip` become the far-return target pushed for `RETF`.
    ///
    /// **Not** a SeaBIOS option-ROM scan, PnP BEV/BCV dispatch, or INT 10h.
    /// For the scan loop see [`Self::scan_option_rom_region`] /
    /// [`Self::post_scan_invoke_option_roms`].
    pub fn map_and_invoke_option_rom(
        &mut self,
        phys_base: u64,
        data: &[u8],
        resume_cs: u16,
        resume_ip: u16,
    ) -> Result<(), MachineError> {
        self.map_option_rom(phys_base, data)?;
        self.invoke_option_rom_entry(phys_base, resume_cs, resume_ip)
    }

    /// Map at [`VGA_OPTION_ROM_BASE`] and invoke, resuming at a host `HLT` stub.
    ///
    /// Plants `HLT` (`0xF4`) at [`OPTION_ROM_RESUME_PHYS`] and uses
    /// `CS:IP = 0000:0500` as the far-return target.
    pub fn map_and_invoke_vga_option_rom(&mut self, data: &[u8]) -> Result<(), MachineError> {
        self.plant_option_rom_resume_hlt()?;
        self.map_and_invoke_option_rom(
            VGA_OPTION_ROM_BASE,
            data,
            0x0000,
            OPTION_ROM_RESUME_PHYS as u16,
        )
    }

    /// Discover valid expansion ROMs currently visible in the legacy scan window.
    ///
    /// Spec: BIOS Boot Specification / IBM PC — scan `0xC0000`–`0xDFFFF` on
    /// 2 KiB steps for `55 AA`, verify size+checksum via [`prepare_option_rom`],
    /// then advance by the declared size rounded up to 2 KiB.
    ///
    /// Does **not** execute entries; use [`Self::post_scan_invoke_option_roms`]
    /// or [`Self::invoke_option_rom_entry`].
    pub fn scan_option_rom_region(&self) -> Vec<OptionRomScanHit> {
        let mut hits = Vec::new();
        let mut base = OPTION_ROM_REGION_BASE;
        while base < OPTION_ROM_REGION_END {
            match self.try_read_option_rom_hit(base) {
                Some(hit) => {
                    let next = hit.next_scan_base();
                    hits.push(hit);
                    base = next.max(base + OPTION_ROM_SCAN_STEP);
                }
                None => base += OPTION_ROM_SCAN_STEP,
            }
        }
        hits
    }

    /// POST-style: far-call every ROM found by [`Self::scan_option_rom_region`].
    ///
    /// For each hit: push `resume_cs:resume_ip`, set `CS:IP` to the entry, then
    /// step the guest until it returns to that resume point (or
    /// `steps_budget_per_rom` is exhausted). Plants `HLT` at the resume
    /// physical when `resume_cs == 0` and `resume_ip` matches
    /// [`OPTION_ROM_RESUME_PHYS`].
    ///
    /// Returns the number of ROMs successfully invoked and returned.
    ///
    /// Gaps vs SeaBIOS (documented): no PCI BDF in `AX`/`BX`/`DX`, no PnP
    /// header / BEV/BCV, no claim that SeaVGABIOS installs fonts or INT 10h.
    pub fn post_scan_invoke_option_roms(
        &mut self,
        resume_cs: u16,
        resume_ip: u16,
        steps_budget_per_rom: usize,
    ) -> Result<usize, MachineError> {
        if resume_cs == 0 && u64::from(resume_ip) == OPTION_ROM_RESUME_PHYS {
            self.plant_option_rom_resume_hlt()?;
        }
        let hits = self.scan_option_rom_region();
        let mut invoked = 0usize;
        for hit in hits {
            self.invoke_option_rom_entry(hit.phys_base, resume_cs, resume_ip)?;
            let mut steps = 0usize;
            while steps < steps_budget_per_rom {
                if self.cpu.cs.selector == resume_cs && self.cpu.ip16() == resume_ip {
                    break;
                }
                self.step()?;
                steps += 1;
            }
            if self.cpu.cs.selector != resume_cs || self.cpu.ip16() != resume_ip {
                return Err(MachineError::OptionRomScanDidNotReturn);
            }
            invoked += 1;
        }
        Ok(invoked)
    }

    /// Convenience: scan + invoke with the default `0000:0500` `HLT` resume.
    pub fn post_scan_invoke_option_roms_default(
        &mut self,
        steps_budget_per_rom: usize,
    ) -> Result<usize, MachineError> {
        self.post_scan_invoke_option_roms(
            0x0000,
            OPTION_ROM_RESUME_PHYS as u16,
            steps_budget_per_rom,
        )
    }

    /// Far-call the expansion ROM already mapped at `phys_base`.
    ///
    /// Spec: BIOS Boot Specification — initialization entry at offset 3;
    /// classic BIOS issues a far call so the ROM returns with `RETF`.
    ///
    /// Steps:
    /// 1. Re-read the mapped image and re-validate via [`prepare_option_rom`].
    /// 2. Ensure a real-mode stack can hold a 4-byte far return (`SS=0`,
    ///    [`OPTION_ROM_INVOKE_DEFAULT_SP`] when `SP < 4`).
    /// 3. Push `resume_cs` / `resume_ip` (far-call return frame).
    /// 4. Set `CS:IP` to [`option_rom_entry_cs_ip`].
    ///
    /// Gaps vs SeaBIOS (documented, not implemented here):
    /// - No `AX`/`BX`/`DX` PCI location / BDF convention for the call.
    /// - No PnP expansion header (`0x1A`), BEV/BCV, or runtime-size shrink.
    /// - No claim that SeaVGABIOS completes or installs fonts/INT 10h.
    ///
    /// Region discovery is [`Self::scan_option_rom_region`].
    pub fn invoke_option_rom_entry(
        &mut self,
        phys_base: u64,
        resume_cs: u16,
        resume_ip: u16,
    ) -> Result<(), MachineError> {
        let (entry_cs, entry_ip) =
            option_rom_entry_cs_ip(phys_base).ok_or(MachineError::OptionRomEntryInvalid)?;

        let image = self.read_mapped_option_rom(phys_base)?;
        let _validated = prepare_option_rom(phys_base, &image)?;

        self.ensure_option_rom_invoke_stack();
        self.push_real_mode_far_return(resume_cs, resume_ip)?;

        self.cpu.cs = SegmentReg::real_mode_code(entry_cs);
        self.cpu.set_ip16(entry_ip);
        self.cpu.halted = false;
        Ok(())
    }

    fn try_read_option_rom_hit(&self, phys_base: u64) -> Option<OptionRomScanHit> {
        let b0 = self.mem.read_u8(phys_base).ok()?;
        let b1 = self.mem.read_u8(phys_base + 1).ok()?;
        if b0 != OPTION_ROM_SIGNATURE[0] || b1 != OPTION_ROM_SIGNATURE[1] {
            return None;
        }
        let image = self.read_mapped_option_rom(phys_base).ok()?;
        prepare_option_rom(phys_base, &image).ok()?;
        let blocks = image.get(2).copied().unwrap_or(0);
        Some(OptionRomScanHit { phys_base, blocks })
    }

    /// Read the declared initialization extent from guest physical memory.
    fn read_mapped_option_rom(&self, phys_base: u64) -> Result<Vec<u8>, MachineError> {
        let mut header = [0u8; OPTION_ROM_HEADER_LEN];
        for (i, slot) in header.iter_mut().enumerate() {
            *slot = self
                .mem
                .read_u8(phys_base + i as u64)
                .map_err(|_| MachineError::OptionRomNotMapped)?;
        }
        let blocks = usize::from(header[2]);
        if blocks == 0 {
            return Err(MachineError::OptionRom(
                firmware_interface::OptionRomError::ZeroSize,
            ));
        }
        let len = blocks
            .checked_mul(OPTION_ROM_BLOCK_SIZE)
            .ok_or(MachineError::OptionRomNotMapped)?;
        let mut image = vec![0u8; len];
        for (i, slot) in image.iter_mut().enumerate() {
            *slot = self
                .mem
                .read_u8(phys_base + i as u64)
                .map_err(|_| MachineError::OptionRomNotMapped)?;
        }
        Ok(image)
    }

    fn ensure_option_rom_invoke_stack(&mut self) {
        let sp = self.cpu.gpr_u16(CpuState::RSP);
        if sp < 4 {
            self.cpu.ss = SegmentReg::real_mode(0x0000);
            self.cpu
                .set_gpr_u16(CpuState::RSP, OPTION_ROM_INVOKE_DEFAULT_SP);
        }
    }

    fn push_real_mode_far_return(
        &mut self,
        resume_cs: u16,
        resume_ip: u16,
    ) -> Result<(), MachineError> {
        // Far CALL pushes CS then IP so RETF sees [SP]=IP, [SP+2]=CS
        // (Intel SDM Vol. 2 CALL / RET — real-mode intersegment).
        let sp_cs = self.cpu.gpr_u16(CpuState::RSP).wrapping_sub(2);
        let sp_ip = sp_cs.wrapping_sub(2);
        let ss_base = self.cpu.ss.base;
        self.mem
            .write_u8(ss_base + u64::from(sp_cs), (resume_cs & 0xFF) as u8)
            .map_err(|_| MachineError::OptionRomStackFault)?;
        self.mem
            .write_u8(ss_base + u64::from(sp_cs) + 1, (resume_cs >> 8) as u8)
            .map_err(|_| MachineError::OptionRomStackFault)?;
        self.mem
            .write_u8(ss_base + u64::from(sp_ip), (resume_ip & 0xFF) as u8)
            .map_err(|_| MachineError::OptionRomStackFault)?;
        self.mem
            .write_u8(ss_base + u64::from(sp_ip) + 1, (resume_ip >> 8) as u8)
            .map_err(|_| MachineError::OptionRomStackFault)?;
        self.cpu.set_gpr_u16(CpuState::RSP, sp_ip);
        Ok(())
    }

    fn plant_option_rom_resume_hlt(&mut self) -> Result<(), MachineError> {
        self.mem
            .write_u8(OPTION_ROM_RESUME_PHYS, 0xF4)
            .map_err(|_| MachineError::OptionRomStackFault)?;
        Ok(())
    }
}

/// Re-export for callers that need the entry offset constant beside Machine.
pub use firmware_interface::OPTION_ROM_ENTRY_OFFSET as OPTION_ROM_INVOKE_ENTRY_OFFSET;

#[cfg(test)]
mod tests {
    use super::*;
    use firmware_interface::{OPTION_ROM_BLOCK_SIZE as BLOCK, OPTION_ROM_SIGNATURE};

    fn synthetic_option_rom(blocks: u8) -> Vec<u8> {
        let mut rom = vec![0u8; usize::from(blocks) * BLOCK];
        rom[0] = OPTION_ROM_SIGNATURE[0];
        rom[1] = OPTION_ROM_SIGNATURE[1];
        rom[2] = blocks;
        rom[3] = 0xCB; // RETF
        let sum = rom.iter().fold(0u8, |acc, b| acc.wrapping_add(*b));
        let last = rom.len() - 1;
        rom[last] = rom[last].wrapping_sub(sum);
        rom
    }

    /// Spec: BIOS Boot Spec — `C000:0003` entry; far CALL return frame; RETF.
    #[test]
    fn invoke_vga_option_rom_retf_reaches_resume_hlt() {
        let rom = synthetic_option_rom(2);
        let mut m = Machine::new(1024 * 1024);
        m.map_and_invoke_vga_option_rom(&rom)
            .expect("map+invoke VGA option ROM");

        assert_eq!(m.cpu.cs.selector, 0xC000);
        assert_eq!(
            m.cpu.ip16(),
            firmware_interface::OPTION_ROM_ENTRY_OFFSET as u16
        );
        assert_eq!(m.mem.read_u8(0x000C_0003).unwrap(), 0xCB);

        m.step().expect("RETF");
        assert_eq!(m.cpu.cs.selector, 0x0000);
        assert_eq!(m.cpu.ip16(), OPTION_ROM_RESUME_PHYS as u16);
        assert_eq!(m.mem.read_u8(OPTION_ROM_RESUME_PHYS).unwrap(), 0xF4);

        m.step().expect("HLT");
        assert!(m.cpu.halted);
    }

    #[test]
    fn invoke_rejects_unmapped_base() {
        let mut m = Machine::new(1024 * 1024);
        let err = m
            .invoke_option_rom_entry(VGA_OPTION_ROM_BASE, 0, 0x0500)
            .expect_err("nothing mapped");
        assert!(matches!(
            err,
            MachineError::OptionRomNotMapped | MachineError::OptionRom(_)
        ));
    }

    #[test]
    fn map_and_invoke_custom_resume() {
        let rom = synthetic_option_rom(1);
        let mut m = Machine::new(1024 * 1024);
        // Resume at 0000:0600 with NOP;ROM RETF should land there.
        m.mem.write_u8(0x0600, 0x90).unwrap();
        m.map_and_invoke_option_rom(VGA_OPTION_ROM_BASE, &rom, 0x0000, 0x0600)
            .expect("invoke");
        m.step().expect("RETF");
        assert_eq!(m.cpu.ip16(), 0x0600);
        assert_eq!(m.mem.read_u8(0x0600).unwrap(), 0x90);
    }

    /// Spec: BIOS Boot Spec — scan `C0000`–`DFFFF` on 2 KiB; invoke each entry.
    #[test]
    fn post_scan_finds_and_invokes_vga_and_second_rom() {
        let vga = synthetic_option_rom(2); // 1 KiB → advances 2 KiB
        let second = synthetic_option_rom(2);
        let mut m = Machine::new(1024 * 1024);
        m.map_option_rom(VGA_OPTION_ROM_BASE, &vga)
            .expect("map VGA");
        m.map_option_rom(0x000C_0800, &second).expect("map second");

        let hits = m.scan_option_rom_region();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].phys_base, VGA_OPTION_ROM_BASE);
        assert_eq!(hits[1].phys_base, 0x000C_0800);

        let n = m
            .post_scan_invoke_option_roms_default(16)
            .expect("POST scan invoke");
        assert_eq!(n, 2);
        assert_eq!(m.cpu.cs.selector, 0x0000);
        assert_eq!(m.cpu.ip16(), OPTION_ROM_RESUME_PHYS as u16);
    }

    #[test]
    fn post_scan_skips_bad_signature_holes() {
        let rom = synthetic_option_rom(4); // 2 KiB
        let mut m = Machine::new(1024 * 1024);
        m.map_option_rom(0x000C_1000, &rom).expect("map");
        let hits = m.scan_option_rom_region();
        assert_eq!(
            hits,
            [OptionRomScanHit {
                phys_base: 0x000C_1000,
                blocks: 4,
            }]
        );
        assert_eq!(hits[0].next_scan_base(), 0x000C_1800);
    }

    #[test]
    fn empty_region_scan_returns_zero() {
        let m = Machine::new(1024 * 1024);
        assert!(m.scan_option_rom_region().is_empty());
        let mut m = Machine::new(1024 * 1024);
        assert_eq!(m.post_scan_invoke_option_roms_default(4).unwrap(), 0);
    }

    /// Spec: BIOS Boot Spec — checksum must be zero mod 256; bad sum is skipped.
    #[test]
    fn post_scan_skips_bad_checksum() {
        let mut bad = synthetic_option_rom(2);
        let last = bad.len() - 1;
        bad[last] = bad[last].wrapping_add(1); // break checksum
        let mut m = Machine::new(1024 * 1024);
        // Bypass prepare_option_rom by writing raw bytes into the window.
        for (i, b) in bad.iter().enumerate() {
            m.mem.write_u8(VGA_OPTION_ROM_BASE + i as u64, *b).unwrap();
        }
        assert!(
            m.scan_option_rom_region().is_empty(),
            "bad checksum must not be a scan hit"
        );
    }

    /// Spec: BIOS Boot Spec — far-call path re-validates checksum before entry.
    #[test]
    fn invoke_rejects_bad_checksum_image() {
        let mut bad = synthetic_option_rom(2);
        let last = bad.len() - 1;
        bad[last] = bad[last].wrapping_add(1);
        let mut m = Machine::new(1024 * 1024);
        // Leave the region as RAM (no ROM map) so host writes are visible to
        // `read_mapped_option_rom`; ROM windows ignore guest writes.
        for (i, b) in bad.iter().enumerate() {
            m.mem.write_u8(VGA_OPTION_ROM_BASE + i as u64, *b).unwrap();
        }
        let err = m
            .invoke_option_rom_entry(VGA_OPTION_ROM_BASE, 0, 0x0500)
            .expect_err("corrupt checksum must fail before far-call");
        assert!(matches!(
            err,
            MachineError::OptionRom(firmware_interface::OptionRomError::BadChecksum)
                | MachineError::OptionRom(_)
        ));
    }

    /// Honesty: synthetic RETF option ROM is not SeaVGABIOS — BDA video + font
    /// stay whatever the host INT 10h stub / bring-up font path set.
    #[test]
    fn option_rom_retf_preserves_bda_and_font() {
        use crate::int10::{
            setup_int10_set_cursor, setup_int10_set_mode, BDA_ACTIVE_PAGE, BDA_CURSOR_PAGE0,
            BDA_VIDEO_COLS, BDA_VIDEO_MODE, BDA_VIDEO_PAGE_SIZE, INT10_MODE03_PAGE_SIZE,
            INT10_MODE_03H_TEXT,
        };

        let rom = synthetic_option_rom(1);
        let mut m = Machine::new(1024 * 1024);
        setup_int10_set_mode(&mut m.cpu, INT10_MODE_03H_TEXT);
        m.service_int10();
        setup_int10_set_cursor(&mut m.cpu, 0, 2, 9);
        m.service_int10();
        assert!(!m.vga.text_font_installed());

        m.map_and_invoke_vga_option_rom(&rom)
            .expect("map+invoke RETF ROM");
        m.step().expect("RETF");

        assert_eq!(m.mem.read_u8(BDA_VIDEO_MODE).unwrap(), INT10_MODE_03H_TEXT);
        assert_eq!(m.mem.read_u8(BDA_VIDEO_COLS).unwrap(), 80);
        assert_eq!(
            u16::from(m.mem.read_u8(BDA_VIDEO_PAGE_SIZE).unwrap())
                | (u16::from(m.mem.read_u8(BDA_VIDEO_PAGE_SIZE + 1).unwrap()) << 8),
            INT10_MODE03_PAGE_SIZE
        );
        assert_eq!(m.mem.read_u8(BDA_CURSOR_PAGE0).unwrap(), 9);
        assert_eq!(m.mem.read_u8(BDA_CURSOR_PAGE0 + 1).unwrap(), 2);
        assert_eq!(m.mem.read_u8(BDA_ACTIVE_PAGE).unwrap(), 0);
        assert!(!m.vga.text_font_installed());
    }
}
