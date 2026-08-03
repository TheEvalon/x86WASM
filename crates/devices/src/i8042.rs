//! IBM PC AT 8042 / PS/2 controller register bank (ports `0x60` / `0x64`).
//!
//! # Spec refs
//!
//! - OSDev Wiki: [I8042 PS/2 Controller](https://wiki.osdev.org/I8042_PS/2_Controller)
//!   — data/status/command ports, status OBF/IBF, controller self-test `0xAA`→`0x55`,
//!   configuration byte via `0x20`/`0x60`, disable/enable first port `0xAD`/`0xAE`.
//! - IBM PC/AT 8042 keyboard-controller programming model (command/status/data).
//! - `docs/sources.md` (PS/2 and 8042 references), `docs/machine-model-pc-v1.md`,
//!   `plan.md` §15.4.
//!
//! # Scope (this slice)
//!
//! Register bank wired onto `machine-pc::MachineBus` at ports `0x60`/`0x64`:
//! status bits useful for firmware polling, a small documented command subset,
//! and an output-buffer data path. Instant command completion (IBF never stays
//! set across a status poll). No IRQ1 delivery.
//!
//! # Unsupported (explicit)
//!
//! - IRQ1 / IRQ12 delivery
//! - PS/2 keyboard or mouse device protocol (no scancodes, no `0xFA` ACK)
//! - Second PS/2 port (`0xA7`/`0xA8`/`0xA9`/`0xD4`)
//! - A20 gate / output-port / pulse-reset (`0xD0`/`0xD1`/`0xFE`, …) — accepted
//!   as documented no-ops where noted; no A20 side effects
//! - Interface test `0xAB`, diagnostic dump `0xAC`

use crate::PortDevice;

/// Keyboard/controller data port (classic PC AT).
pub const I8042_DATA: u16 = 0x60;
/// Status (read) / command (write) port.
pub const I8042_STATUS_CMD: u16 = 0x64;

/// Status bit 0: output buffer full (data available at `0x60`).
pub const STATUS_OBF: u8 = 1 << 0;
/// Status bit 1: input buffer full (host must wait before writing).
pub const STATUS_IBF: u8 = 1 << 1;
/// Status bit 2: system flag (mirrors configuration byte bit 2 after POST).
pub const STATUS_SYS: u8 = 1 << 2;
/// Status bit 3: last write was command (`0x64`) rather than data (`0x60`).
pub const STATUS_CMD: u8 = 1 << 3;

/// Controller command: read configuration byte → response on data port.
pub const CMD_READ_CONFIG: u8 = 0x20;
/// Controller command: write next data-port byte to configuration byte.
pub const CMD_WRITE_CONFIG: u8 = 0x60;
/// Controller command: self-test; success response `0x55`.
pub const CMD_SELF_TEST: u8 = 0xAA;
/// Controller command: disable first PS/2 port (keyboard clock inhibit).
pub const CMD_DISABLE_KBD: u8 = 0xAD;
/// Controller command: enable first PS/2 port.
pub const CMD_ENABLE_KBD: u8 = 0xAE;

/// Self-test passed response (OSDev / IBM AT).
pub const SELF_TEST_OK: u8 = 0x55;

/// Configuration bit 4: first PS/2 port clock disabled when set.
const CFG_KBD_CLOCK_DISABLE: u8 = 1 << 4;
/// Configuration bit 6: first PS/2 port translation enabled when set.
const CFG_TRANSLATE: u8 = 1 << 6;

/// Reset default configuration: keyboard clock disabled, translation on.
/// IRQ enables clear (honest: no IRQ delivery in this slice).
const DEFAULT_CONFIG: u8 = CFG_KBD_CLOCK_DISABLE | CFG_TRANSLATE; // 0x50

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingWrite {
    None,
    /// Next `0x60` write updates the controller configuration byte (`0x60` cmd).
    ConfigByte,
    /// Next `0x60` write is a no-op payload for an unsupported two-byte command
    /// (e.g. A20 / output-port write `0xD1`).
    DiscardData,
}

