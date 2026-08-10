//! PIIX IDE Prog IF honesty: bus-master bit matches BMIDE stubs only.
//!
//! Spec: PCI Local Bus — mass-storage / IDE programming interface bit 7
//! advertises bus-master IDE. This tree keeps `PCI_PROG_IF_IDE_BUS_MASTER`
//! (`0x80`) because the BMIDE BAR and host-called PRD walkers exist. A guest
//! write to BMICOM.SSBM does not start a transfer (no ATA DMA engine).
//! Docs: `docs/pci-r4-bar-sizing-and-enumeration.md`,
//! `docs/pci-bmide-prd-directions.md`.

use devices::{
    PciConfig, PortDevice, PCI_COMMAND_BUS_MASTER, PCI_COMMAND_IO, PCI_COMMAND_OFFSET,
    PCI_CONFIG_ADDRESS, PCI_CONFIG_DATA, PCI_PIIX_IDE_BMIBA_OFFSET, PCI_PIIX_IDE_BMICOM_PRIMARY,
    PCI_PIIX_IDE_BMICOM_SSBM, PCI_PIIX_IDE_BMIDTP_PRIMARY, PCI_PIIX_IDE_BMISTA_ACTIVE,
    PCI_PIIX_IDE_BMISTA_PRIMARY, PCI_PROG_IF_IDE_BUS_MASTER,
};

fn cfg_read_u8(pci: &mut PciConfig, device: u8, function: u8, offset: u8) -> u8 {
    let addr = PciConfig::make_address(0, device, function, offset & !3, true);
    pci.port_write(PCI_CONFIG_ADDRESS, 4, addr);
    let dword = pci.port_read(PCI_CONFIG_DATA, 4);
    ((dword >> (8 * (offset & 3))) & 0xFF) as u8
}

fn program_bmide(pci: &mut PciConfig, bmiba: u16) {
    pci.port_write(
        PCI_CONFIG_ADDRESS,
        4,
        PciConfig::make_address(0, 1, 1, PCI_PIIX_IDE_BMIBA_OFFSET, true),
    );
    pci.port_write(PCI_CONFIG_DATA, 4, u32::from(bmiba) | 1);
    pci.port_write(
        PCI_CONFIG_ADDRESS,
        4,
        PciConfig::make_address(0, 1, 1, PCI_COMMAND_OFFSET, true),
    );
    pci.port_write(
        PCI_CONFIG_DATA,
        2,
        u32::from(PCI_COMMAND_IO | PCI_COMMAND_BUS_MASTER),
    );
}

#[test]
fn piix_ide_prog_if_advertises_bus_master_stub() {
    let mut pci = PciConfig::new();
    // Class code dword at 0x08: rev, prog IF, subclass, class.
    assert_eq!(
        cfg_read_u8(&mut pci, 1, 1, 0x09),
        PCI_PROG_IF_IDE_BUS_MASTER,
        "Prog IF must stay 0x80 to match BMIDE BAR + host PRD stubs"
    );
    assert_eq!(PCI_PROG_IF_IDE_BUS_MASTER, 0x80);
}

/// Guest BMICOM.SSBM is store/readback only: writing Start does not walk the
/// PRDT or touch guest memory. Transfers require the host `start_bm_*` helpers.
#[test]
fn guest_bmide_ssbm_write_does_not_start_prd_transfer() {
    let mut pci = PciConfig::new();
    program_bmide(&mut pci, 0xF000);

    // Point BMIDTP at a plausible table and seed a guest buffer pattern via a
    // fake mem space the port path cannot reach — we only check that SSBM
    // store/readback leaves BMISTA.Active clear and does not invoke walkers.
    pci.port_write(0xF000 + u16::from(PCI_PIIX_IDE_BMIDTP_PRIMARY), 4, 0x2000);
    pci.port_write(
        0xF000 + u16::from(PCI_PIIX_IDE_BMICOM_PRIMARY),
        1,
        u32::from(PCI_PIIX_IDE_BMICOM_SSBM),
    );

    assert_eq!(
        pci.port_read(0xF000 + u16::from(PCI_PIIX_IDE_BMICOM_PRIMARY), 1) as u8
            & PCI_PIIX_IDE_BMICOM_SSBM,
        PCI_PIIX_IDE_BMICOM_SSBM,
        "SSBM stores"
    );
    // BMISTA Active (bit 0) stays clear — no walk started from the guest write.
    assert_eq!(
        pci.bmide_io[PCI_PIIX_IDE_BMISTA_PRIMARY as usize] & PCI_PIIX_IDE_BMISTA_ACTIVE,
        0,
        "guest SSBM must not latch BMISTA.Active"
    );
}
