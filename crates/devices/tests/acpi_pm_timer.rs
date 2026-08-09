//! PIIX4 ACPI PM1a / PM_TMR behaviour beyond store/readback.
//!
//! Spec: ACPI Specification fixed-hardware registers (PM1a_EVT / PM1a_CNT /
//! PM_TMR) and Intel 82371AB (PIIX4) ACPI function PM I/O at PMBASE. Timer
//! frequency model: 3.579545 MHz (`ACPI_PM_TIMER_HZ`).

use devices::{
    PciConfig, PortDevice, ACPI_PM1_CNT_SCI_EN, ACPI_PM1_EN_PWRBTN, ACPI_PM1_EN_TMR,
    ACPI_PM1_STS_PWRBTN, ACPI_PM1_STS_TMR, ACPI_PM_TIMER_HZ, ACPI_PM_TIMER_MASK, PCI_COMMAND_IO,
    PCI_COMMAND_OFFSET, PCI_CONFIG_ADDRESS, PCI_CONFIG_DATA, PCI_PIIX_ACPI_PM1A_CNT,
    PCI_PIIX_ACPI_PM1A_EVT, PCI_PIIX_ACPI_PMBASE_OFFSET, PCI_PIIX_ACPI_PM_TMR,
};

fn enable_pm_io(pci: &mut PciConfig, pmbase: u16) {
    let addr = PciConfig::make_address(0, 1, 3, PCI_PIIX_ACPI_PMBASE_OFFSET, true);
    pci.port_write(PCI_CONFIG_ADDRESS, 4, addr);
    pci.port_write(PCI_CONFIG_DATA, 4, u32::from(pmbase) | 1);
    let cmd = PciConfig::make_address(0, 1, 3, PCI_COMMAND_OFFSET, true);
    pci.port_write(PCI_CONFIG_ADDRESS, 4, cmd);
    pci.port_write(PCI_CONFIG_DATA, 2, u32::from(PCI_COMMAND_IO));
}

#[test]
fn acpi_pm_timer_is_free_running_and_accepts_load() {
    let mut pci = PciConfig::new();
    enable_pm_io(&mut pci, 0xB000);
    assert_eq!(pci.acpi_pm_timer(), 0);

    pci.tick_acpi_pm(100);
    assert_eq!(
        pci.port_read(0xB000 + u16::from(PCI_PIIX_ACPI_PM_TMR), 4),
        100
    );
    // Guest/firmware may load the counter; tick continues from there.
    pci.port_write(0xB000 + u16::from(PCI_PIIX_ACPI_PM_TMR), 4, 0x1000);
    assert_eq!(pci.acpi_pm_timer(), 0x1000);
    // Same dword lives in acpi_pm_io (machine step-clock may poke these bytes).
    let off = PCI_PIIX_ACPI_PM_TMR as usize;
    assert_eq!(
        u32::from_le_bytes(pci.acpi_pm_io[off..off + 4].try_into().unwrap()),
        0x1000
    );

    pci.tick_acpi_pm_ns(1_000_000_000);
    assert_eq!(
        pci.acpi_pm_timer(),
        (0x1000 + ACPI_PM_TIMER_HZ) & ACPI_PM_TIMER_MASK
    );
}

#[test]
fn acpi_pm_timer_visible_when_machine_pokes_acpi_pm_io() {
    // Composition with machine-pc step-clock freerun: sibling writes
    // acpi_pm_io[PM_TMR..] directly; guest port reads must see that value.
    let mut pci = PciConfig::new();
    enable_pm_io(&mut pci, 0xB000);
    let off = PCI_PIIX_ACPI_PM_TMR as usize;
    pci.acpi_pm_io[off..off + 4].copy_from_slice(&0x00AB_CDEF_u32.to_le_bytes());
    assert_eq!(pci.acpi_pm_timer(), 0x00AB_CDEF);
    assert_eq!(
        pci.port_read(0xB000 + u16::from(PCI_PIIX_ACPI_PM_TMR), 4),
        0x00AB_CDEF
    );
}

#[test]
fn acpi_pm_timer_msb_sets_tmr_sts() {
    let mut pci = PciConfig::new();
    enable_pm_io(&mut pci, 0xB000);
    // Advance just past bit 23.
    pci.tick_acpi_pm(1 << 23);
    assert_eq!(pci.acpi_pm1_sts() & ACPI_PM1_STS_TMR, ACPI_PM1_STS_TMR);
    // Clear by store (full W1C deferred).
    pci.port_write(0xB000 + u16::from(PCI_PIIX_ACPI_PM1A_EVT), 2, 0);
    assert_eq!(pci.acpi_pm1_sts() & ACPI_PM1_STS_TMR, 0);
}

#[test]
fn acpi_power_button_and_sci_en_stub() {
    let mut pci = PciConfig::new();
    enable_pm_io(&mut pci, 0xB000);
    assert!(!pci.acpi_sci_asserted());

    pci.acpi_assert_power_button();
    assert_eq!(
        pci.acpi_pm1_sts() & ACPI_PM1_STS_PWRBTN,
        ACPI_PM1_STS_PWRBTN
    );
    // Enable power-button SCI source but SCI_EN still clear → no SCI.
    pci.port_write(
        0xB000 + u16::from(PCI_PIIX_ACPI_PM1A_EVT) + 2,
        2,
        u32::from(ACPI_PM1_EN_PWRBTN),
    );
    assert!(!pci.acpi_sci_asserted());

    pci.port_write(
        0xB000 + u16::from(PCI_PIIX_ACPI_PM1A_CNT),
        2,
        u32::from(ACPI_PM1_CNT_SCI_EN),
    );
    assert!(pci.acpi_sci_asserted());

    // Clear PWRBTN_STS → SCI drops.
    pci.port_write(0xB000 + u16::from(PCI_PIIX_ACPI_PM1A_EVT), 2, 0);
    assert!(!pci.acpi_sci_asserted());
}

#[test]
fn acpi_sci_from_timer_overflow_when_enabled() {
    let mut pci = PciConfig::new();
    enable_pm_io(&mut pci, 0x4000);
    pci.port_write(
        0x4000 + u16::from(PCI_PIIX_ACPI_PM1A_EVT) + 2,
        2,
        u32::from(ACPI_PM1_EN_TMR),
    );
    pci.port_write(
        0x4000 + u16::from(PCI_PIIX_ACPI_PM1A_CNT),
        2,
        u32::from(ACPI_PM1_CNT_SCI_EN),
    );
    assert!(!pci.acpi_sci_asserted());
    pci.tick_acpi_pm(1 << 23);
    assert!(pci.acpi_sci_asserted());
}

#[test]
fn acpi_pm1_cnt_slp_en_does_not_stick() {
    let mut pci = PciConfig::new();
    enable_pm_io(&mut pci, 0xB000);
    // SCI_EN | SLP_EN | some SLP_TYP
    pci.port_write(0xB000 + u16::from(PCI_PIIX_ACPI_PM1A_CNT), 2, 0x2C01);
    assert_eq!(
        pci.acpi_pm1_cnt() & ACPI_PM1_CNT_SCI_EN,
        ACPI_PM1_CNT_SCI_EN
    );
    assert_eq!(pci.acpi_pm1_cnt() & (1 << 13), 0, "SLP_EN must not latch");
}