/// Minimal IBM PC AT 8042-compatible controller state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct I8042 {
    /// Controller configuration byte (RAM byte 0; commands `0x20` / `0x60`).
    pub config: u8,
    /// One-byte output buffer (device/controller → host).
    output: Option<u8>,
    /// Status bit 3: last host write targeted the command port.
    last_write_was_cmd: bool,
    pending: PendingWrite,
    /// Counts of unsupported command bytes seen (for tests / diagnostics).
    pub unsupported_commands: u32,
}

impl I8042 {
    pub fn new() -> Self {
        let mut s = Self {
            config: DEFAULT_CONFIG,
            output: None,
            last_write_was_cmd: false,
            pending: PendingWrite::None,
            unsupported_commands: 0,
        };
        s.apply_reset_defaults();
        s
    }

    fn apply_reset_defaults(&mut self) {
        self.config = DEFAULT_CONFIG;
        self.output = None;
        self.last_write_was_cmd = false;
        self.pending = PendingWrite::None;
        self.unsupported_commands = 0;
    }

    pub fn reset(&mut self) {
        self.apply_reset_defaults();
    }

    /// Status register value (port `0x64` read).
    pub fn status(&self) -> u8 {
        let mut s = 0u8;
        if self.output.is_some() {
            s |= STATUS_OBF;
        }
        // Instant completion: IBF is never sticky across a status poll.
        if self.config & STATUS_SYS != 0 {
            // Config bit 2 is the system flag; mirror into status bit 2.
            s |= STATUS_SYS;
        }
        if self.last_write_was_cmd {
            s |= STATUS_CMD;
        }
        s
    }

    pub fn output_buffer(&self) -> Option<u8> {
        self.output
    }

    pub fn keyboard_clock_disabled(&self) -> bool {
        self.config & CFG_KBD_CLOCK_DISABLE != 0
    }

    fn push_output(&mut self, value: u8) {
        self.output = Some(value);
    }

    fn handle_command(&mut self, cmd: u8) {
        match cmd {
            CMD_READ_CONFIG => {
                self.push_output(self.config);
            }
            CMD_WRITE_CONFIG => {
                self.pending = PendingWrite::ConfigByte;
            }
            CMD_SELF_TEST => {
                // Success response; keep config (firmware may re-read/write it).
                // Documented side effects of a full KBC reset are not modeled.
                self.push_output(SELF_TEST_OK);
            }
            CMD_DISABLE_KBD => {
                self.config |= CFG_KBD_CLOCK_DISABLE;
            }
            CMD_ENABLE_KBD => {
                self.config &= !CFG_KBD_CLOCK_DISABLE;
            }
            // A20 / output-port write: accept command; next data byte discarded.
            0xD1 => {
                self.pending = PendingWrite::DiscardData;
                self.unsupported_commands = self.unsupported_commands.saturating_add(1);
            }
            _ => {
                self.unsupported_commands = self.unsupported_commands.saturating_add(1);
            }
        }
    }
}

impl Default for I8042 {
    fn default() -> Self {
        Self::new()
    }
}

impl PortDevice for I8042 {
    fn port_read(&mut self, port: u16, _size: u8) -> u32 {
        match port {
            I8042_DATA => {
                let v = self.output.take().unwrap_or(0);
                u32::from(v)
            }
            I8042_STATUS_CMD => u32::from(self.status()),
            _ => 0xFFFF_FFFF,
        }
    }

