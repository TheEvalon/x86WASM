//! Local APIC MMIO device is owned by Machine and survives reset.
//!
//! Spec: Intel SDM Vol. 3A §10.4.4 / §10.4.8 / §10.5 — ID + Version + timer stub.
//! Bus claim / unmapped-page coverage lives in `machine-pc` lib tests
//! (`machine_bus_lapic_*`).

use devices::{
    LocalApicMmio, LAPIC_DEFAULT_BASE, LAPIC_REG_ID, LAPIC_REG_LVT_TIMER, LAPIC_REG_SVR,
    LAPIC_REG_TIMER_DCR, LAPIC_REG_TIMER_ICR, LAPIC_SVR_SW_ENABLE, LAPIC_VERSION_VALUE,
};
use machine_pc::Machine;

#[test]
fn device_lapic_wired_on_machine_reset() {
    let mut m = Machine::new(64 * 1024);
    assert_eq!(
        m.lapic.mmio_read_u8(LAPIC_DEFAULT_BASE + 0x30).unwrap(),
        LAPIC_VERSION_VALUE as u8
    );
    assert!(m
        .lapic
        .mmio_write_u8(LAPIC_DEFAULT_BASE + u64::from(LAPIC_REG_ID) + 3, 0x01));
    assert_eq!(m.lapic.apic_id(), 0x01);
    m.reset();
    assert_eq!(m.lapic, LocalApicMmio::new());
}

/// Spec: SDM §10.5 — Machine::tick_lapic_timer latches LVT vector locally.
#[test]
fn machine_tick_lapic_timer_latches_local_vector() {
    let mut m = Machine::new(64 * 1024);
    for (i, b) in (LAPIC_SVR_SW_ENABLE | 0xFF)
        .to_le_bytes()
        .into_iter()
        .enumerate()
    {
        assert!(m
            .lapic
            .mmio_write_u8(LAPIC_DEFAULT_BASE + u64::from(LAPIC_REG_SVR) + i as u64, b));
    }
    for (i, b) in 0b1011u32.to_le_bytes().into_iter().enumerate() {
        assert!(m.lapic.mmio_write_u8(
            LAPIC_DEFAULT_BASE + u64::from(LAPIC_REG_TIMER_DCR) + i as u64,
            b
        ));
    }
    for (i, b) in 0x52u32.to_le_bytes().into_iter().enumerate() {
        assert!(m.lapic.mmio_write_u8(
            LAPIC_DEFAULT_BASE + u64::from(LAPIC_REG_LVT_TIMER) + i as u64,
            b
        ));
    }
    for (i, b) in 1u32.to_le_bytes().into_iter().enumerate() {
        assert!(m.lapic.mmio_write_u8(
            LAPIC_DEFAULT_BASE + u64::from(LAPIC_REG_TIMER_ICR) + i as u64,
            b
        ));
    }
    assert!(m.tick_lapic_timer(1));
    assert_eq!(m.lapic.take_interrupt(), Some(0x52));
}
