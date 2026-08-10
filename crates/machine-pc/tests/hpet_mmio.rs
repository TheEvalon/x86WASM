//! HPET MMIO device is owned by Machine and survives reset.
//!
//! Spec: IA-PC HPET 1.0a — CAPS/ID presence. Bus claim coverage is in
//! `machine_bus_hpet_caps_and_probe_claim`.

use devices::{HpetMmio, HPET_CAPS_ID_VALUE, HPET_DEFAULT_BASE, HPET_REG_CONFIG};
use machine_pc::Machine;

#[test]
fn device_hpet_wired_on_machine_reset() {
    let mut m = Machine::new(64 * 1024);
    assert_eq!(
        m.hpet.mmio_read_u8(HPET_DEFAULT_BASE).unwrap(),
        HPET_CAPS_ID_VALUE as u8
    );
    assert!(m
        .hpet
        .mmio_write_u8(HPET_DEFAULT_BASE + u64::from(HPET_REG_CONFIG), 0x01));
    assert_eq!(m.hpet.config(), 1);
    m.reset();
    assert_eq!(m.hpet, HpetMmio::new());
}
