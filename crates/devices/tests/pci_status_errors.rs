//! PCI Status RMA / STA honesty on config Master-Abort and RW1C.
//!
//! Spec: PCI Local Bus Specification Revision 3.0 §3.2.2.3.4 / footnote 15
//! (absent target → all ones) and §6.2.3 Status Register — Received Master
//! Abort (bit 13) on the initiator; Signaled Target Abort (bit 11) is a
//! *target* latch and must not be set by a Master-Abort completion.
//! docs/pci-r8-status-errors.md.

use devices::{
    PciConfig, PortDevice, PCI_CONFIG_ADDRESS, PCI_CONFIG_DATA, PCI_HOST_BRIDGE_STATUS_STUB,
    PCI_STATUS_OFFSET, PCI_STATUS_REC_MASTER_ABORT, PCI_STATUS_REC_TARGET_ABORT,
    PCI_STATUS_RW1C_MASK, PCI_STATUS_SIG_TARGET_ABORT,
};

fn host_status(pci: &mut PciConfig) -> u16 {
    let addr = PciConfig::make_address(0, 0, 0, PCI_STATUS_OFFSET, true);
    pci.port_write(PCI_CONFIG_ADDRESS, 4, addr);
    pci.port_read(PCI_CONFIG_DATA + 2, 2) as u16
}

#[test]
fn config_master_abort_sets_rma_not_sta_or_rta() {
    let mut pci = PciConfig::new();
    let addr = PciConfig::make_address(0, 0x1F, 0, 0, true);
    pci.port_write(PCI_CONFIG_ADDRESS, 4, addr);
    assert_eq!(pci.port_read(PCI_CONFIG_DATA, 4), 0xFFFF_FFFF);

    let status = host_status(&mut pci);
    assert_eq!(
        status & PCI_STATUS_REC_MASTER_ABORT,
        PCI_STATUS_REC_MASTER_ABORT,
        "initiator must latch Received Master Abort"
    );
    assert_eq!(
        status & PCI_STATUS_SIG_TARGET_ABORT,
        0,
        "Master-Abort must not set Signaled Target Abort"
    );
    assert_eq!(
        status & PCI_STATUS_REC_TARGET_ABORT,
        0,
        "Master-Abort must not set Received Target Abort"
    );
    assert_eq!(
        status & !PCI_STATUS_RW1C_MASK,
        PCI_HOST_BRIDGE_STATUS_STUB,
        "hardwired CapList/FastB2B/DevSel unchanged"
    );
}

#[test]
fn injected_sta_clears_rw1c_on_host_bridge() {
    let mut pci = PciConfig::new();
    assert!(pci.latch_status_errors(0, 0, 0, PCI_STATUS_SIG_TARGET_ABORT));
    assert_eq!(
        host_status(&mut pci),
        PCI_HOST_BRIDGE_STATUS_STUB | PCI_STATUS_SIG_TARGET_ABORT
    );

    // Write-1 clears STA; write-0 would leave it.
    let st = PciConfig::make_address(0, 0, 0, PCI_STATUS_OFFSET, true);
    pci.port_write(PCI_CONFIG_ADDRESS, 4, st);
    pci.port_write(PCI_CONFIG_DATA + 2, 2, 0);
    assert_eq!(
        host_status(&mut pci),
        PCI_HOST_BRIDGE_STATUS_STUB | PCI_STATUS_SIG_TARGET_ABORT
    );
    pci.port_write(
        PCI_CONFIG_DATA + 2,
        2,
        u32::from(PCI_STATUS_SIG_TARGET_ABORT),
    );
    assert_eq!(host_status(&mut pci), PCI_HOST_BRIDGE_STATUS_STUB);
}

#[test]
fn latch_status_errors_rejects_absent_and_non_rw1c_bits() {
    let mut pci = PciConfig::new();
    assert!(!pci.latch_status_errors(0, 0x1F, 0, PCI_STATUS_SIG_TARGET_ABORT));
    // CapList is RO — must not be injectable through the error latch helper.
    assert!(!pci.latch_status_errors(0, 0, 0, 1 << 4));
    assert_eq!(host_status(&mut pci), PCI_HOST_BRIDGE_STATUS_STUB);
}
