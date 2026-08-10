//! Host helpers for Round-7 I/O APIC RTE → Local APIC Fixed delivery.
//!
//! Coordinates with DualPic only by **not** mirroring: ISA device IRQs still
//! use `DualPic`; IOAPIC GSI pins are a separate path. See
//! `docs/ioapic-r7-rte-irq.md`.

use devices::{IoApicDelivery, IoApicMmio, LocalApicMmio};

/// Assert an I/O APIC input pin and, on Fixed delivery to this LAPIC's ID,
/// latch the vector on the Local APIC.
///
/// Returns the delivery descriptor when the RTE produced one (even if the
/// Local APIC dropped it due to software-disable or a busy pending slot).
pub fn assert_ioapic_gsi(
    ioapic: &mut IoApicMmio,
    lapic: &mut LocalApicMmio,
    gsi: u8,
    high: bool,
) -> Option<IoApicDelivery> {
    let delivery = ioapic.assert_pin(gsi, high)?;
    if delivery.dest_apic_id == lapic.apic_id() {
        let _ = lapic.inject_fixed(delivery.vector);
    }
    Some(delivery)
}
