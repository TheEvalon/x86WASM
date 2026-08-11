//! Host helpers for HPET Timer 0 → I/O APIC GSI delivery (R10/R11).
//!
//! Kept out of the `Machine` monolith so parallel lanes can merge without
//! rewriting MMIO dispatch. Legacy PIC replacement stays an explicit non-claim
//! (`LEG_RT_CAP` clear — `docs/timer-r14-hpet-legacy.md`); FSB/MSI remains out
//! (`docs/hpet-r12-msi-irq.md`).
//!
//! Ownership (R10/R11/R14 usb-timer): this module + thin `Machine::advance_hpet` /
//! `Machine::sync_hpet_irq_to_ioapic` / `Machine::advance_hpet_ioapic` /
//! `Machine::eoi_lapic_ioapic` (HPET level re-sync) wrappers.

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

/// Local APIC EOI + I/O APIC Remote IRR clear, then re-drive HPET level IRQ.
///
/// Spec: 82093AA — after EOI clears Remote IRR, a still-asserted level pin
/// must be allowed to re-deliver. When HPET `Tn_INT_STS` remains set (level
/// mode, status not yet W1C), [`sync_hpet_irq_to_ioapic`] re-asserts the GSI.
pub fn eoi_lapic_ioapic_resync_hpet(
    hpet: &HpetMmio,
    ioapic: &mut IoApicMmio,
    lapic: &mut LocalApicMmio,
) -> Option<(u8, Option<IoApicDelivery>)> {
    let vec = ioapic_wire::eoi_lapic_and_ioapic(ioapic, lapic)?;
    let redlivery = sync_hpet_irq_to_ioapic(hpet, ioapic, lapic);
    Some((vec, redlivery))
}
