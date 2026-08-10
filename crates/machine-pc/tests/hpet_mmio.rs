//! HPET MMIO device is owned by Machine and survives reset.
//!
//! Spec: IA-PC HPET 1.0a — CAPS/ID presence + Timer 0 comparator stub.
//! Bus claim coverage is in `machine_bus_hpet_caps_and_probe_claim`.

use devices::{
    HpetMmio, HPET_CAPS_ID_VALUE, HPET_DEFAULT_BASE, HPET_DEFAULT_IOAPIC_GSI, HPET_REG_CONFIG,
    HPET_REG_T0_COMPARATOR, HPET_REG_T0_CONFIG, HPET_TN_INT_ENB, HPET_TN_INT_ROUTE_SHIFT,
    HPET_TN_INT_TYPE, IOAPIC_DEFAULT_BASE, IOAPIC_IND_REDTBL0, IOAPIC_IOWIN, IOAPIC_RTE_LEVEL,
    LAPIC_DEFAULT_BASE, LAPIC_REG_SVR, LAPIC_SVR_SW_ENABLE,
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

fn write_u32_mmio(m: &mut Machine, base: u64, off: u32, value: u32) {
    for (i, b) in value.to_le_bytes().into_iter().enumerate() {
        if base == HPET_DEFAULT_BASE {
            assert!(m.hpet.mmio_write_u8(base + u64::from(off) + i as u64, b));
        } else if base == IOAPIC_DEFAULT_BASE {
            assert!(m.ioapic.mmio_write_u8(base + u64::from(off) + i as u64, b));
        } else if base == LAPIC_DEFAULT_BASE {
            assert!(m.lapic.mmio_write_u8(base + u64::from(off) + i as u64, b));
        } else {
            panic!("unexpected base");
        }
    }
}

/// Spec: HPET 1.0a + 82093AA — comparator fire → GSI 2 Fixed RTE → LAPIC vector.
#[test]
fn machine_hpet_comparator_delivers_fixed_via_ioapic() {
    let mut m = Machine::new(64 * 1024);
    write_u32_mmio(
        &mut m,
        LAPIC_DEFAULT_BASE,
        LAPIC_REG_SVR,
        LAPIC_SVR_SW_ENABLE | 0xFF,
    );

    // Unmasked Fixed edge RTE on GSI 2 → vector 0x52, dest APIC ID 0.
    assert!(m
        .ioapic
        .mmio_write_u8(IOAPIC_DEFAULT_BASE, IOAPIC_IND_REDTBL0 + 4));
    write_u32_mmio(&mut m, IOAPIC_DEFAULT_BASE, IOAPIC_IOWIN, 0x0000_0052);
    assert!(m
        .ioapic
        .mmio_write_u8(IOAPIC_DEFAULT_BASE, IOAPIC_IND_REDTBL0 + 5));
    write_u32_mmio(&mut m, IOAPIC_DEFAULT_BASE, IOAPIC_IOWIN, 0);

    write_u32_mmio(&mut m, HPET_DEFAULT_BASE, HPET_REG_CONFIG, 1);
    write_u32_mmio(
        &mut m,
        HPET_DEFAULT_BASE,
        HPET_REG_T0_CONFIG,
        HPET_TN_INT_ENB as u32 | (u32::from(HPET_DEFAULT_IOAPIC_GSI) << HPET_TN_INT_ROUTE_SHIFT),
    );
    write_u32_mmio(&mut m, HPET_DEFAULT_BASE, HPET_REG_T0_COMPARATOR, 40);

    assert_eq!(m.hpet.ioapic_gsi(), HPET_DEFAULT_IOAPIC_GSI);
    let d = m.advance_hpet_ioapic(40).expect("Fixed delivery");
    assert_eq!(d.gsi, 2);
    assert_eq!(d.vector, 0x52);
    assert_eq!(m.lapic.take_interrupt(), Some(0x52));
    // Honesty: DualPic untouched.
    assert_eq!(m.pic.master.irr, 0);
    assert_eq!(m.pic.slave.irr, 0);
}

/// Spec: HPET level IRQ + IOAPIC level RTE — Remote IRR until EOI; still-asserted
/// HPET line re-delivers after EOI (R11).
#[test]
fn machine_hpet_level_ioapic_sets_remote_irr() {
    let mut m = Machine::new(64 * 1024);
    write_u32_mmio(
        &mut m,
        LAPIC_DEFAULT_BASE,
        LAPIC_REG_SVR,
        LAPIC_SVR_SW_ENABLE | 0xFF,
    );

    assert!(m
        .ioapic
        .mmio_write_u8(IOAPIC_DEFAULT_BASE, IOAPIC_IND_REDTBL0 + 4));
    write_u32_mmio(
        &mut m,
        IOAPIC_DEFAULT_BASE,
        IOAPIC_IOWIN,
        IOAPIC_RTE_LEVEL | 0x53,
    );
    assert!(m
        .ioapic
        .mmio_write_u8(IOAPIC_DEFAULT_BASE, IOAPIC_IND_REDTBL0 + 5));
    write_u32_mmio(&mut m, IOAPIC_DEFAULT_BASE, IOAPIC_IOWIN, 0);

    write_u32_mmio(&mut m, HPET_DEFAULT_BASE, HPET_REG_CONFIG, 1);
    write_u32_mmio(
        &mut m,
        HPET_DEFAULT_BASE,
        HPET_REG_T0_CONFIG,
        (HPET_TN_INT_ENB | HPET_TN_INT_TYPE | (2 << HPET_TN_INT_ROUTE_SHIFT)) as u32,
    );
    write_u32_mmio(&mut m, HPET_DEFAULT_BASE, HPET_REG_T0_COMPARATOR, 10);

    assert!(m.advance_hpet_ioapic(10).is_some());
    assert!(m.ioapic.remote_irr(2));
    assert!(m.lapic.tmr_bit(0x53));
    assert_eq!(m.lapic.take_interrupt(), Some(0x53));
    // Still high + Remote IRR → suppressed.
    assert!(m.advance_hpet_ioapic(0).is_none());
    // EOI clears Remote IRR; HPET level still asserted → re-delivery.
    assert_eq!(m.eoi_lapic_ioapic(), Some(0x53));
    assert!(m.hpet.irq_line());
    assert!(m.ioapic.remote_irr(2));
    assert_eq!(m.lapic.take_interrupt(), Some(0x53));
}

/// Spec: HPET 1.0a + 82093AA — after EOI, W1C of T0_INT_STS drops the line so
/// no re-assert.
#[test]
fn machine_hpet_level_eoi_no_reassert_after_status_clear() {
    let mut m = Machine::new(64 * 1024);
    write_u32_mmio(
        &mut m,
        LAPIC_DEFAULT_BASE,
        LAPIC_REG_SVR,
        LAPIC_SVR_SW_ENABLE | 0xFF,
    );
    assert!(m
        .ioapic
        .mmio_write_u8(IOAPIC_DEFAULT_BASE, IOAPIC_IND_REDTBL0 + 4));
    write_u32_mmio(
        &mut m,
        IOAPIC_DEFAULT_BASE,
        IOAPIC_IOWIN,
        IOAPIC_RTE_LEVEL | 0x54,
    );
    assert!(m
        .ioapic
        .mmio_write_u8(IOAPIC_DEFAULT_BASE, IOAPIC_IND_REDTBL0 + 5));
    write_u32_mmio(&mut m, IOAPIC_DEFAULT_BASE, IOAPIC_IOWIN, 0);

    write_u32_mmio(&mut m, HPET_DEFAULT_BASE, HPET_REG_CONFIG, 1);
    write_u32_mmio(
        &mut m,
        HPET_DEFAULT_BASE,
        HPET_REG_T0_CONFIG,
        (HPET_TN_INT_ENB | HPET_TN_INT_TYPE | (2 << HPET_TN_INT_ROUTE_SHIFT)) as u32,
    );
    write_u32_mmio(&mut m, HPET_DEFAULT_BASE, HPET_REG_T0_COMPARATOR, 5);
    assert!(m.advance_hpet_ioapic(5).is_some());
    assert_eq!(m.lapic.take_interrupt(), Some(0x54));
    // Guest clears level status before EOI.
    write_u32_mmio(&mut m, HPET_DEFAULT_BASE, devices::HPET_REG_INTR_STATUS, 1);
    assert!(!m.hpet.irq_line());
    let _ = m.sync_hpet_irq_to_ioapic();
    assert_eq!(m.eoi_lapic_ioapic(), Some(0x54));
    assert!(!m.ioapic.remote_irr(2));
    assert!(m.lapic.take_interrupt().is_none());
}
