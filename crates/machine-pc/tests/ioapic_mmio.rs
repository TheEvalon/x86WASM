//! I/O APIC MMIO device is owned by Machine and survives reset.
//!
//! Spec: Intel 82093AA — IOREGSEL/IOWIN presence + RTE → LAPIC Fixed path.
//! Wire-all coverage is in `machine_bus_ioapic_and_platform_wire_all`.

use devices::{
    IoApicMmio, IOAPIC_DEFAULT_BASE, IOAPIC_IND_REDTBL0, IOAPIC_IND_VER, IOAPIC_IOREGSEL,
    IOAPIC_IOWIN, IOAPIC_VER_VALUE, LAPIC_DEFAULT_BASE, LAPIC_REG_ID, LAPIC_REG_SVR,
    LAPIC_SVR_SW_ENABLE,
};
use machine_pc::Machine;

#[test]
fn device_ioapic_wired_on_machine_reset() {
    let mut m = Machine::new(64 * 1024);
    assert!(m.ioapic.mmio_write_u8(
        IOAPIC_DEFAULT_BASE + u64::from(IOAPIC_IOREGSEL),
        IOAPIC_IND_VER
    ));
    assert_eq!(m.ioapic.index(), IOAPIC_IND_VER);
    let mut bytes = [0u8; 4];
    for i in 0..4u64 {
        bytes[i as usize] = m
            .ioapic
            .mmio_read_u8(IOAPIC_DEFAULT_BASE + 0x10 + i)
            .unwrap();
    }
    assert_eq!(u32::from_le_bytes(bytes), IOAPIC_VER_VALUE);
    m.reset();
    assert_eq!(m.ioapic, IoApicMmio::new());
}

fn write_ioapic_u32(m: &mut Machine, off: u32, value: u32) {
    for (i, b) in value.to_le_bytes().into_iter().enumerate() {
        assert!(m
            .ioapic
            .mmio_write_u8(IOAPIC_DEFAULT_BASE + u64::from(off) + i as u64, b));
    }
}

fn write_lapic_u32(m: &mut Machine, off: u32, value: u32) {
    for (i, b) in value.to_le_bytes().into_iter().enumerate() {
        assert!(m
            .lapic
            .mmio_write_u8(LAPIC_DEFAULT_BASE + u64::from(off) + i as u64, b));
    }
}

/// Spec: 82093AA Fixed RTE → Local APIC when dest APIC ID matches.
#[test]
fn machine_ioapic_rte_delivers_to_matching_lapic() {
    let mut m = Machine::new(64 * 1024);
    // LAPIC ID = 1, software enabled.
    write_lapic_u32(&mut m, LAPIC_REG_ID, 0x0100_0000);
    write_lapic_u32(&mut m, LAPIC_REG_SVR, LAPIC_SVR_SW_ENABLE | 0xFF);

    // GSI 7 → vector 0x37, unmasked Fixed, dest APIC ID 1.
    assert!(m
        .ioapic
        .mmio_write_u8(IOAPIC_DEFAULT_BASE, IOAPIC_IND_REDTBL0 + 14));
    write_ioapic_u32(&mut m, IOAPIC_IOWIN, 0x0000_0037);
    assert!(m
        .ioapic
        .mmio_write_u8(IOAPIC_DEFAULT_BASE, IOAPIC_IND_REDTBL0 + 15));
    write_ioapic_u32(&mut m, IOAPIC_IOWIN, 0x0100_0000);

    let d = m.assert_ioapic_gsi(7, true).expect("delivery");
    assert_eq!(d.vector, 0x37);
    assert_eq!(d.dest_apic_id, 1);
    assert_eq!(m.lapic.take_interrupt(), Some(0x37));
    // Honesty: DualPic IRR unchanged.
    assert_eq!(m.pic.master.irr, 0);
    assert_eq!(m.pic.slave.irr, 0);
}

/// Spec: 82093AA Remote IRR — level Fixed suppresses until LAPIC+IOAPIC EOI.
#[test]
fn machine_level_remote_irr_cleared_by_eoi_helper() {
    use devices::{IOAPIC_RTE_LEVEL, IOAPIC_RTE_REMOTE_IRR};
    let mut m = Machine::new(64 * 1024);
    write_lapic_u32(&mut m, LAPIC_REG_ID, 0);
    write_lapic_u32(&mut m, LAPIC_REG_SVR, LAPIC_SVR_SW_ENABLE | 0xFF);

    assert!(m
        .ioapic
        .mmio_write_u8(IOAPIC_DEFAULT_BASE, IOAPIC_IND_REDTBL0));
    write_ioapic_u32(&mut m, IOAPIC_IOWIN, IOAPIC_RTE_LEVEL | 0x42);
    assert!(m
        .ioapic
        .mmio_write_u8(IOAPIC_DEFAULT_BASE, IOAPIC_IND_REDTBL0 + 1));
    write_ioapic_u32(&mut m, IOAPIC_IOWIN, 0); // dest APIC ID 0

    assert!(m.assert_ioapic_gsi(0, true).is_some());
    assert!(m.ioapic.remote_irr(0));
    assert_eq!(m.lapic.take_interrupt(), Some(0x42));
    // Suppressed while Remote IRR set.
    assert!(m.assert_ioapic_gsi(0, true).is_none());

    assert_eq!(m.eoi_lapic_ioapic(), Some(0x42));
    assert!(!m.ioapic.remote_irr(0));
    // Pin still high → new delivery + Remote IRR.
    assert!(m.assert_ioapic_gsi(0, true).is_some());
    assert_eq!(
        m.ioapic.redtbl_low(0).unwrap() & IOAPIC_RTE_REMOTE_IRR,
        IOAPIC_RTE_REMOTE_IRR
    );
}
