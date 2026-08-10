//! Host-driven option-ROM initialization entry (map + far-call `base+3`).
//!
//! Isolated from MachineBus / port wiring so parallel device slices can merge
//! without fighting this file.
//!
//! Spec: PCI Firmware Specification / BIOS Boot Specification — PC-compatible
//! expansion ROM header (`55 AA`, size, checksum); initialization entry at
//! offset 3; classic BIOS far-calls `CS:IP = (base>>4):0003` and expects `RETF`.

use crate::{Machine, MachineError};
use firmware_interface::{
    option_rom_entry_cs_ip, prepare_option_rom, OPTION_ROM_BLOCK_SIZE, OPTION_ROM_HEADER_LEN,
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

impl Machine {
    /// Map a validated expansion ROM, then far-call its entry at offset 3.
    ///
    /// Combines [`Self::map_option_rom`] with [`Self::invoke_option_rom_entry`].
    /// `resume_cs` / `resume_ip` become the far-return target pushed for `RETF`.
    ///
    /// **Not** a SeaBIOS option-ROM scan, PnP BEV/BCV dispatch, or INT 10h.
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
    /// - No `0xC0000`–`0xDFFFF` scan loop or 2 KiB step discovery.
    /// - No `AX`/`BX`/`DX` PCI location / BDF convention for the call.
    /// - No PnP expansion header (`0x1A`), BEV/BCV, or runtime-size shrink.
    /// - No claim that SeaVGABIOS completes or installs fonts/INT 10h.
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
            return Err(MachineError::OptionRom(firmware_interface::OptionRomError::ZeroSize));
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
            self.cpu.set_gpr_u16(CpuState::RSP, OPTION_ROM_INVOKE_DEFAULT_SP);
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
    use firmware_interface::{OPTION_ROM_SIGNATURE, OPTION_ROM_BLOCK_SIZE as BLOCK};

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
}
