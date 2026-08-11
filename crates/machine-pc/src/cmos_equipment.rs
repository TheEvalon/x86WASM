//! CMOS equipment / floppy-type coherence with BDA equipment (R15 polish).
//!
//! Spec: RBIL CMOS `14h` Table C0019; CMOS `10h` Tables C0007/C0008;
//! RBIL MEM `0040:0010` equipment list (same low-byte layout as CMOS `14h`).

use crate::{guest_boot::BDA_EQUIPMENT, Machine, MachineError};
use devices::{CmosRtc, EQUIP_FLOPPY_INSTALLED, EQUIP_KEYBOARD_ENABLED};

impl Machine {
    /// Whether CMOS equipment / floppy-type bytes match this machine's model.
    pub fn cmos_equipment_coherent(&self) -> bool {
        let equip = self.cmos.equipment_byte();
        if equip != self.equipment_byte() {
            return false;
        }
        if equip & EQUIP_KEYBOARD_ENABLED == 0 {
            return false;
        }
        let drive_a = self.cmos.floppy_drive_type(0);
        let drive_b = self.cmos.floppy_drive_type(1);
        let floppy_bit = equip & EQUIP_FLOPPY_INSTALLED != 0;
        let type_present = drive_a != CmosRtc::FLOPPY_TYPE_NONE;
        if floppy_bit != type_present {
            return false;
        }
        if floppy_bit && drive_a != CmosRtc::FLOPPY_TYPE_1440K {
            return false;
        }
        drive_b == CmosRtc::FLOPPY_TYPE_NONE
    }

    /// Seed BDA `40:10` from the stored CMOS equipment byte (`14h`).
    pub fn seed_bda_equipment_from_cmos(&mut self) -> Result<(), MachineError> {
        let equip = self.cmos.equipment_byte();
        self.mem
            .write_u8(BDA_EQUIPMENT, equip)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        self.mem
            .write_u8(BDA_EQUIPMENT + 1, 0)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        Ok(())
    }

    /// BDA equipment low byte (`0040:0010`).
    pub fn bda_equipment_byte(&self) -> Result<u8, MachineError> {
        self.mem
            .read_u8(BDA_EQUIPMENT)
            .map_err(|_| MachineError::MbrRamTooSmall)
    }

    /// Host-visible snapshot of CMOS `14h` / drive types.
    pub fn cmos_equipment_and_floppy_types(&self) -> (u8, u8, u8) {
        (
            self.cmos.equipment_byte(),
            self.cmos.floppy_drive_type(0),
            self.cmos.floppy_drive_type(1),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use devices::{FDC_1440_IMAGE_SIZE, REG_EQUIPMENT, REG_FLOPPY_TYPE};

    #[test]
    fn new_machine_cmos_equipment_coherent_no_floppy() {
        let m = Machine::new(64 * 1024);
        assert!(m.cmos_equipment_coherent());
        let (equip, a, b) = m.cmos_equipment_and_floppy_types();
        assert_eq!(equip & EQUIP_KEYBOARD_ENABLED, EQUIP_KEYBOARD_ENABLED);
        assert_eq!(equip & EQUIP_FLOPPY_INSTALLED, 0);
        assert_eq!(a, CmosRtc::FLOPPY_TYPE_NONE);
        assert_eq!(b, CmosRtc::FLOPPY_TYPE_NONE);
        assert_eq!(m.cmos.read_reg(REG_EQUIPMENT), equip);
        assert_eq!(m.cmos.read_reg(REG_FLOPPY_TYPE), 0);
    }

    #[test]
    fn floppy_attach_keeps_keyboard_bit_and_matches_type() {
        let mut m = Machine::new(64 * 1024);
        m.attach_floppy_image(vec![0u8; FDC_1440_IMAGE_SIZE])
            .expect("1.44 image");
        assert!(m.cmos_equipment_coherent());
        let (equip, a, b) = m.cmos_equipment_and_floppy_types();
        assert_eq!(equip & EQUIP_KEYBOARD_ENABLED, EQUIP_KEYBOARD_ENABLED);
        assert_eq!(equip & EQUIP_FLOPPY_INSTALLED, EQUIP_FLOPPY_INSTALLED);
        assert_eq!(a, CmosRtc::FLOPPY_TYPE_1440K);
        assert_eq!(b, CmosRtc::FLOPPY_TYPE_NONE);
        assert_eq!(m.cmos.read_reg(REG_FLOPPY_TYPE), 0x40);
    }

    #[test]
    fn seed_bda_from_cmos_mirrors_equipment_byte() {
        let mut m = Machine::new(64 * 1024);
        m.attach_floppy_image(vec![0u8; FDC_1440_IMAGE_SIZE])
            .expect("1.44 image");
        m.seed_bda_equipment_from_cmos().unwrap();
        assert_eq!(m.bda_equipment_byte().unwrap(), m.cmos.equipment_byte());
        assert_eq!(m.bda_equipment_byte().unwrap(), m.equipment_byte());
        assert_ne!(m.bda_equipment_byte().unwrap() & EQUIP_KEYBOARD_ENABLED, 0);
    }

    #[test]
    fn coherence_detects_floppy_type_mismatch() {
        let mut m = Machine::new(64 * 1024);
        m.attach_floppy_image(vec![0u8; FDC_1440_IMAGE_SIZE])
            .expect("1.44 image");
        assert!(m.cmos_equipment_coherent());
        m.cmos
            .set_floppy_drive_types(CmosRtc::FLOPPY_TYPE_NONE, CmosRtc::FLOPPY_TYPE_NONE);
        assert!(!m.cmos_equipment_coherent());
    }
}
