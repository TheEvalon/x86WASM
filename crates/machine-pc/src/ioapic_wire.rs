//! Host helpers for I/O APIC RTE → Local APIC Fixed delivery + EOI.
//!
//! Coordinates with DualPic only by **not** mirroring: ISA device IRQs still
//! use `DualPic`; IOAPIC GSI pins are a separate path. See
//! `docs/ioapic-r7-rte-irq.md`, `docs/ioapic-r8-eoi.md`.

use devices::{IoApicDelivery, IoApicMmio, LocalApicMmio, IOAPIC_RTE_LEVEL};

/// Assert an I/O APIC input pin and, on Fixed delivery to this LAPIC's ID,
/// latch the vector on the Local APIC.
///
/// Returns the delivery descriptor when the RTE produced one (even if the
/// Local APIC dropped it due to software-disable or a busy pending slot).
/// Level-triggered Fixed deliveries set Remote IRR inside [`IoApicMmio::assert_pin`]
/// and record TMR on the Local APIC via [`LocalApicMmio::inject_fixed_trigger`].
pub fn assert_ioapic_gsi(
    ioapic: &mut IoApicMmio,
    lapic: &mut LocalApicMmio,
    gsi: u8,
    high: bool,
) -> Option<IoApicDelivery> {
    let delivery = ioapic.assert_pin(gsi, high)?;
    if delivery.dest_apic_id == lapic.apic_id() {
        let level = ioapic
            .redtbl_low(gsi)
            .map(|low| low & IOAPIC_RTE_LEVEL != 0)
            .unwrap_or(false);
        let _ = lapic.inject_fixed_trigger(delivery.vector, level);
    }
    Some(delivery)
}

/// Local APIC EOI plus I/O APIC Remote IRR clear for the retired vector.
///
/// Spec: SDM §10.8.5 + 82093AA Remote IRR — level Fixed interrupts need an EOI
/// broadcast so the I/O APIC can re-arm. This stub clears Remote IRR entries
/// matching the vector that left ISR.
pub fn eoi_lapic_and_ioapic(ioapic: &mut IoApicMmio, lapic: &mut LocalApicMmio) -> Option<u8> {
    let vec = lapic.eoi()?;
    ioapic.eoi(vec);
    Some(vec)
}
