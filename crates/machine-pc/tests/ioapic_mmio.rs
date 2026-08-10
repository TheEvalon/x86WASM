//! I/O APIC MMIO device is owned by Machine and survives reset.
//!
//! Spec: Intel 82093AA — IOREGSEL/IOWIN presence. Wire-all coverage is in
//! `machine_bus_ioapic_and_platform_wire_all`.

use devices::{IoApicMmio, IOAPIC_DEFAULT_BASE, IOAPIC_IND_VER, IOAPIC_IOREGSEL, IOAPIC_VER_VALUE};
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
