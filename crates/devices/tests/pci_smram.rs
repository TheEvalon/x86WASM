//! i440FX PMC SMRAM register (`00:00.0` config `0x72`) decode + host accessor.
//!
//! Spec: Intel 440FX PCIset 82441FX (PMC) datasheet §3.2.23 "SMRAM — System
//! Management RAM Control Register" (default `02h`) and Table 4 "SMRAM Space
//! Cycles". Bit names use the modern D_OPEN / D_CLS / D_LCK / G_SMRAME /
//! C_BASE_SEG spellings for the datasheet's DOPEN / DCLS / DLCK / SMRAME /
//! DBASESEG fields.

use devices::{
    PciConfig, PortDevice, PCI_CONFIG_ADDRESS, PCI_CONFIG_DATA, PCI_PMC_SMRAM_COMPAT_END,
    PCI_PMC_SMRAM_COMPAT_START, PCI_PMC_SMRAM_C_BASE_SEG_A0000, PCI_PMC_SMRAM_DEFAULT,
    PCI_PMC_SMRAM_D_CLS, PCI_PMC_SMRAM_D_LCK, PCI_PMC_SMRAM_D_OPEN, PCI_PMC_SMRAM_G_SMRAME,
    PCI_PMC_SMRAM_OFFSET, PCI_PMC_SMRAM_WRITABLE_MASK,
};

fn cfg_read_u8(pci: &mut PciConfig, offset: u8) -> u8 {
    let addr = PciConfig::make_address(0, 0, 0, offset, true);
    pci.port_write(PCI_CONFIG_ADDRESS, 4, addr);
    pci.port_read(PCI_CONFIG_DATA + u16::from(offset & 0x03), 1) as u8
}

fn cfg_write_u8(pci: &mut PciConfig, offset: u8, value: u8) {
    let addr = PciConfig::make_address(0, 0, 0, offset, true);
    pci.port_write(PCI_CONFIG_ADDRESS, 4, addr);
    pci.port_write(
        PCI_CONFIG_DATA + u16::from(offset & 0x03),
        1,
        u32::from(value),
    );
}

#[test]
fn smram_defaults_to_c_base_seg_a0000_at_reset() {
    let mut pci = PciConfig::new();
    assert_eq!(
        cfg_read_u8(&mut pci, PCI_PMC_SMRAM_OFFSET),
        PCI_PMC_SMRAM_DEFAULT
    );
    assert_eq!(pci.smram_register(), PCI_PMC_SMRAM_DEFAULT);
    let region = pci.smram_region(false);
    assert_eq!(region.c_base_seg, PCI_PMC_SMRAM_C_BASE_SEG_A0000);
    assert_eq!(region.start, PCI_PMC_SMRAM_COMPAT_START);
    assert_eq!(region.end, PCI_PMC_SMRAM_COMPAT_END);
    assert!(!region.g_smrame);
    assert!(!region.code_to_dram);
    assert!(!region.data_to_dram);
}

#[test]
fn smram_reserved_bit7_reads_zero() {
    let mut pci = PciConfig::new();
    cfg_write_u8(&mut pci, PCI_PMC_SMRAM_OFFSET, 0xFF);
    assert_eq!(
        cfg_read_u8(&mut pci, PCI_PMC_SMRAM_OFFSET),
        PCI_PMC_SMRAM_WRITABLE_MASK & !PCI_PMC_SMRAM_D_OPEN | PCI_PMC_SMRAM_D_LCK
    );
    // Writing all ones with D_LCK set forces D_OPEN clear.
    assert_eq!(pci.smram_register() & PCI_PMC_SMRAM_D_OPEN, 0);
    assert_eq!(
        pci.smram_register() & PCI_PMC_SMRAM_D_LCK,
        PCI_PMC_SMRAM_D_LCK
    );
}

