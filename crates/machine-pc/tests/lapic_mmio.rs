//! Local APIC MMIO device is owned by Machine and survives reset.
//!
//! Spec: Intel SDM Vol. 3A §10.4.4 / §10.4.8 — ID + Version presence.
//! Bus claim / unmapped-page coverage lives in `machine-pc` lib tests
//! (`machine_bus_lapic_*`).

use devices::{LocalApicMmio, LAPIC_DEFAULT_BASE, LAPIC_REG_ID, LAPIC_VERSION_VALUE};
use machine_pc::Machine;

#[test]
fn device_lapic_wired_on_machine_reset() {
    let mut m = Machine::new(64 * 1024);
    assert_eq!(
        m.lapic
            .mmio_read_u8(LAPIC_DEFAULT_BASE + 0x30)
            .unwrap(),
        LAPIC_VERSION_VALUE as u8
    );
    assert!(m
        .lapic
        .mmio_write_u8(LAPIC_DEFAULT_BASE + u64::from(LAPIC_REG_ID) + 3, 0x01));
    assert_eq!(m.lapic.apic_id(), 0x01);
    m.reset();
    assert_eq!(m.lapic, LocalApicMmio::new());
}
