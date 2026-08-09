//! Intel APM / SMI command and status ports (`0xB2` / `0xB3`).
//!
//! Spec:
//! - Intel PCH / ICH fixed I/O — APM_CNT (Advanced Power Management Control)
//!   at `B2h` and APM_STS (Advanced Power Management Status) at `B3h`: both
//!   are 8-bit R/W registers, default `00h`. A write to APM_CNT stores the
//!   command byte and, when APMC_EN is set in the chipset SMI enable path,
//!   asserts SMI#. A write to APM_STS does **not** raise SMI#; the status
//!   register is a software scratchpad between the OS/firmware and the SMI
//!   handler (APM BIOS 1.2 interface / PIIX APM ports).
//! - SeaBIOS `smm_relocate_and_restore` (emulator SMM bring-up):
//!   `outb(0x01, PORT_SMI_STATUS); outb(0x00, PORT_SMI_CMD);` then polls
//!   `while (inb(PORT_SMI_STATUS) != 0)` until the SMI handler clears status.
//!
//! This model stores both registers and, on every APM_CNT write, **stub-
//! completes** the handshake by clearing APM_STS to `0x00` as a real SMI
//! handler would. It does **not** enter System Management Mode, relocate
//! SMBASE, or deliver an architectural SMI.

use crate::PortDevice;

/// APM control / SMI command port (APM_CNT).
pub const APM_CNT_PORT: u16 = 0xB2;

/// APM status / scratchpad port (APM_STS).
pub const APM_STS_PORT: u16 = 0xB3;

/// Power-on / reset default for both registers.
const APM_DEFAULT: u8 = 0x00;

/// Intel APM_CNT / APM_STS fixed I/O pair with a completion stub for firmware
/// that polls status after a command write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApmSmi {
    /// APM_CNT — last command byte written to `0xB2`.
    cnt: u8,
    /// APM_STS — scratchpad at `0xB3` (cleared by the completion stub).
    sts: u8,
    /// Number of APM_CNT writes that ran the completion stub (SMI not delivered).
    stub_completions: u64,
}

impl Default for ApmSmi {
    fn default() -> Self {
        Self::new()
    }
}

impl ApmSmi {
    pub fn new() -> Self {
        Self {
            cnt: APM_DEFAULT,
            sts: APM_DEFAULT,
            stub_completions: 0,
        }
    }

    pub fn reset(&mut self) {
        self.cnt = APM_DEFAULT;
        self.sts = APM_DEFAULT;
        self.stub_completions = 0;
    }

    /// Whether this device claims `port` (byte ports `0xB2` and `0xB3` only).
    pub fn owns_port(port: u16) -> bool {
        port == APM_CNT_PORT || port == APM_STS_PORT
    }

    pub fn command(&self) -> u8 {
        self.cnt
    }

    pub fn status(&self) -> u8 {
        self.sts
    }

    /// How many APM_CNT writes were stub-completed without entering SMM.
    pub fn stub_completions(&self) -> u64 {
        self.stub_completions
    }

    fn write_cnt(&mut self, value: u8) {
        self.cnt = value;
        // Spec: a write to APM_CNT raises SMI# when APMC_EN is set; the SMI
        // handler is what clears APM_STS for SeaBIOS's poll. Without SMM this
        // stub completes that handshake immediately and records the gap.
        self.sts = 0x00;
        self.stub_completions = self.stub_completions.saturating_add(1);
    }

    fn write_sts(&mut self, value: u8) {
        // Spec: APM_STS is a software scratchpad; writes do not raise SMI#.
        self.sts = value;
    }
}

impl PortDevice for ApmSmi {
    fn port_read(&mut self, port: u16, _size: u8) -> u32 {
        match port {
            APM_CNT_PORT => u32::from(self.cnt),
            APM_STS_PORT => u32::from(self.sts),
            _ => 0xFFFF_FFFF,
        }
    }

    fn port_write(&mut self, port: u16, _size: u8, value: u32) {
        let byte = value as u8;
        match port {
            APM_CNT_PORT => self.write_cnt(byte),
            APM_STS_PORT => self.write_sts(byte),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spec: PCH APM_CNT/APM_STS default `00h`.
    #[test]
    fn reset_defaults_both_ports_zero() {
        let mut apm = ApmSmi::new();
        assert_eq!(apm.port_read(APM_CNT_PORT, 1), 0);
        assert_eq!(apm.port_read(APM_STS_PORT, 1), 0);
        assert_eq!(apm.stub_completions(), 0);
    }

    /// Spec: APM_STS is a scratchpad; writing it does not clear itself or raise SMI.
    #[test]
    fn status_scratchpad_stores_without_completion() {
        let mut apm = ApmSmi::new();
        apm.port_write(APM_STS_PORT, 1, 0x01);
        assert_eq!(apm.status(), 0x01);
        assert_eq!(apm.port_read(APM_STS_PORT, 1), 0x01);
        assert_eq!(apm.stub_completions(), 0);
        assert_eq!(apm.command(), 0);
    }

    /// Spec: APM_CNT stores the command; this stub then clears APM_STS as the
    /// SMI handler would (SeaBIOS `smm_relocate_and_restore` poll).
    #[test]
    fn command_write_stores_and_clears_status() {
        let mut apm = ApmSmi::new();
        apm.port_write(APM_STS_PORT, 1, 0x01);
        apm.port_write(APM_CNT_PORT, 1, 0x00);
        assert_eq!(apm.command(), 0x00);
        assert_eq!(apm.status(), 0x00);
        assert_eq!(apm.stub_completions(), 1);
        assert_eq!(apm.port_read(APM_STS_PORT, 1), 0x00);
    }

    /// SeaBIOS SMM bring-up sequence must leave the status poll satisfied.
    #[test]
    fn seabios_smm_handshake_poll_completes() {
        let mut apm = ApmSmi::new();
        // smm_relocate_and_restore:
        apm.port_write(APM_STS_PORT, 1, 0x01);
        apm.port_write(APM_CNT_PORT, 1, 0x00);
        let mut spins = 0u32;
        while apm.port_read(APM_STS_PORT, 1) as u8 != 0 {
            spins += 1;
            assert!(spins < 4, "stub must clear status on the command write");
        }
        assert_eq!(spins, 0);
        assert_eq!(apm.stub_completions(), 1);
    }

    #[test]
    fn owns_only_b2_and_b3() {
        assert!(ApmSmi::owns_port(APM_CNT_PORT));
        assert!(ApmSmi::owns_port(APM_STS_PORT));
        assert!(!ApmSmi::owns_port(0xB1));
        assert!(!ApmSmi::owns_port(0xB4));
    }

    #[test]
    fn reset_clears_latched_state_and_completion_count() {
        let mut apm = ApmSmi::new();
        apm.port_write(APM_STS_PORT, 1, 0x5A);
        apm.port_write(APM_CNT_PORT, 1, 0x12);
        assert_eq!(apm.command(), 0x12);
        assert_eq!(apm.stub_completions(), 1);
        apm.reset();
        assert_eq!(apm, ApmSmi::new());
    }
}
