//! ACPI PM1a_CNT sleep-enable stub (SLP_EN + SLP_TYP).
//!
//! Spec: ACPI Specification fixed-hardware `PM1a_CNT_BLK` — `SLP_TYPx` sticky,
//! `SLP_EN` write-only trigger. Intel 82371AB (PIIX4) PM I/O at PMBASE+4.
//! Model: docs/acpi-r8-pm1-sleep.md — no S3 machine; host latches only.

use devices::{
    PciConfig, PortDevice, ACPI_PM1_CNT_SCI_EN, ACPI_PM1_CNT_SLP_EN, ACPI_PM1_CNT_SLP_TYP_MASK,
    ACPI_PM1_CNT_SLP_TYP_SHIFT, ACPI_SLP_TYP_S5, PCI_COMMAND_IO, PCI_COMMAND_OFFSET,
    PCI_CONFIG_ADDRESS, PCI_CONFIG_DATA, PCI_PIIX_ACPI_PM1A_CNT, PCI_PIIX_ACPI_PMBASE_OFFSET,
};

fn enable_pm_io(pci: &mut PciConfig, pmbase: u16) {
    let addr = PciConfig::make_address(0, 1, 3, PCI_PIIX_ACPI_PMBASE_OFFSET, true);
    pci.port_write(PCI_CONFIG_ADDRESS, 4, addr);
    pci.port_write(PCI_CONFIG_DATA, 4, u32::from(pmbase) | 1);
    let cmd = PciConfig::make_address(0, 1, 3, PCI_COMMAND_OFFSET, true);
    pci.port_write(PCI_CONFIG_ADDRESS, 4, cmd);
    pci.port_write(PCI_CONFIG_DATA, 2, u32::from(PCI_COMMAND_IO));
}

fn pm1_cnt_sleep(sci_en: bool, typ: u8, slp_en: bool) -> u32 {
    let mut v = u16::from(typ) << ACPI_PM1_CNT_SLP_TYP_SHIFT;
    if sci_en {
        v |= ACPI_PM1_CNT_SCI_EN;
    }
    if slp_en {
        v |= ACPI_PM1_CNT_SLP_EN;
    }
    u32::from(v)
}

#[test]
fn slp_en_with_s5_typ_latches_power_off_and_does_not_stick() {
    let mut pci = PciConfig::new();
    enable_pm_io(&mut pci, 0xB000);
    assert!(!pci.acpi_power_off_pending());
    assert_eq!(pci.acpi_sleep_request(), None);

    // Spec: ACPI — SLP_EN is a write-only trigger; SLP_TYP stays sticky.
    pci.port_write(
        0xB000 + u16::from(PCI_PIIX_ACPI_PM1A_CNT),
        2,
        pm1_cnt_sleep(true, ACPI_SLP_TYP_S5, true),
    );

    assert_eq!(
        pci.acpi_pm1_cnt() & ACPI_PM1_CNT_SCI_EN,
        ACPI_PM1_CNT_SCI_EN
    );
    assert_eq!(
        pci.acpi_pm1_cnt() & ACPI_PM1_CNT_SLP_EN,
        0,
        "SLP_EN must not latch in PM1_CNT"
    );
    assert_eq!(
        (pci.acpi_pm1_cnt() & ACPI_PM1_CNT_SLP_TYP_MASK) >> ACPI_PM1_CNT_SLP_TYP_SHIFT,
        u16::from(ACPI_SLP_TYP_S5)
    );
    assert!(pci.acpi_power_off_pending());
    assert_eq!(pci.acpi_sleep_request(), None);

    assert!(pci.take_acpi_power_off_request());
    assert!(!pci.acpi_power_off_pending());
    assert!(!pci.take_acpi_power_off_request());
}

#[test]
fn slp_en_with_non_s5_typ_latches_sleep_request_only() {
    let mut pci = PciConfig::new();
    enable_pm_io(&mut pci, 0xB000);
    let typ = 1u8; // documented non-S5 SLP_TYP; no resume path

    pci.port_write(
        0xB000 + u16::from(PCI_PIIX_ACPI_PM1A_CNT),
        2,
        pm1_cnt_sleep(false, typ, true),
    );

    assert!(!pci.acpi_power_off_pending());
    assert_eq!(pci.acpi_sleep_request(), Some(typ));
    assert_eq!(pci.take_acpi_sleep_request(), Some(typ));
    assert_eq!(pci.acpi_sleep_request(), None);
    assert_eq!(pci.take_acpi_sleep_request(), None);
}

#[test]
fn slp_typ_without_slp_en_does_not_request_sleep_or_power_off() {
    let mut pci = PciConfig::new();
    enable_pm_io(&mut pci, 0xB000);

    pci.port_write(
        0xB000 + u16::from(PCI_PIIX_ACPI_PM1A_CNT),
        2,
        pm1_cnt_sleep(true, ACPI_SLP_TYP_S5, false),
    );
    assert!(!pci.acpi_power_off_pending());
    assert_eq!(pci.acpi_sleep_request(), None);
    assert_eq!(
        (pci.acpi_pm1_cnt() & ACPI_PM1_CNT_SLP_TYP_MASK) >> ACPI_PM1_CNT_SLP_TYP_SHIFT,
        u16::from(ACPI_SLP_TYP_S5),
        "SLP_TYP remains sticky without SLP_EN"
    );
}

#[test]
fn sleep_latches_clear_on_pci_reset() {
    let mut pci = PciConfig::new();
    enable_pm_io(&mut pci, 0xB000);
    pci.port_write(
        0xB000 + u16::from(PCI_PIIX_ACPI_PM1A_CNT),
        2,
        pm1_cnt_sleep(false, ACPI_SLP_TYP_S5, true),
    );
    assert!(pci.acpi_power_off_pending());
    pci.reset();
    assert!(!pci.acpi_power_off_pending());
    assert_eq!(pci.acpi_sleep_request(), None);
}
