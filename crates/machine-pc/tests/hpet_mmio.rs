//! HPET MMIO device is owned by Machine and survives reset.
//!
//! Spec: IA-PC HPET 1.0a — CAPS/ID presence + Timer 0 comparator stub.
//! Bus claim coverage is in `machine_bus_hpet_caps_and_probe_claim`.

use devices::{
    HpetMmio, HPET_CAPS_ID_VALUE, HPET_DEFAULT_BASE, HPET_REG_CONFIG, HPET_REG_T0_COMPARATOR,
    HPET_REG_T0_CONFIG, HPET_TN_INT_ENB, HPET_TN_INT_TYPE,
};
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

/// Spec: HPET 1.0a — Machine::advance_hpet drives Timer 0 stub IRQ latch.
#[test]
fn machine_advance_hpet_raises_device_irq_line() {
    let mut m = Machine::new(64 * 1024);
    for (i, b) in 1u32.to_le_bytes().into_iter().enumerate() {
        assert!(m
            .hpet
            .mmio_write_u8(HPET_DEFAULT_BASE + u64::from(HPET_REG_CONFIG) + i as u64, b));
    }
    let t0 = (HPET_TN_INT_ENB | HPET_TN_INT_TYPE) as u32;
    for (i, b) in t0.to_le_bytes().into_iter().enumerate() {
        assert!(m.hpet.mmio_write_u8(
            HPET_DEFAULT_BASE + u64::from(HPET_REG_T0_CONFIG) + i as u64,
            b
        ));
    }
    for (i, b) in 25u32.to_le_bytes().into_iter().enumerate() {
        assert!(m.hpet.mmio_write_u8(
            HPET_DEFAULT_BASE + u64::from(HPET_REG_T0_COMPARATOR) + i as u64,
            b
        ));
    }
    assert!(!m.hpet.irq_line());
    assert!(m.advance_hpet(25));
    assert!(m.hpet.irq_line());
    // Honesty: PIC IR0 is unaffected (no auto-wire).
    assert_eq!(m.pic.master.irr & 1, 0);
}
