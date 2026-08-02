//! Firmware interface stubs (fw_cfg / ACPI arrive later).

#![forbid(unsafe_code)]

/// How a ROM image should be placed in physical memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RomImage {
    pub phys_base: u64,
    pub data: Vec<u8>,
}

impl RomImage {
    pub fn new(phys_base: u64, data: Vec<u8>) -> Self {
        Self { phys_base, data }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rom_image_stores_bytes() {
        let r = RomImage::new(0xFFFF_0000, vec![0xF4]);
        assert_eq!(r.data, vec![0xF4]);
    }
}
