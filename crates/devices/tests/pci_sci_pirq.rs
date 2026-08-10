//! ACPI SCI_EN honesty and optional SCI→PIRQ soft-wire.
//!
//! Spec: ACPI §4.8.1 — SCI is `(PM1_STS & PM1_EN)` while `SCI_EN=1`.
//! Intel 82371SB PIRQRC — software PIRQ lines route to ISA IRQs when enabled.
//! Model: docs/pci-r8-sci-pirq.md (optional host stub; no FADT SCI_INT yet).

use devices::{
    DualPic, PciConfig, PortDevice, ACPI_PM1_CNT_SCI_EN, ACPI_PM1_EN_PWRBTN, ACPI_PM1_STS_PWRBTN,
    PCI_COMMAND_IO, PCI_COMMAND_OFFSET, PCI_CONFIG_ADDRESS, PCI_CONFIG_DATA,
    PCI_PIIX_ACPI_PM1A_CNT, PCI_PIIX_ACPI_PM1A_EVT, PCI_PIIX_ACPI_PMBASE_OFFSET,
    PCI_PIIX_ISA_PIRQRC_OFFSET, PIC_MASTER_DATA,
};

fn enable_pm_io(pci: &mut PciConfig, pmbase: u16) {
    let addr = PciConfig::make_address(0, 1, 3, PCI_PIIX_ACPI_PMBASE_OFFSET, true);
    pci.port_write(PCI_CONFIG_ADDRESS, 4, addr);
    pci.port_write(PCI_CONFIG_DATA, 4, u32::from(pmbase) | 1);
    let cmd = PciConfig::make_address(0, 1, 3, PCI_COMMAND_OFFSET, true);
    pci.port_write(PCI_CONFIG_ADDRESS, 4, cmd);
    pci.port_write(PCI_CONFIG_DATA, 2, u32::from(PCI_COMMAND_IO));
}

fn assert_pwrbtn_sci_sources(pci: &mut PciConfig) {
    pci.acpi_assert_power_button();
    pci.port_write(
        0xB000 + u16::from(PCI_PIIX_ACPI_PM1A_EVT) + 2,
        2,
        u32::from(ACPI_PM1_EN_PWRBTN),
    );
}

#[test]
fn sci_en_clear_keeps_sci_deasserted_despite_sts_and_en() {
    let mut pci = PciConfig::new();
    enable_pm_io(&mut pci, 0xB000);
    assert_pwrbtn_sci_sources(&mut pci);
    assert_eq!(
        pci.acpi_pm1_sts() & ACPI_PM1_STS_PWRBTN,
        ACPI_PM1_STS_PWRBTN
    );
    assert_eq!(pci.acpi_pm1_en() & ACPI_PM1_EN_PWRBTN, ACPI_PM1_EN_PWRBTN);
    // SCI_EN still 0 → honest deassert.
    assert_eq!(pci.acpi_pm1_cnt() & ACPI_PM1_CNT_SCI_EN, 0);
    assert!(!pci.acpi_sci_asserted());

    pci.port_write(
        0xB000 + u16::from(PCI_PIIX_ACPI_PM1A_CNT),
        2,
        u32::from(ACPI_PM1_CNT_SCI_EN),
    );
    assert!(pci.acpi_sci_asserted());
}

#[test]
fn sync_acpi_sci_to_pirq_mirrors_level_and_routes_when_enabled() {
    let mut pci = PciConfig::new();
    let mut pic = DualPic::new();
    enable_pm_io(&mut pci, 0xB000);
    assert_pwrbtn_sci_sources(&mut pci);
    pci.port_write(
        0xB000 + u16::from(PCI_PIIX_ACPI_PM1A_CNT),
        2,
        u32::from(ACPI_PM1_CNT_SCI_EN),
    );
    assert!(pci.acpi_sci_asserted());

    // Default PIRQRC disabled — SCI soft-wire asserts the pin but not the PIC.
    pci.sync_acpi_sci_to_pirq(0);
    assert!(pci.pirq_line(0));
    pci.sync_pirq_to_pic(&mut pic);
    assert_eq!(pci.pirq_pic_driven, 0);

    // Route PIRQA → IRQ5, unmask, re-sync.
    pci.port_write(
        PCI_CONFIG_ADDRESS,
        4,
        PciConfig::make_address(0, 1, 0, PCI_PIIX_ISA_PIRQRC_OFFSET, true),
    );
    pci.port_write(PCI_CONFIG_DATA, 1, 0x05);
    pic.port_write(PIC_MASTER_DATA, 1, 0x00); // unmask all master IRQs for the stub
    pci.sync_acpi_sci_to_pirq(0);
    pci.sync_pirq_to_pic(&mut pic);
    assert_eq!(pci.pirq_pic_driven & (1 << 5), 1 << 5);

    // Clear PWRBTN_STS → SCI drops → soft-wire deasserts PIRQ and PIC line.
    pci.port_write(
        0xB000 + u16::from(PCI_PIIX_ACPI_PM1A_EVT),
        2,
        u32::from(ACPI_PM1_STS_PWRBTN),
    );
    assert!(!pci.acpi_sci_asserted());
    pci.sync_acpi_sci_to_pirq(0);
    pci.sync_pirq_to_pic(&mut pic);
    assert!(!pci.pirq_line(0));
    assert_eq!(pci.pirq_pic_driven & (1 << 5), 0);
}