    fn port_write(&mut self, port: u16, _size: u8, value: u32) {
        let v = value as u8;
        match port {
            I8042_DATA => {
                self.last_write_was_cmd = false;
                match self.pending {
                    PendingWrite::ConfigByte => {
                        self.config = v;
                        self.pending = PendingWrite::None;
                    }
                    PendingWrite::DiscardData => {
                        // No-op payload (e.g. A20 bit in output-port write).
                        self.pending = PendingWrite::None;
                    }
                    PendingWrite::None => {
                        // Keyboard data path: accept and drop (no device ACK).
                    }
                }
            }
            I8042_STATUS_CMD => {
                self.last_write_was_cmd = true;
                self.handle_command(v);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spec: reset leaves OBF/IBF clear; default config disables keyboard clock.
    #[test]
    fn reset_state() {
        let k = I8042::new();
        assert_eq!(k.status() & (STATUS_OBF | STATUS_IBF), 0);
        assert!(k.keyboard_clock_disabled());
        assert_eq!(k.config, DEFAULT_CONFIG);
        assert_eq!(k.output_buffer(), None);

        let mut k2 = I8042::new();
        k2.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_SELF_TEST));
        k2.reset();
        assert_eq!(k2, I8042::new());
    }

    /// Spec: OSDev / IBM AT — command `0xAA` places `0x55` in the output buffer.
    #[test]
    fn self_test_aa_returns_55() {
        let mut k = I8042::new();
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_SELF_TEST));
        assert_ne!(k.status() & STATUS_OBF, 0);
        assert_eq!(k.status() & STATUS_IBF, 0);
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, SELF_TEST_OK);
        assert_eq!(k.status() & STATUS_OBF, 0);
    }

    /// Spec: commands `0x20` / `0x60` read/write the configuration byte.
    #[test]
    fn read_write_command_byte() {
        let mut k = I8042::new();
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_READ_CONFIG));
        assert_ne!(k.status() & STATUS_OBF, 0);
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, DEFAULT_CONFIG);

        let new_cfg = 0x47u8; // IRQ1 enable + system flag + translate (example)
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_CONFIG));
        k.port_write(I8042_DATA, 1, u32::from(new_cfg));
        assert_eq!(k.config, new_cfg);

        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_READ_CONFIG));
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, new_cfg);
        // System flag mirrored into status bit 2 when config bit 2 is set.
        assert_ne!(k.status() & STATUS_SYS, 0);
    }

    /// Spec: `0xAD` / `0xAE` disable / enable first PS/2 port (config bit 4).
    #[test]
    fn disable_enable_keyboard() {
        let mut k = I8042::new();
        assert!(k.keyboard_clock_disabled());
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_ENABLE_KBD));
        assert!(!k.keyboard_clock_disabled());
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_DISABLE_KBD));
        assert!(k.keyboard_clock_disabled());
    }

    #[test]
    fn data_read_pops_output_buffer() {
        let mut k = I8042::new();
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_SELF_TEST));
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, SELF_TEST_OK);
        // Empty buffer: OBF clear; read returns 0.
        assert_eq!(k.status() & STATUS_OBF, 0);
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, 0);
    }

    #[test]
    fn data_write_without_pending_is_ignored() {
        let mut k = I8042::new();
        let before = k.clone();
        k.port_write(I8042_DATA, 1, 0xED); // typical LED command — no device
        assert_eq!(k.config, before.config);
        assert_eq!(k.output_buffer(), None);
        assert_eq!(k.status() & STATUS_OBF, 0);
    }

    #[test]
    fn status_cmd_bit_tracks_last_write_port() {
        let mut k = I8042::new();
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_DISABLE_KBD));
        assert_ne!(k.status() & STATUS_CMD, 0);
        k.port_write(I8042_DATA, 1, 0x00);
        assert_eq!(k.status() & STATUS_CMD, 0);
    }

    /// A20 / output-port write `0xD1`: command accepted; data byte discarded; no A20 effect.
    #[test]
    fn a20_write_output_port_is_documented_noop() {
        let mut k = I8042::new();
        let cfg = k.config;
        k.port_write(I8042_STATUS_CMD, 1, 0xD1);
        k.port_write(I8042_DATA, 1, 0xDF); // classic "A20 on" payload on real HW
        assert_eq!(k.config, cfg);
        assert!(k.unsupported_commands >= 1);
        // Still accepts a subsequent real command.
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_SELF_TEST));
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, SELF_TEST_OK);
    }

    #[test]
    fn state_clone_equality_round_trip() {
        let mut k = I8042::new();
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_ENABLE_KBD));
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_CONFIG));
        k.port_write(I8042_DATA, 1, 0x45);
        let cloned = k.clone();
        assert_eq!(k, cloned);
        assert_eq!(cloned.config, 0x45);
    }

    #[test]
    fn unrelated_ports_ignored() {
        let mut k = I8042::new();
        k.port_write(0x3F8, 1, 0x10);
        assert_eq!(k, I8042::new());
        assert_eq!(k.port_read(0x3F8, 1), 0xFFFF_FFFF);
    }
}
