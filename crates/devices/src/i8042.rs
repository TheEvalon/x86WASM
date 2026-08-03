//! IBM PC AT 8042 / PS/2 controller register bank (ports `0x60` / `0x64`).
//!
//! # Spec refs
//!
//! - OSDev Wiki: [I8042 PS/2 Controller](https://wiki.osdev.org/I8042_PS/2_Controller)
//!   — data/status/command ports, status OBF/IBF, controller self-test `0xAA`→`0x55`,
//!   configuration byte via `0x20`/`0x60` (bit0 = first-port IRQ enable → IRQ1),
//!   disable/enable first port `0xAD`/`0xAE`.
//! - IBM PC/AT 8042 keyboard-controller programming model (command/status/data);
//!   output-buffer-full with IRQ enable → ISA IRQ1 (8259A master IR1).
//! - `docs/sources.md` (PS/2 and 8042 references), `docs/machine-model-pc-v1.md`,
//!   `plan.md` §15.4.
//!
//! # Scope (this slice)
//!
//! Register bank wired onto `machine-pc::MachineBus` at ports `0x60`/`0x64`:
//! status bits useful for firmware polling, a small documented command subset,
//! output-buffer data path, IRQ1 when config bit0 is set and OBF is set, and a
//! make-code inject stub (`inject_scancode`) that respects keyboard clock disable.
//! Instant command completion (IBF never stays set across a status poll).
//!
//! # Unsupported (explicit)
//!
//! - IRQ12 / second PS/2 port (`0xA7`/`0xA8`/`0xA9`/`0xD4`)
//! - Full AT keyboard protocol (no host→device commands, no `0xFA` ACK, no break codes)
//! - Set2↔Set1 translation table (config bit6 is stored; inject is passthrough)
//! - Mouse device / pulse-reset lines (`0xFE` / output-port bit0 system-reset)
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
/// Controller command: read output port → response on data port.
pub const CMD_READ_OUTPUT_PORT: u8 = 0xD0;
/// Controller command: write next data-port byte to output port (A20 bit1).
pub const CMD_WRITE_OUTPUT_PORT: u8 = 0xD1;

/// Self-test passed response (OSDev / IBM AT).
pub const SELF_TEST_OK: u8 = 0x55;

/// Output-port bit 1: A20 gate enable (1 = A20 line high / unmasked).
pub const OUTPUT_PORT_A20: u8 = 1 << 1;
/// Power-on / reset default output port: A20 enabled (classic AT open gate).
/// Other bits are stored but not claimed (system-reset / clock / IRQ lines).
const DEFAULT_OUTPUT_PORT: u8 = 0xDF;

/// Configuration bit 0: first PS/2 port interrupt (IRQ1) enable when set.
pub const CFG_INT1: u8 = 1 << 0;
/// Configuration bit 1: second PS/2 port interrupt (IRQ12) — not delivered here.
pub const CFG_INT12: u8 = 1 << 1;
/// Configuration bit 4: first PS/2 port clock disabled when set.
pub const CFG_KBD_CLOCK_DISABLE: u8 = 1 << 4;
/// Configuration bit 6: first PS/2 port translation enabled when set.
///
/// Stored and readable; this stub does **not** remap Set2↔Set1 bytes.
pub const CFG_TRANSLATE: u8 = 1 << 6;

/// Reset default configuration: keyboard clock disabled, translation on.
/// IRQ enables clear until firmware writes the config byte.
const DEFAULT_CONFIG: u8 = CFG_KBD_CLOCK_DISABLE | CFG_TRANSLATE; // 0x50

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingWrite {
    None,
    /// Next `0x60` write updates the controller configuration byte (`0x60` cmd).
    ConfigByte,
    /// Next `0x60` write updates the controller output port (`0xD1` cmd).
    OutputPort,
}

