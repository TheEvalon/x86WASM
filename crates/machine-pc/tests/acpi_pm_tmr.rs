//! ACPI PM_TMR advances with the instruction-count step clock.
//!
//! Spec: Intel 82371AB (PIIX4) — `PM_TMR` at PMBASE+`08h` is a 24-bit
//! free-running counter at 3.579545 MHz (three times the 8254 input clock).
//! SeaBIOS busy-waits on `IN` of that dword; a stuck-at-zero stub never exits.

use devices::{
    PciConfig, PortDevice, PCI_COMMAND_IO, PCI_COMMAND_OFFSET, PCI_CONFIG_ADDRESS, PCI_CONFIG_DATA,
    PCI_PIIX_ACPI_PMBASE_OFFSET, PCI_PIIX_ACPI_PM_TMR,
};
use machine_pc::{Machine, StepClock};

const PMBASE: u16 = 0xB000;

fn program_pmbase(m: &mut Machine) {
    m.pci.port_write(
        PCI_CONFIG_ADDRESS,
        4,
        PciConfig::make_address(0, 1, 3, PCI_PIIX_ACPI_PMBASE_OFFSET, true),
    );
    m.pci
        .port_write(PCI_CONFIG_DATA, 4, u32::from(PMBASE) | 1);
    m.pci.port_write(
        PCI_CONFIG_ADDRESS,
        4,
        PciConfig::make_address(0, 1, 3, PCI_COMMAND_OFFSET, true),
    );
    let cmd = m.pci.port_read(PCI_CONFIG_DATA, 2) as u16 | PCI_COMMAND_IO;
    m.pci.port_write(PCI_CONFIG_DATA, 2, u32::from(cmd));
    assert_eq!(m.pci.acpi_pm_io_base(), Some(PMBASE));
}

fn read_pm_tmr(m: &Machine) -> u32 {
    let off = PCI_PIIX_ACPI_PM_TMR as usize;
    u32::from_le_bytes([
        m.pci.acpi_pm_io[off],
        m.pci.acpi_pm_io[off + 1],
        m.pci.acpi_pm_io[off + 2],
        m.pci.acpi_pm_io[off + 3],
    ]) & 0x00FF_FFFF
}

#[test]
fn step_clock_advances_acpi_pm_timer_24bit() {
    let mut m = Machine::new(1024 * 1024);
    program_pmbase(&mut m);
    m.set_step_clock(StepClock::enabled_default());

    assert_eq!(read_pm_tmr(&m), 0);

    // One PIT clock per step → three ACPI PM ticks (3.579545 / 1.193182 = 3).
    for _ in 0..10 {
        m.step().expect("step");
    }
    assert_eq!(read_pm_tmr(&m), 30, "10 steps × 3 PM ticks");
}

#[test]
fn pm_tmr_wraps_at_24_bits() {
    let mut m = Machine::new(64 * 1024);
    program_pmbase(&mut m);
    let off = PCI_PIIX_ACPI_PM_TMR as usize;
    m.pci.acpi_pm_io[off..off + 4].copy_from_slice(&0x00FF_FFFE_u32.to_le_bytes());
    m.set_step_clock(StepClock::enabled_default());
    m.step().expect("step"); // +3 → wraps to 1
    assert_eq!(read_pm_tmr(&m), 1);
}
