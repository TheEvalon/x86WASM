//! Host helpers for Round-7 Local APIC timer stub.
//!
//! Isolated from the `Machine` MMIO monolith. Does **not** inject into the CPU
//! interpreter — see `docs/lapic-r7-timer-lvt.md`.

use devices::LocalApicMmio;

/// Advance the Local APIC timer; returns whether a local IRQ was newly latched.
pub fn tick_lapic_timer(lapic: &mut LocalApicMmio, bus_clocks: u64) -> bool {
    lapic.tick_timer(bus_clocks)
}