/// Minimal IBM PC AT 8042-compatible controller state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct I8042 {
    /// Controller configuration byte (RAM byte 0; commands `0x20` / `0x60`).
    pub config: u8,
    /// Controller output port (commands `0xD0` / `0xD1`); bit1 = A20 gate.
    pub output_port: u8,
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
            output_port: DEFAULT_OUTPUT_PORT,
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
        self.output_port = DEFAULT_OUTPUT_PORT;
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

    /// First-port interrupt enable (config bit 0).
    pub fn irq1_enabled(&self) -> bool {
        self.config & CFG_INT1 != 0
    }

    /// ISA IRQ1 line level: OBF ∧ config INT1 enable.
    ///
    /// Spec: OSDev I8042 / IBM PC AT — keyboard IRQ when output buffer full and
    /// interrupt enabled in the controller configuration byte.
    pub fn irq1_line(&self) -> bool {
        self.output.is_some() && self.irq1_enabled()
    }

    /// A20 gate from output-port bit 1 (1 = enabled / unmasked).
    ///
    /// Spec: IBM PC AT 8042 output port — bit1 gates address line A20.
    pub fn a20_enabled(&self) -> bool {
        self.output_port & OUTPUT_PORT_A20 != 0
    }

    /// Place a byte in the output buffer (device/controller → host).
    ///
    /// Used by tests and controller responses. Returns true if IRQ1 had a
    /// rising edge (false→true) as a result.
    pub fn place_output(&mut self, value: u8) -> bool {
        let prev = self.irq1_line();
        self.push_output(value);
        !prev && self.irq1_line()
    }

    /// Inject a keyboard make-code into the output buffer (device → host).
    ///
    /// Spec: OSDev I8042 / IBM PC AT — when the first-port clock is disabled
    /// (config bit4), the keyboard interface ignores device traffic. When
    /// enabled, a make-code is placed in the output buffer (OBF) and may raise
    /// IRQ1 if config INT1 is set.
    ///
    /// Translation (config bit6): passthrough stub only — no Set2↔Set1 remap.
    /// Callers should supply already-Set1 codes when translation is on (the
    /// firmware default); when translation is off the same raw byte is placed.
    ///
    /// Returns true if IRQ1 had a rising edge (same as [`Self::place_output`]).
    pub fn inject_scancode(&mut self, make_code: u8) -> bool {
        if self.keyboard_clock_disabled() {
            return false;
        }
        // Translation bit is stored only; no Set2↔Set1 table in this slice.
        self.place_output(make_code)
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
            CMD_READ_OUTPUT_PORT => {
                self.push_output(self.output_port);
            }
            CMD_WRITE_OUTPUT_PORT => {
                self.pending = PendingWrite::OutputPort;
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
                    PendingWrite::OutputPort => {
                        // Spec: IBM PC AT — output port bit1 = A20 gate.
                        self.output_port = v;
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
        assert!(!k.irq1_line());

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

    /// Spec: OBF + config INT1 → IRQ1; reading 0x60 clears OBF / IRQ1.
    #[test]
    fn place_output_with_irq_enable_asserts_irq1() {
        let mut k = I8042::new();
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_CONFIG));
        k.port_write(I8042_DATA, 1, u32::from(CFG_INT1 | CFG_TRANSLATE));
        assert!(!k.irq1_line());
        assert!(k.place_output(0x1C)); // make code 'A' (test payload)
        assert!(k.irq1_line());
        assert_ne!(k.status() & STATUS_OBF, 0);
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, 0x1C);
        assert!(!k.irq1_line());
        assert_eq!(k.status() & STATUS_OBF, 0);
    }

    /// Spec: OBF without INT1 enable does not assert IRQ1.
    #[test]
    fn place_output_without_irq_enable_no_irq1() {
        let mut k = I8042::new();
        // Default config: INT1 clear.
        assert!(!k.irq1_enabled());
        assert!(!k.place_output(0xAA));
        assert_ne!(k.status() & STATUS_OBF, 0);
        assert!(!k.irq1_line());
    }

    /// Spec: clearing INT1 while OBF set deasserts IRQ1; re-enable restores level.
    #[test]
    fn disable_int1_while_obf_deasserts_irq1() {
        let mut k = I8042::new();
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_CONFIG));
        k.port_write(I8042_DATA, 1, u32::from(CFG_INT1));
        k.place_output(0x02);
        assert!(k.irq1_line());
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_CONFIG));
        k.port_write(I8042_DATA, 1, 0x00); // INT1 off
        assert!(!k.irq1_line());
        assert_ne!(k.status() & STATUS_OBF, 0);
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_CONFIG));
        k.port_write(I8042_DATA, 1, u32::from(CFG_INT1));
        assert!(k.irq1_line());
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

    /// Spec: IBM PC AT — `0xD1` writes output port; bit1 = A20; `0xD0` reads it back.
    #[test]
    fn a20_output_port_d1_d0() {
        let mut k = I8042::new();
        assert!(k.a20_enabled());
        assert_eq!(k.output_port, DEFAULT_OUTPUT_PORT);

        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_OUTPUT_PORT));
        k.port_write(I8042_DATA, 1, 0xDD); // classic "A20 off" (bit1 clear)
        assert!(!k.a20_enabled());
        assert_eq!(k.output_port, 0xDD);
        assert_eq!(k.unsupported_commands, 0);

        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_READ_OUTPUT_PORT));
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, 0xDD);

        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_OUTPUT_PORT));
        k.port_write(I8042_DATA, 1, 0xDF); // A20 on
        assert!(k.a20_enabled());

        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_SELF_TEST));
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, SELF_TEST_OK);
    }

    #[test]
    fn reset_restores_a20_enabled() {
        let mut k = I8042::new();
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_OUTPUT_PORT));
        k.port_write(I8042_DATA, 1, 0xDD);
        assert!(!k.a20_enabled());
        k.reset();
        assert!(k.a20_enabled());
        assert_eq!(k.output_port, DEFAULT_OUTPUT_PORT);
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

    /// Spec: OSDev I8042 — keyboard clock disabled (config bit4) drops device data.
    #[test]
    fn inject_scancode_dropped_when_clock_disabled() {
        let mut k = I8042::new();
        assert!(k.keyboard_clock_disabled());
        assert!(!k.inject_scancode(0x1C));
        assert_eq!(k.status() & STATUS_OBF, 0);
        assert_eq!(k.output_buffer(), None);
        assert!(!k.irq1_line());
    }

    /// Spec: OSDev I8042 / IBM PC AT — enabled first port accepts make-code → OBF.
    #[test]
    fn inject_scancode_sets_obf_when_kbd_enabled() {
        let mut k = I8042::new();
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_ENABLE_KBD));
        assert!(!k.keyboard_clock_disabled());
        // INT1 clear: inject still fills OBF; no IRQ rising edge.
        assert!(!k.inject_scancode(0x1C)); // Set1 make-code 'A'
        assert_ne!(k.status() & STATUS_OBF, 0);
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, 0x1C);
        assert_eq!(k.status() & STATUS_OBF, 0);
    }

    /// Spec: OBF ∧ config INT1 → IRQ1; reading 0x60 clears OBF / IRQ1.
    #[test]
    fn inject_scancode_with_int1_asserts_irq1_cleared_by_read() {
        let mut k = I8042::new();
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_ENABLE_KBD));
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_CONFIG));
        // Clock enabled (bit4 clear), INT1 + translate (firmware-like).
        k.port_write(I8042_DATA, 1, u32::from(CFG_INT1 | CFG_TRANSLATE));
        assert!(k.inject_scancode(0x1C));
        assert!(k.irq1_line());
        assert_ne!(k.status() & STATUS_OBF, 0);
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, 0x1C);
        assert!(!k.irq1_line());
        assert_eq!(k.status() & STATUS_OBF, 0);
    }

    /// Translation bit is passthrough: raw make-code placed whether on or off.
    #[test]
    fn inject_scancode_passthrough_regardless_of_translate() {
        let mut k = I8042::new();
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_ENABLE_KBD));
        // Translate off.
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_CONFIG));
        k.port_write(I8042_DATA, 1, 0x00);
        assert_eq!(k.config & CFG_TRANSLATE, 0);
        k.inject_scancode(0x1E);
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, 0x1E);

        // Translate on — still no Set2↔Set1 remap in this stub.
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_CONFIG));
        k.port_write(I8042_DATA, 1, u32::from(CFG_TRANSLATE));
        k.inject_scancode(0x1E);
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, 0x1E);
    }
}
