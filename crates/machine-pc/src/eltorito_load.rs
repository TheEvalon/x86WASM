//! El Torito no-emulation boot image → guest RAM handoff.
//!
//! Spec: Phoenix/IBM "El Torito" Bootable CD-ROM Format Specification 1.0 —
//! after catalog validation, copy `sector_count × 512` bytes from `load_rba`
//! and set `CS:IP = load_segment:0000` (default phys `0x7C00`).

use crate::{Machine, MachineError};

impl Machine {
    /// Load the El Torito no-emulation boot image into guest RAM and set
    /// `CS:IP = load_segment:0000` (default phys `0x7C00`).
    ///
    /// Spec: El Torito 1.0 — validate catalog via [`Self::inspect_atapi_el_torito`],
    /// require media type `00h`, copy bytes from `load_rba`, far-jump to the load
    /// segment. Floppy/HDD emulation remain out of scope. Guest-callable host
    /// INT 13h CD reads use [`Self::service_int13_cd`] (`DL=E0h`), not this helper.
    pub fn load_eltorito_to_7c00(&mut self) -> Result<(), MachineError> {
        let info = self.inspect_atapi_el_torito()?;
        if !info.bootable {
            return Err(MachineError::ElToritoNotBootable);
        }
        if info.media_type != firmware_interface::EL_TORITO_MEDIA_NO_EMUL {
            return Err(MachineError::ElToritoUnsupportedMedia);
        }
        let byte_len = info
            .load_byte_len()
            .filter(|n| *n > 0)
            .ok_or(MachineError::ElToritoInvalidSectorCount)?;
        let phys = info.load_phys();
        let image = self
            .ide
            .atapi_medium_image()
            .ok_or(firmware_interface::ElToritoError::Truncated)?;
        let src_off = (info.load_rba as usize)
            .checked_mul(firmware_interface::EL_TORITO_SECTOR_BYTES)
            .ok_or(MachineError::ElToritoBootImageOob)?;
        let src_end = src_off
            .checked_add(byte_len)
            .ok_or(MachineError::ElToritoBootImageOob)?;
        if src_end > image.len() {
            return Err(MachineError::ElToritoBootImageOob);
        }
        let need = (phys as usize)
            .checked_add(byte_len)
            .ok_or(MachineError::ElToritoRamTooSmall)?;
        if self.mem.ram_len() < need {
            return Err(MachineError::ElToritoRamTooSmall);
        }
        let boot = image[src_off..src_end].to_vec();
        for (i, byte) in boot.iter().enumerate() {
            self.mem
                .write_u8(phys + i as u64, *byte)
                .map_err(|_| MachineError::ElToritoRamTooSmall)?;
        }
        let seg = info.effective_load_segment();
        self.cpu.cs = x86_core::SegmentReg::real_mode_code(seg);
        self.cpu.set_ip16(0);
        Ok(())
    }
}
