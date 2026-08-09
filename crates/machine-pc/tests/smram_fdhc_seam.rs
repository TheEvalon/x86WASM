//! PhysMem / MachineBus seam for i440FX SMRAM (`0x72`) and FDHC (`0x68`).
//!
//! Spec: Intel 440FX 82441FX (PMC) §3.2.23 Table 4 / §3.2.20. Config writes that
//! overlap those registers must re-attribute memory the same way PAM does.

use devices::{
    PciConfig, PortDevice, PCI_CONFIG_ADDRESS, PCI_CONFIG_DATA, PCI_PMC_FDHC_HEN_512K_640K,
    PCI_PMC_FDHC_OFFSET, PCI_PMC_SMRAM_D_OPEN, PCI_PMC_SMRAM_G_SMRAME, PCI_PMC_SMRAM_OFFSET,
};
use machine_pc::Machine;

fn write_host_bridge_byte(m: &mut Machine, offset: u8, value: u8) {
    let addr = PciConfig::make_address(0, 0, 0, offset & 0xFC, true);
    let lane = offset & 3;
    let data_port = PCI_CONFIG_DATA + u16::from(lane);
    m.pci.port_write(PCI_CONFIG_ADDRESS, 4, addr);
    // Detect overlap after the address latch (same order as MachineBus).
    let smram_touch = m.pci.smram_config_write_overlaps(data_port, 1);
    let fdhc_touch = m.pci.fdhc_config_write_overlaps(data_port, 1);
    m.pci.port_write(data_port, 1, u32::from(value));
    if smram_touch {
        m.sync_smram_to_memory(false);
    }
    if fdhc_touch {
        m.sync_fdhc_to_memory();
    }
}

#[test]
fn smram_config_write_steers_a0000_to_dram() {
    let mut m = Machine::new(2 * 1024 * 1024);
    // Enable global SMRAM + D_OPEN outside SMM → data/code to DRAM (Table 4).
    let smram = PCI_PMC_SMRAM_G_SMRAME | PCI_PMC_SMRAM_D_OPEN | 0x02;
    write_host_bridge_byte(&mut m, PCI_PMC_SMRAM_OFFSET, smram);
    assert_eq!(m.pci.smram_register(), smram);
    assert!(m.mem.smram_steers_write_to_dram(0xA0000));

    assert!(m.mem.write_u8(0xA0000, 0xA5).is_ok());
    assert_eq!(m.mem.read_u8(0xA0000).unwrap(), 0xA5);
}

#[test]
fn fdhc_config_write_opens_512k_hole() {
    let mut m = Machine::new(2 * 1024 * 1024);
    assert!(m.mem.write_u8(0x80000, 0x11).is_ok());
    assert_eq!(m.mem.read_u8(0x80000).unwrap(), 0x11);

    write_host_bridge_byte(&mut m, PCI_PMC_FDHC_OFFSET, PCI_PMC_FDHC_HEN_512K_640K << 6);
    assert!(m.pci.fdhc_hole().is_some());

    // Hole → open bus (read 0xFF, write dropped).
    assert_eq!(m.mem.read_u8(0x80000).unwrap(), 0xFF);
    let _ = m.mem.write_u8(0x80000, 0x22);

    // Underlying DRAM byte unchanged after the hole write.
    m.mem.apply_fdhc_hole(None);
    assert_eq!(m.mem.read_u8(0x80000).unwrap(), 0x11);
}

#[test]
fn pci_accessor_defaults_match_reset() {
    let pci = PciConfig::new();
    assert_eq!(pci.smram_register(), 0x02);
    assert!(pci.fdhc_hole().is_none());
    let region = pci.smram_region(false);
    assert!(!region.g_smrame);
    assert!(!region.data_to_dram);
}