#[test]
fn smram_open_enables_dram_outside_smm_until_locked() {
    let mut pci = PciConfig::new();
    // G_SMRAME | D_OPEN | C_BASE_SEG=010
    cfg_write_u8(
        &mut pci,
        PCI_PMC_SMRAM_OFFSET,
        PCI_PMC_SMRAM_G_SMRAME | PCI_PMC_SMRAM_D_OPEN | PCI_PMC_SMRAM_C_BASE_SEG_A0000,
    );
    let open = pci.smram_region(false);
    assert!(open.g_smrame);
    assert!(open.d_open);
    assert!(open.code_to_dram);
    assert!(open.data_to_dram);

    // Lock: D_OPEN clears and sticks; outside SMM maps to PCI again.
    cfg_write_u8(
        &mut pci,
        PCI_PMC_SMRAM_OFFSET,
        PCI_PMC_SMRAM_G_SMRAME
            | PCI_PMC_SMRAM_D_OPEN
            | PCI_PMC_SMRAM_D_LCK
            | PCI_PMC_SMRAM_C_BASE_SEG_A0000,
    );
    assert_eq!(pci.smram_register() & PCI_PMC_SMRAM_D_OPEN, 0);
    assert!(pci.smram_region(false).d_lck);
    assert!(!pci.smram_region(false).code_to_dram);
    assert!(pci.smram_region(true).code_to_dram);
    assert!(pci.smram_region(true).data_to_dram);

    // Further attempts to clear D_LCK or set D_OPEN are ignored.
    cfg_write_u8(
        &mut pci,
        PCI_PMC_SMRAM_OFFSET,
        PCI_PMC_SMRAM_G_SMRAME | PCI_PMC_SMRAM_D_OPEN | PCI_PMC_SMRAM_C_BASE_SEG_A0000,
    );
    assert_eq!(
        pci.smram_register() & PCI_PMC_SMRAM_D_LCK,
        PCI_PMC_SMRAM_D_LCK
    );
    assert_eq!(pci.smram_register() & PCI_PMC_SMRAM_D_OPEN, 0);
}

#[test]
fn smram_d_cls_closes_data_but_allows_code_in_smm() {
    let mut pci = PciConfig::new();
    cfg_write_u8(
        &mut pci,
        PCI_PMC_SMRAM_OFFSET,
        PCI_PMC_SMRAM_G_SMRAME | PCI_PMC_SMRAM_D_CLS | PCI_PMC_SMRAM_C_BASE_SEG_A0000,
    );
    let smm = pci.smram_region(true);
    assert!(smm.code_to_dram);
    assert!(!smm.data_to_dram);
    let nonsmm = pci.smram_region(false);
    assert!(!nonsmm.code_to_dram);
    assert!(!nonsmm.data_to_dram);
}

#[test]
fn smram_clears_on_device_reset_including_d_lck() {
    let mut pci = PciConfig::new();
    cfg_write_u8(
        &mut pci,
        PCI_PMC_SMRAM_OFFSET,
        PCI_PMC_SMRAM_G_SMRAME | PCI_PMC_SMRAM_D_LCK | PCI_PMC_SMRAM_C_BASE_SEG_A0000,
    );
    assert!(pci.smram_region(false).d_lck);
    pci.reset();
    assert_eq!(pci.smram_register(), PCI_PMC_SMRAM_DEFAULT);
    assert!(!pci.smram_region(false).d_lck);
}

#[test]
fn smram_config_write_overlaps_only_host_bridge_offset_72() {
    let mut pci = PciConfig::new();
    let addr = PciConfig::make_address(0, 0, 0, PCI_PMC_SMRAM_OFFSET, true);
    pci.port_write(PCI_CONFIG_ADDRESS, 4, addr);
    assert!(pci
        .smram_config_write_overlaps(PCI_CONFIG_DATA + u16::from(PCI_PMC_SMRAM_OFFSET & 0x03), 1));
    let piix = PciConfig::make_address(0, 1, 0, PCI_PMC_SMRAM_OFFSET, true);
    pci.port_write(PCI_CONFIG_ADDRESS, 4, piix);
    assert!(!pci
        .smram_config_write_overlaps(PCI_CONFIG_DATA + u16::from(PCI_PMC_SMRAM_OFFSET & 0x03), 1));
}
