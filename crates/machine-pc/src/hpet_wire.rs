//! Host helpers for Round-7 HPET comparator IRQ stub.
//!
//! Kept out of the `Machine` monolith so parallel lanes can merge without
//! rewriting MMIO dispatch. Delivery onto PIC / I/O APIC is intentionally
//! **not** performed here — see `docs/hpet-r7-comparator-irq.md`.

use devices::HpetMmio;

/// Advance the HPET main counter and return whether Timer 0 newly requested IRQ.
pub fn advance_hpet(hpet: &mut HpetMmio, delta: u64) -> bool {
    hpet.advance_main_counter(delta)
}
