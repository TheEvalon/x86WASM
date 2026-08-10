//! Host helpers for HPET Timer 0 → I/O APIC GSI delivery (R10).
//!
//! Kept out of the `Machine` monolith so parallel lanes can merge without
//! rewriting MMIO dispatch. Legacy PIC replacement and FSB/MSI remain out of
//! scope — see `docs/hpet-r10-ioapic-wire.md`.
//!
//! Ownership (R10 timers-apic): this module + thin `Machine::advance_hpet` /
//! `Machine::sync_hpet_irq_to_ioapic` / `Machine::advance_hpet_ioapic` wrappers.

use devices::{HpetMmio, IoApicDelivery, IoApicMmio, LocalApicMmio};

use crate::ioapic_wire;

/// Advance the HPET main counter and mirror [`HpetMmio::irq_line`] onto the
/// resolved Timer 0 I/O APIC GSI (default IRQ2).
///
/// Returns whether Timer 0 newly latched an interrupt event this advance.
pub fn advance_hpet(
    hpet: &mut HpetMmio,
    ioapic: &mut IoApicMmio,
    lapic: &mut LocalApicMmio,
    delta: u64,
) -> bool {
    let fired = hpet.advance_main_counter(delta);
    let _ = sync_hpet_irq_to_ioapic(hpet, ioapic, lapic);
    fired
}

/// Drive the resolved HPET Timer 0 GSI from the current device `irq_line`.
///
/// Spec: HPET 1.0a non-legacy route + 82093AA pin assert. Uses
/// [`HpetMmio::ioapic_gsi`] (capable `Tn_INT_ROUTE_CNF`, else GSI 2).
pub fn sync_hpet_irq_to_ioapic(
    hpet: &HpetMmio,
    ioapic: &mut IoApicMmio,
    lapic: &mut LocalApicMmio,
) -> Option<IoApicDelivery> {
    let gsi = hpet.ioapic_gsi();
    let high = hpet.irq_line();
    ioapic_wire::assert_ioapic_gsi(ioapic, lapic, gsi, high)
}

/// Advance the HPET main counter, then sync Timer 0 IRQ onto the I/O APIC.
///
/// Returns a Fixed [`IoApicDelivery`] when the RTE accepts the pin transition.
pub fn advance_hpet_ioapic(
    hpet: &mut HpetMmio,
    ioapic: &mut IoApicMmio,
    lapic: &mut LocalApicMmio,
    delta: u64,
) -> Option<IoApicDelivery> {
    let _ = hpet.advance_main_counter(delta);
    sync_hpet_irq_to_ioapic(hpet, ioapic, lapic)
}
