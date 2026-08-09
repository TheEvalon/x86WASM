//! i440FX PMC FDHC register (`00:00.0` config `0x68`) decode + host accessor.
//!
//! Spec: Intel 440FX PCIset 82441FX (PMC) datasheet §3.2.20 "FDHC — Fixed DRAM
//! Hole Control Register" — default `00h`, HEN bits [7:6] select none /
//! 512 KB–640 KB (`080000h`–`09FFFFh`) / 15 MB–16 MB / reserved.

use devices::{
    PciConfig, PortDevice, PCI_CONFIG_ADDRESS, PCI_CONFIG_DATA, PCI_PMC_FDHC_15M_END,
    PCI_PMC_FDHC_15M_START, PCI_PMC_FDHC_512K_END, PCI_PMC_FDHC_512K_START, PCI_PMC_FDHC_DEFAULT,
    PCI_PMC_FDHC_HEN_15M_16M, PCI_PMC_FDHC_HEN_512K_640K, PCI_PMC_FDHC_HEN_NONE,
    PCI_PMC_FDHC_HEN_RESERVED, PCI_PMC_FDHC_OFFSET, PCI_PMC_FDHC_WRITABLE_MASK,
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
fn fdhc_defaults_to_no_hole_at_reset() {
    let mut pci = PciConfig::new();
    assert_eq!(
        cfg_read_u8(&mut pci, PCI_PMC_FDHC_OFFSET),
        PCI_PMC_FDHC_DEFAULT
    );
    assert_eq!(pci.fdhc_hen(), PCI_PMC_FDHC_HEN_NONE);
    assert!(pci.fdhc_hole().is_none());
}

#[test]
fn fdhc_reserved_bits_read_zero() {
    let mut pci = PciConfig::new();
    cfg_write_u8(&mut pci, PCI_PMC_FDHC_OFFSET, 0xFF);
    assert_eq!(
        cfg_read_u8(&mut pci, PCI_PMC_FDHC_OFFSET),
        PCI_PMC_FDHC_WRITABLE_MASK
    );
    assert_eq!(pci.fdhc_hen(), PCI_PMC_FDHC_HEN_RESERVED);
    assert!(pci.fdhc_hole().is_none());
}

#[test]
fn fdhc_hen_512k_640k_decodes_080000_09ffff() {
    let mut pci = PciConfig::new();
    cfg_write_u8(
        &mut pci,
        PCI_PMC_FDHC_OFFSET,
        PCI_PMC_FDHC_HEN_512K_640K << 6,
    );
    let hole = pci.fdhc_hole().expect("512K–640K hole");
    assert_eq!(hole.start, PCI_PMC_FDHC_512K_START);
    assert_eq!(hole.end, PCI_PMC_FDHC_512K_END);
    assert_eq!(hole.hen, PCI_PMC_FDHC_HEN_512K_640K);
    assert!(hole.contains(0x0008_0000));
    assert!(hole.contains(0x0009_FFFF));
    assert!(!hole.contains(0x0007_FFFF));
    assert!(!hole.contains(0x000A_0000));
}

#[test]
fn fdhc_hen_15m_16m_decodes_f00000_ffffff() {
    let mut pci = PciConfig::new();
    cfg_write_u8(&mut pci, PCI_PMC_FDHC_OFFSET, PCI_PMC_FDHC_HEN_15M_16M << 6);
    let hole = pci.fdhc_hole().expect("15M–16M hole");
    assert_eq!(hole.start, PCI_PMC_FDHC_15M_START);
    assert_eq!(hole.end, PCI_PMC_FDHC_15M_END);
    assert_eq!(hole.len(), 1024 * 1024);
}

#[test]
fn fdhc_clears_on_reset() {
    let mut pci = PciConfig::new();
    cfg_write_u8(
        &mut pci,
        PCI_PMC_FDHC_OFFSET,
        PCI_PMC_FDHC_HEN_512K_640K << 6,
    );
    pci.reset();
    assert_eq!(pci.fdhc_register(), PCI_PMC_FDHC_DEFAULT);
    assert!(pci.fdhc_hole().is_none());
}

#[test]
fn fdhc_config_write_overlaps_only_host_bridge_offset_68() {
    let mut pci = PciConfig::new();
    let addr = PciConfig::make_address(0, 0, 0, PCI_PMC_FDHC_OFFSET, true);
    pci.port_write(PCI_CONFIG_ADDRESS, 4, addr);
    assert!(
        pci.fdhc_config_write_overlaps(PCI_CONFIG_DATA + u16::from(PCI_PMC_FDHC_OFFSET & 0x03), 1)
    );
    let piix = PciConfig::make_address(0, 1, 0, PCI_PMC_FDHC_OFFSET, true);
    pci.port_write(PCI_CONFIG_ADDRESS, 4, piix);
    assert!(
        !pci.fdhc_config_write_overlaps(PCI_CONFIG_DATA + u16::from(PCI_PMC_FDHC_OFFSET & 0x03), 1)
    );
}
