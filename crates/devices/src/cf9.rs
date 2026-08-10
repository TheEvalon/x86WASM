//! ICH / PIIX Reset Control Register at I/O port `0xCF9`.
//!
//! Spec: Intel ICH / 82371 family Reset Control Register (RCR) at `CF9h`;
//! SeaBIOS `qemu_reboot` / `pci_reboot` (`PORT_PCI_REBOOT`) writes `0x02` then
//! `0x06` to request a system reset. PCI 3.0 §3.2.2.3.2: only a full DWORD at
//! `0xCF8` is CONFIG_ADDRESS — a byte/word access at `0xCF9` is ordinary I/O,
//! which this device claims.
//!
//! Bits (ICH RCR subset modeled here):
//! - bit1 `SYS_RST`: system-reset type (0 = soft / CPU-only style, 1 = hard)
//! - bit2 `RST_CPU`: initiate reset when written 1
//!
//! SeaBIOS sequence `outb(0x02); outb(0x06)` sets SYS_RST then pulses RST_CPU
//! with SYS_RST still set → hard reset. This stub latches
//! [`Self::take_system_reset_request`] when bit2 is written 1.

use crate::PortDevice;

/// ICH Reset Control Register (byte port overlapping PCI CONFIG_ADDRESS +1).
pub const PORT_RESET_CTRL: u16 = 0xCF9;

/// Bit1: system-reset type (1 = full / hard when RST_CPU pulses).
pub const CF9_SYS_RST: u8 = 1 << 1;

/// Bit2: write-1 initiates CPU/system reset.
pub const CF9_RST_CPU: u8 = 1 << 2;

/// Power-on / after-reset default (no pending pulse).
const DEFAULT_VALUE: u8 = 0;

/// ICH Reset Control Register at `0xCF9`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cf9Reset {
    /// Latched register bits (RST_CPU is write-pulse, not sticky on readback).
    value: u8,
    /// Set when a write has bit2 (`RST_CPU`) set.
    system_reset_pending: bool,
    /// Host-visible count of RST_CPU pulses (diagnostic / POST probe).
    reset_pulse_count: u64,
}

impl Default for Cf9Reset {
    fn default() -> Self {
        Self::new()
    }
}

impl Cf9Reset {
    pub fn new() -> Self {
        Self {
            value: DEFAULT_VALUE,
            system_reset_pending: false,
            reset_pulse_count: 0,
        }
    }

    pub fn reset(&mut self) {
        self.value = DEFAULT_VALUE;
        self.system_reset_pending = false;
        // Pulse count is host diagnostic state across guest resets.
    }

    /// Take a latched reset request (RST_CPU write-1).
    ///
    /// Returns `true` once per pulse; the machine layer should then run
    /// [`crate`]-side `Machine::reset` (same path as 8042 `0xFE` / port `0x92`).
    pub fn take_system_reset_request(&mut self) -> bool {
        let pending = self.system_reset_pending;
        self.system_reset_pending = false;
        pending
    }

    /// Current register value (RST_CPU always reads 0 — pulse is not sticky).
    pub fn value(&self) -> u8 {
        self.value
    }

    /// Number of RST_CPU pulses observed since construction (survives [`Self::reset`]).
    pub fn reset_pulse_count(&self) -> u64 {
        self.reset_pulse_count
    }

    /// Whether this port/size pair is the ordinary I/O RCR (not PCI CONFIG_ADDRESS).
    ///
    /// Spec: PCI 3.0 §3.2.2.3.2 — only a full DWORD at `0xCF8` latches
    /// CONFIG_ADDRESS; byte/word at `0xCF9` are ordinary I/O.
    pub fn owns_access(port: u16, size: u8) -> bool {
        port == PORT_RESET_CTRL && size != 4
    }

    fn write_u8(&mut self, value: u8) {
        if value & CF9_RST_CPU != 0 {
            self.system_reset_pending = true;
            self.reset_pulse_count = self.reset_pulse_count.saturating_add(1);
        }
        // RST_CPU is a write pulse; SYS_RST and other bits store for RMW.
        self.value = value & !CF9_RST_CPU;
    }
}

impl PortDevice for Cf9Reset {
    fn port_read(&mut self, port: u16, size: u8) -> u32 {
        if !Self::owns_access(port, size) {
            return 0xFFFF_FFFF;
        }
        u32::from(self.value)
    }

    fn port_write(&mut self, port: u16, size: u8, value: u32) {
        if !Self::owns_access(port, size) {
            return;
        }
        self.write_u8(value as u8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spec: ICH RCR — reset default is clear; no pending pulse.
    #[test]
    fn reset_default_clear() {
        let p = Cf9Reset::new();
        assert_eq!(p.value(), 0);
        assert_eq!(p.reset_pulse_count(), 0);
    }

    /// Spec: SeaBIOS `qemu_reboot` — `outb(0x02)` stores SYS_RST without reset.
    #[test]
    fn write_02_stores_sys_rst_without_reset() {
        let mut p = Cf9Reset::new();
        p.port_write(PORT_RESET_CTRL, 1, u32::from(CF9_SYS_RST));
        assert_eq!(p.value(), CF9_SYS_RST);
        assert!(!p.take_system_reset_request());
        assert_eq!(p.reset_pulse_count(), 0);
    }

    /// Spec: SeaBIOS `qemu_reboot` — `outb(0x06)` = SYS_RST|RST_CPU pulses hard reset.
    #[test]
    fn write_06_latches_system_reset_pulse() {
        let mut p = Cf9Reset::new();
        p.port_write(PORT_RESET_CTRL, 1, u32::from(CF9_SYS_RST));
        p.port_write(PORT_RESET_CTRL, 1, u32::from(CF9_SYS_RST | CF9_RST_CPU));
        assert_eq!(p.value() & CF9_RST_CPU, 0, "RST_CPU is not sticky");
        assert_eq!(p.value() & CF9_SYS_RST, CF9_SYS_RST);
        assert_eq!(p.reset_pulse_count(), 1);
        assert!(p.take_system_reset_request());
        assert!(!p.take_system_reset_request());
    }

    /// Spec: PCI 3.0 §3.2.2.3.2 — DWORD at CF9 is not this register.
    #[test]
    fn dword_access_is_not_owned() {
        assert!(!Cf9Reset::owns_access(PORT_RESET_CTRL, 4));
        let mut p = Cf9Reset::new();
        p.port_write(PORT_RESET_CTRL, 4, 0xDEAD_BEEF);
        assert_eq!(p.value(), 0);
        assert!(!p.take_system_reset_request());
        assert_eq!(p.port_read(PORT_RESET_CTRL, 4), 0xFFFF_FFFF);
    }

    #[test]
    fn reset_clears_pending_keeps_pulse_count() {
        let mut p = Cf9Reset::new();
        p.port_write(PORT_RESET_CTRL, 1, u32::from(CF9_RST_CPU));
        assert_eq!(p.reset_pulse_count(), 1);
        p.reset();
        assert_eq!(p.value(), 0);
        assert!(!p.take_system_reset_request());
        assert_eq!(p.reset_pulse_count(), 1);
    }
}
