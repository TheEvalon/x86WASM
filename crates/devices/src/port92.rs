//! System Control Port A (`0x92`) — Fast Gate A20 + optional fast reset pulse.
//!
//! Spec: OSDev Wiki "A20 Line" (Fast A20 Gate) + IBM PS/2 System Control Port A:
//! - bit0 write-1: fast system reset pulse
//! - bit1 R/W: A20 gate (0 = masked / disabled, 1 = enabled)
//!
//! Other bits are stored for RMW (firmware often `IN`/`OR`/`AND`/`OUT`) without
//! side effects in this slice.

use crate::PortDevice;

/// System Control Port A (Fast A20 / fast reset).
pub const PORT_SYSTEM_CONTROL_A: u16 = 0x92;

/// Bit0: write-1 pulses a fast system reset.
pub const PORT92_RESET: u8 = 1 << 0;

/// Bit1: Fast Gate A20 (1 = enabled / unmasked).
pub const PORT92_A20: u8 = 1 << 1;

/// Power-on / reset default: A20 enabled, reset pulse clear.
const DEFAULT_VALUE: u8 = PORT92_A20;

/// IBM PS/2 System Control Port A (`0x92`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Port92 {
    /// Latched register; bit0 is never sticky (pulse-only).
    value: u8,
    /// Latched when the host writes bit0 = 1.
    ///
    /// Spec: OSDev A20 Line / IBM PS/2 — fast reset. Cleared by
    /// [`Self::take_system_reset_request`] or [`Self::reset`]. Same machine-layer
    /// pattern as [`crate::I8042::take_system_reset_request`].
    system_reset_pending: bool,
}

impl Default for Port92 {
    fn default() -> Self {
        Self::new()
    }
}

impl Port92 {
    pub fn new() -> Self {
        Self {
            value: DEFAULT_VALUE,
            system_reset_pending: false,
        }
    }

    pub fn reset(&mut self) {
        self.value = DEFAULT_VALUE;
        self.system_reset_pending = false;
    }

    /// Take a latched fast-reset request (bit0 write-1).
    ///
    /// Returns `true` once per pulse; the machine layer should then run
    /// `Machine::reset` (same path as 8042 pulse-reset `0xFE`).
    pub fn take_system_reset_request(&mut self) -> bool {
        let pending = self.system_reset_pending;
        self.system_reset_pending = false;
        pending
    }

    /// A20 gate from bit1 (1 = enabled / unmasked).
    pub fn a20_enabled(&self) -> bool {
        self.value & PORT92_A20 != 0
    }

    /// Mirror A20 into bit1 without touching other stored bits or pulsing reset.
    ///
    /// Used by `MachineBus` when the 8042 output-port A20 bit changes so both
    /// gates stay coordinated.
    pub fn set_a20_enabled(&mut self, enabled: bool) {
        if enabled {
            self.value |= PORT92_A20;
        } else {
            self.value &= !PORT92_A20;
        }
    }

    /// Current register value (bit0 always reads as 0 — pulse is not sticky).
    pub fn value(&self) -> u8 {
        self.value
    }

    fn write_u8(&mut self, value: u8) {
        if value & PORT92_RESET != 0 {
            self.system_reset_pending = true;
        }
        // Spec: OSDev A20 Line — bit0 is fast reset; guests clear it on RMW
        // (`AND 0xFE`) so it is modeled as write-pulse, not sticky.
        self.value = value & !PORT92_RESET;
    }
}

impl PortDevice for Port92 {
    fn port_read(&mut self, port: u16, _size: u8) -> u32 {
        if port != PORT_SYSTEM_CONTROL_A {
            return 0xFFFF_FFFF;
        }
        u32::from(self.value)
    }

    fn port_write(&mut self, port: u16, _size: u8, value: u32) {
        if port != PORT_SYSTEM_CONTROL_A {
            return;
        }
        self.write_u8(value as u8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spec: OSDev A20 Line — Fast A20 Gate is port `0x92` bit1.
    #[test]
    fn reset_default_a20_enabled_bit0_clear() {
        let p = Port92::new();
        assert_eq!(p.value(), DEFAULT_VALUE);
        assert!(p.a20_enabled());
        assert_eq!(p.value() & PORT92_RESET, 0);
    }

    /// Spec: OSDev A20 Line — writing bit1 clear disables A20; bit1 set enables.
    #[test]
    fn bit1_toggles_a20_store_and_readback() {
        let mut p = Port92::new();
        p.port_write(PORT_SYSTEM_CONTROL_A, 1, 0x00);
        assert!(!p.a20_enabled());
        assert_eq!(p.port_read(PORT_SYSTEM_CONTROL_A, 1) as u8, 0x00);

        p.port_write(PORT_SYSTEM_CONTROL_A, 1, u32::from(PORT92_A20));
        assert!(p.a20_enabled());
        assert_eq!(p.port_read(PORT_SYSTEM_CONTROL_A, 1) as u8, PORT92_A20);
    }

    /// Spec: OSDev A20 Line / IBM PS/2 — bit0 write-1 pulses fast reset; bit0
    /// is not sticky on readback.
    #[test]
    fn bit0_write_latches_system_reset_not_sticky() {
        let mut p = Port92::new();
        assert!(!p.take_system_reset_request());

        p.port_write(
            PORT_SYSTEM_CONTROL_A,
            1,
            u32::from(PORT92_RESET | PORT92_A20),
        );
        assert!(p.a20_enabled());
        assert_eq!(p.value() & PORT92_RESET, 0);
        assert!(p.take_system_reset_request());
        assert!(!p.take_system_reset_request());
    }

    /// Spec: OSDev A20 Line — writing with bit0 clear does not reset.
    #[test]
    fn bit0_clear_does_not_latch_system_reset() {
        let mut p = Port92::new();
        p.port_write(PORT_SYSTEM_CONTROL_A, 1, 0x00);
        assert!(!p.take_system_reset_request());
        p.port_write(PORT_SYSTEM_CONTROL_A, 1, u32::from(PORT92_A20));
        assert!(!p.take_system_reset_request());
    }

    #[test]
    fn set_a20_enabled_mirrors_without_reset_pulse() {
        let mut p = Port92::new();
        p.set_a20_enabled(false);
        assert!(!p.a20_enabled());
        assert!(!p.take_system_reset_request());
        p.set_a20_enabled(true);
        assert!(p.a20_enabled());
        assert!(!p.take_system_reset_request());
    }

    #[test]
    fn reset_restores_default_and_clears_pending() {
        let mut p = Port92::new();
        p.port_write(PORT_SYSTEM_CONTROL_A, 1, 0x00); // A20 off
        p.port_write(PORT_SYSTEM_CONTROL_A, 1, u32::from(PORT92_RESET));
        p.reset();
        assert_eq!(p.value(), DEFAULT_VALUE);
        assert!(p.a20_enabled());
        assert!(!p.take_system_reset_request());
    }

    /// RMW preserves non-modeled bits (password/LED stubs) except bit0 pulse.
    #[test]
    fn other_bits_store_for_rmw() {
        let mut p = Port92::new();
        p.port_write(PORT_SYSTEM_CONTROL_A, 1, 0xC2); // bits 7:6 + A20
        assert_eq!(p.port_read(PORT_SYSTEM_CONTROL_A, 1) as u8, 0xC2);
        assert!(p.a20_enabled());
    }
}
