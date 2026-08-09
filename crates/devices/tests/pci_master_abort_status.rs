//! Host-bridge Received Master Abort on Mechanism #1 absent-target cycles.
//!
//! Spec: PCI Local Bus Specification Revision 3.0 §3.2.2.3.4 / footnote 15
//! (absent target → all ones / dropped write) and §6.2.3 Status Register
//! "Received Master Abort" (RW1C) — the host bridge is the initiator of
//! CONFIG_DATA cycles.

use devices::{
    PciConfig, PortDevice, PCI_CONFIG_ADDRESS, PCI_CONFIG_DATA, PCI_HOST_BRIDGE_STATUS_STUB,
    PCI_STATUS_OFFSET, PCI_STATUS_REC_MASTER_ABORT,
};

fn host_status(pci: &mut PciConfig) -> u16 {
    let addr = PciConfig::make_address(0, 0, 0, PCI_STATUS_OFFSET, true);
    pci.port_write(PCI_CONFIG_ADDRESS, 4, addr);
    pci.port_read(PCI_CONFIG_DATA + 2, 2) as u16
}

#[test]
fn absent_device_config_read_sets_received_master_abort() {
    let mut pci = PciConfig::new();
    assert_eq!(host_status(&mut pci), PCI_HOST_BRIDGE_STATUS_STUB);

    // 00:1F.0 is absent on this i440FX stub.
    let addr = PciConfig::make_address(0, 0x1F, 0, 0, true);
    pci.port_write(PCI_CONFIG_ADDRESS, 4, addr);
    assert_eq!(pci.port_read(PCI_CONFIG_DATA, 4), 0xFFFF_FFFF);
    assert_eq!(
        host_status(&mut pci),
        PCI_HOST_BRIDGE_STATUS_STUB | PCI_STATUS_REC_MASTER_ABORT
    );
}

#[test]
fn absent_device_config_write_sets_received_master_abort() {
    let mut pci = PciConfig::new();
    let addr = PciConfig::make_address(0, 2, 0, 0, true);
    pci.port_write(PCI_CONFIG_ADDRESS, 4, addr);
    pci.port_write(PCI_CONFIG_DATA, 4, 0x1234_5678);
    assert_eq!(
        host_status(&mut pci),
        PCI_HOST_BRIDGE_STATUS_STUB | PCI_STATUS_REC_MASTER_ABORT
    );
}

#[test]
fn received_master_abort_is_rw1c_and_enable_clear_is_not_abort() {
    let mut pci = PciConfig::new();
    let addr = PciConfig::make_address(0, 3, 0, 0, true);
    pci.port_write(PCI_CONFIG_ADDRESS, 4, addr);
    let _ = pci.port_read(PCI_CONFIG_DATA, 4);
    assert!(host_status(&mut pci) & PCI_STATUS_REC_MASTER_ABORT != 0);

    // RW1C clear.
    let st = PciConfig::make_address(0, 0, 0, PCI_STATUS_OFFSET, true);
    pci.port_write(PCI_CONFIG_ADDRESS, 4, st);
    pci.port_write(
        PCI_CONFIG_DATA + 2,
        2,
        u32::from(PCI_STATUS_REC_MASTER_ABORT),
    );
    assert_eq!(host_status(&mut pci), PCI_HOST_BRIDGE_STATUS_STUB);

    // Enable bit clear → ordinary open-bus I/O, not a config Master-Abort.
    let disabled = PciConfig::make_address(0, 3, 0, 0, false);
    pci.port_write(PCI_CONFIG_ADDRESS, 4, disabled);
    assert_eq!(pci.port_read(PCI_CONFIG_DATA, 4), 0xFFFF_FFFF);
    assert_eq!(host_status(&mut pci), PCI_HOST_BRIDGE_STATUS_STUB);
}
