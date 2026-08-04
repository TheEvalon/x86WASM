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
//! - IBM PS/2 keyboard-controller second (auxiliary) port: commands `0xA7`
//!   (disable aux interface), `0xA8` (enable aux interface), `0xA9` (test aux
//!   interface → result byte, `0x00` = no error), `0xD4` (write next data-port
//!   byte to the aux device); status bit 5 = AUX OBF; command byte bit 1 = aux
//!   interrupt enable (IRQ12 / 8259A slave IR4), bit 5 = aux clock disable.
//! - `docs/sources.md` (PS/2 and 8042 references), `docs/machine-model-pc-v1.md`,
//!   `plan.md` §15.4.
//!
//! # Scope (this slice)
//!
//! Register bank wired onto `machine-pc::MachineBus` at ports `0x60`/`0x64`:
//! status bits useful for firmware polling, a small documented command subset,
//! output-buffer data path, IRQ1 when config bit0 is set and OBF is set from the
//! keyboard, and a make-code inject stub (`inject_scancode`) that respects
//! keyboard clock disable. Instant command completion (IBF never stays set
//! across a status poll).
//!
//! Second (auxiliary) PS/2 **port**: `0xA7`/`0xA8` toggle config bit 5, `0xA9`
//! answers `0x00` on the normal output buffer, `0xD4` routes the next data-port
//! byte to the aux device (recorded in [`I8042::last_aux_device_write`] /
//! [`I8042::aux_device_writes`]; no device, so no response byte), and
//! [`I8042::inject_aux_byte`] fills the buffer with AUX OBF set → IRQ12 when
//! config bit 1 is set. The buffered byte's source selects the line: keyboard
//! data drives IRQ1 only, aux data drives IRQ12 only.
//!
//! # Unsupported (explicit)
//!
//! - PS/2 **mouse device** behind the aux port: reset `0xFF`, ACK `0xFA`, enable
//!   reporting `0xF4`, sample rate / resolution, 3-byte movement packets,
//!   wheel / 5-button extensions (`0xD4` bytes are recorded, never answered)
//! - Aux clock disable (config bit 5) is not applied to host→device `0xD4`
//!   writes; it only gates [`I8042::inject_aux_byte`]
//! - Full AT keyboard protocol (no host→device commands, no `0xFA` ACK, no break codes)
//! - Set2↔Set1 translation table (config bit6 is stored; inject is passthrough)
//! - Pulse-reset lines (`0xFE` / output-port bit0 system-reset)
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
/// Status bit 5: auxiliary output buffer full (second-port data at `0x60`).
///
/// Spec: IBM PS/2 keyboard controller — bit 5 flags mouse/aux data. (On the
/// original AT this bit was a transmit/receive timeout; the PS/2 meaning is the
/// one modeled here, matching the classic PC target machine.)
pub const STATUS_AUX_OBF: u8 = 1 << 5;

/// Controller command: read configuration byte → response on data port.
pub const CMD_READ_CONFIG: u8 = 0x20;
/// Controller command: write next data-port byte to configuration byte.
pub const CMD_WRITE_CONFIG: u8 = 0x60;
/// Controller command: disable second PS/2 port (auxiliary clock inhibit).
pub const CMD_DISABLE_AUX: u8 = 0xA7;
/// Controller command: enable second PS/2 port.
pub const CMD_ENABLE_AUX: u8 = 0xA8;
/// Controller command: test second PS/2 port → result byte on data port.
pub const CMD_TEST_AUX: u8 = 0xA9;
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
/// Controller command: write next data-port byte to the auxiliary device.
pub const CMD_WRITE_AUX: u8 = 0xD4;

/// Self-test passed response (OSDev / IBM AT).
pub const SELF_TEST_OK: u8 = 0x55;
/// Second-port interface test result: no error (IBM PS/2 `0xA9`).
pub const TEST_AUX_OK: u8 = 0x00;

/// Output-port bit 1: A20 gate enable (1 = A20 line high / unmasked).
pub const OUTPUT_PORT_A20: u8 = 1 << 1;
/// Power-on / reset default output port: A20 enabled (classic AT open gate).
/// Other bits are stored but not claimed (system-reset / clock / IRQ lines).
const DEFAULT_OUTPUT_PORT: u8 = 0xDF;

/// Configuration bit 0: first PS/2 port interrupt (IRQ1) enable when set.
pub const CFG_INT1: u8 = 1 << 0;
/// Configuration bit 1: second PS/2 port interrupt (IRQ12) enable when set.
pub const CFG_INT12: u8 = 1 << 1;
/// Configuration bit 4: first PS/2 port clock disabled when set.
pub const CFG_KBD_CLOCK_DISABLE: u8 = 1 << 4;
/// Configuration bit 5: second PS/2 port clock disabled when set.
pub const CFG_AUX_CLOCK_DISABLE: u8 = 1 << 5;
/// Configuration bit 6: first PS/2 port translation enabled when set.
///
/// Stored and readable; this stub does **not** remap Set2↔Set1 bytes.
pub const CFG_TRANSLATE: u8 = 1 << 6;

/// Reset default configuration: keyboard clock disabled, translation on,
/// auxiliary clock enabled (bit5 clear). Both IRQ enables (bits 0/1) stay clear
/// until firmware writes the config byte.
const DEFAULT_CONFIG: u8 = CFG_KBD_CLOCK_DISABLE | CFG_TRANSLATE; // 0x50

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingWrite {
    None,
    /// Next `0x60` write updates the controller configuration byte (`0x60` cmd).
    ConfigByte,
    /// Next `0x60` write updates the controller output port (`0xD1` cmd).
    OutputPort,
    /// Next `0x60` write is routed to the auxiliary device (`0xD4` cmd).
    AuxDevice,
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
    /// Source of the buffered byte: auxiliary device (status bit 5) vs keyboard
    /// / controller response. Selects IRQ12 vs IRQ1.
    output_from_aux: bool,
    /// Status bit 3: last host write targeted the command port.
    last_write_was_cmd: bool,
    pending: PendingWrite,
    /// Counts of unsupported command bytes seen (for tests / diagnostics).
    pub unsupported_commands: u32,
    /// Last byte the host sent to the auxiliary device via `0xD4`.
    ///
    /// No mouse device exists in this slice, so aux-bound bytes are recorded
    /// here instead of being answered (see module `# Unsupported`).
    pub last_aux_device_write: Option<u8>,
    /// Count of `0xD4`-routed host→auxiliary-device bytes (tests / diagnostics).
    pub aux_device_writes: u32,
}

impl I8042 {
    pub fn new() -> Self {
        let mut s = Self {
            config: DEFAULT_CONFIG,
            output_port: DEFAULT_OUTPUT_PORT,
            output: None,
            output_from_aux: false,
            last_write_was_cmd: false,
            pending: PendingWrite::None,
            unsupported_commands: 0,
            last_aux_device_write: None,
            aux_device_writes: 0,
        };
        s.apply_reset_defaults();
        s
    }

    fn apply_reset_defaults(&mut self) {
        self.config = DEFAULT_CONFIG;
        self.output_port = DEFAULT_OUTPUT_PORT;
        self.output = None;
        self.output_from_aux = false;
        self.last_write_was_cmd = false;
        self.pending = PendingWrite::None;
        self.unsupported_commands = 0;
        self.last_aux_device_write = None;
        self.aux_device_writes = 0;
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
        if self.aux_obf() {
            // Spec: IBM PS/2 KBC — bit 5 set when the buffered byte is aux data.
            s |= STATUS_AUX_OBF;
        }
        s
    }

    pub fn output_buffer(&self) -> Option<u8> {
        self.output
    }

    pub fn keyboard_clock_disabled(&self) -> bool {
        self.config & CFG_KBD_CLOCK_DISABLE != 0
    }

    /// Second-port (auxiliary) clock inhibit (config bit 5).
    pub fn aux_clock_disabled(&self) -> bool {
        self.config & CFG_AUX_CLOCK_DISABLE != 0
    }

    /// First-port interrupt enable (config bit 0).
    pub fn irq1_enabled(&self) -> bool {
        self.config & CFG_INT1 != 0
    }

    /// Second-port (auxiliary) interrupt enable (config bit 1).
    pub fn irq12_enabled(&self) -> bool {
        self.config & CFG_INT12 != 0
    }

    /// Auxiliary output buffer full (status bit 5): buffered byte came from the
    /// second PS/2 port.
    pub fn aux_obf(&self) -> bool {
        self.output.is_some() && self.output_from_aux
    }

    /// ISA IRQ1 line level: keyboard-sourced OBF ∧ config INT1 enable.
    ///
    /// Spec: OSDev I8042 / IBM PC AT — keyboard IRQ when output buffer full and
    /// interrupt enabled in the controller configuration byte. IBM PS/2 KBC:
    /// auxiliary data raises IRQ12 instead, so aux bytes never drive IRQ1.
    pub fn irq1_line(&self) -> bool {
        self.output.is_some() && !self.output_from_aux && self.irq1_enabled()
    }

    /// ISA IRQ12 line level: AUX OBF ∧ config bit 1 (aux interrupt enable).
    ///
    /// Spec: IBM PS/2 keyboard controller — second-port data raises IRQ12
    /// (8259A slave IR4) when the command byte enables the aux interrupt.
    pub fn irq12_line(&self) -> bool {
        self.aux_obf() && self.irq12_enabled()
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

    /// Inject an auxiliary-device (second PS/2 port) byte into the output buffer.
    ///
    /// Spec: IBM PS/2 keyboard controller — aux data sets OBF *and* AUX OBF
    /// (status bit 5) and raises IRQ12 when config bit 1 is set. The aux clock
    /// disable (config bit 5) inhibits the interface, so the byte is dropped.
    ///
    /// No mouse device is modeled: callers supply the byte a device would send.
    /// Returns true if IRQ12 had a rising edge (false→true) as a result.
    pub fn inject_aux_byte(&mut self, value: u8) -> bool {
        if self.aux_clock_disabled() {
            return false;
        }
        let prev = self.irq12_line();
        self.output = Some(value);
        self.output_from_aux = true;
        !prev && self.irq12_line()
    }

    fn push_output(&mut self, value: u8) {
        self.output = Some(value);
        self.output_from_aux = false;
    }

    /// Pop the output buffer, clearing OBF and AUX OBF (`0x60` read).
    fn take_output(&mut self) -> Option<u8> {
        self.output_from_aux = false;
        self.output.take()
    }

    fn handle_command(&mut self, cmd: u8) {
        match cmd {
            CMD_READ_CONFIG => {
                self.push_output(self.config);
            }
            CMD_WRITE_CONFIG => {
                self.pending = PendingWrite::ConfigByte;
            }
            CMD_DISABLE_AUX => {
                self.config |= CFG_AUX_CLOCK_DISABLE;
            }
            CMD_ENABLE_AUX => {
                self.config &= !CFG_AUX_CLOCK_DISABLE;
            }
            CMD_TEST_AUX => {
                // Spec: IBM PS/2 — controller response on the normal output
                // buffer (OBF, not AUX OBF); 0x00 = no error detected.
                self.push_output(TEST_AUX_OK);
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
            CMD_WRITE_AUX => {
                self.pending = PendingWrite::AuxDevice;
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
                let v = self.take_output().unwrap_or(0);
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
                    PendingWrite::AuxDevice => {
                        // Spec: IBM PS/2 `0xD4` — byte is sent to the aux device.
                        // No mouse device here: record it, never answer it.
                        self.last_aux_device_write = Some(v);
                        self.aux_device_writes = self.aux_device_writes.saturating_add(1);
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

    /// Spec: OSDev I8042 / IBM PS/2 KBC — `0xA8` enables and `0xA7` disables the
    /// second (auxiliary) PS/2 port by clearing / setting config-byte bit 5
    /// (auxiliary clock disable). Both are supported controller commands.
    #[test]
    fn aux_enable_a8_disable_a7_toggle_config_clock_bit() {
        let mut k = I8042::new();
        // Reset default (0x50) leaves the aux clock enabled (bit5 clear).
        assert!(!k.aux_clock_disabled());

        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_DISABLE_AUX));
        assert!(k.aux_clock_disabled());
        assert_ne!(k.config & CFG_AUX_CLOCK_DISABLE, 0);

        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_ENABLE_AUX));
        assert!(!k.aux_clock_disabled());
        assert_eq!(k.config & CFG_AUX_CLOCK_DISABLE, 0);

        // Readable back through the read-config-byte command.
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_DISABLE_AUX));
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_READ_CONFIG));
        let cfg = k.port_read(I8042_DATA, 1) as u8;
        assert_ne!(cfg & CFG_AUX_CLOCK_DISABLE, 0);

        // Neither command counts as unsupported, and neither touches bit4.
        assert_eq!(k.unsupported_commands, 0);
        assert!(k.keyboard_clock_disabled());
    }

    /// Spec: OSDev I8042 / IBM PS/2 KBC — `0xA9` (test second PS/2 port) returns a
    /// result byte (`0x00` = no error). Controller response: normal OBF, not AUX OBF.
    #[test]
    fn test_aux_a9_returns_00_on_normal_output_buffer() {
        let mut k = I8042::new();
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_TEST_AUX));
        assert_ne!(k.status() & STATUS_OBF, 0);
        assert_eq!(k.status() & STATUS_AUX_OBF, 0);
        assert!(!k.aux_obf());
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, TEST_AUX_OK);
        assert_eq!(k.status() & STATUS_OBF, 0);
        assert_eq!(k.unsupported_commands, 0);
    }

    /// Spec: OSDev I8042 / IBM PS/2 KBC — `0xD4` routes the next data-port byte to
    /// the auxiliary device. No mouse device exists in this slice, so the byte is
    /// recorded / counted and produces no device response; the following data-port
    /// write goes back to the keyboard path (accepted and dropped).
    #[test]
    fn write_aux_d4_routes_next_data_byte_to_aux_device() {
        let mut k = I8042::new();
        assert_eq!(k.aux_device_writes, 0);
        assert_eq!(k.last_aux_device_write, None);

        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_AUX));
        k.port_write(I8042_DATA, 1, 0xF4); // mouse enable-reporting (no device)
        assert_eq!(k.aux_device_writes, 1);
        assert_eq!(k.last_aux_device_write, Some(0xF4));
        // No mouse device: no ACK byte, so no OBF / AUX OBF and no IRQ.
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);
        assert!(!k.irq12_line());

        // Next data byte is keyboard-bound again: aux state untouched.
        k.port_write(I8042_DATA, 1, 0xED);
        assert_eq!(k.aux_device_writes, 1);
        assert_eq!(k.last_aux_device_write, Some(0xF4));
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);
        assert_eq!(k.unsupported_commands, 0);
    }

    /// Spec: IBM PS/2 KBC — auxiliary-device data sets status bit0 (OBF) and bit5
    /// (AUX OBF); the data-port read returns the byte and clears both.
    #[test]
    fn inject_aux_byte_sets_obf_and_aux_obf_cleared_by_read() {
        let mut k = I8042::new();
        assert!(!k.inject_aux_byte(0x08)); // INT12 clear: no IRQ12 edge
        assert_ne!(k.status() & STATUS_OBF, 0);
        assert_ne!(k.status() & STATUS_AUX_OBF, 0);
        assert!(k.aux_obf());
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, 0x08);
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);
        assert!(!k.aux_obf());
    }

    /// Spec: IBM PS/2 KBC — aux clock disabled (config bit5) inhibits the second
    /// port, so injected device data is dropped (mirrors bit4 / keyboard).
    #[test]
    fn inject_aux_byte_dropped_when_aux_clock_disabled() {
        let mut k = I8042::new();
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_DISABLE_AUX));
        assert!(!k.inject_aux_byte(0x08));
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);
        assert_eq!(k.output_buffer(), None);
        assert!(!k.irq12_line());
    }

    /// Spec: IBM PS/2 KBC — IRQ12 = AUX OBF ∧ config bit1 (second-port interrupt
    /// enable); the data-port read clears AUX OBF and deasserts IRQ12.
    #[test]
    fn aux_obf_with_int12_asserts_irq12_cleared_by_read() {
        let mut k = I8042::new();
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_CONFIG));
        k.port_write(I8042_DATA, 1, u32::from(CFG_INT12));
        assert!(k.irq12_enabled());
        assert!(!k.irq12_line());

        assert!(k.inject_aux_byte(0x08));
        assert!(k.irq12_line());
        assert_ne!(k.status() & STATUS_AUX_OBF, 0);

        assert_eq!(k.port_read(I8042_DATA, 1) as u8, 0x08);
        assert!(!k.irq12_line());
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);
    }

    /// Spec: IBM PS/2 KBC — AUX OBF without config bit1 does not assert IRQ12.
    #[test]
    fn aux_obf_without_int12_no_irq12() {
        let mut k = I8042::new();
        assert!(!k.irq12_enabled());
        assert!(!k.inject_aux_byte(0x08));
        assert_ne!(k.status() & STATUS_AUX_OBF, 0);
        assert!(!k.irq12_line());
    }

    /// Spec: IBM PS/2 KBC — the source of the buffered byte selects the interrupt:
    /// keyboard data → OBF only → IRQ1; aux data → OBF + AUX OBF → IRQ12.
    #[test]
    fn keyboard_data_drives_irq1_only_aux_data_drives_irq12_only() {
        let mut k = I8042::new();
        // Both interrupt enables set; both clocks enabled (bits 4/5 clear).
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_CONFIG));
        k.port_write(I8042_DATA, 1, u32::from(CFG_INT1 | CFG_INT12));

        assert!(k.inject_scancode(0x1C));
        assert_ne!(k.status() & STATUS_OBF, 0);
        assert_eq!(k.status() & STATUS_AUX_OBF, 0);
        assert!(k.irq1_line());
        assert!(!k.irq12_line());
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, 0x1C);
        assert!(!k.irq1_line());

        assert!(k.inject_aux_byte(0x08));
        assert_ne!(k.status() & STATUS_AUX_OBF, 0);
        assert!(k.irq12_line());
        assert!(!k.irq1_line());
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, 0x08);
        assert!(!k.irq12_line());
    }

    /// Spec: reset clears the output buffer, so AUX OBF / IRQ12 and the recorded
    /// `0xD4` aux-device traffic return to power-on state.
    #[test]
    fn reset_clears_aux_state() {
        let mut k = I8042::new();
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_DISABLE_AUX));
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_AUX));
        k.port_write(I8042_DATA, 1, 0xFF);
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_ENABLE_AUX));
        k.inject_aux_byte(0x08);
        assert!(k.aux_obf());
        k.reset();
        assert_eq!(k, I8042::new());
        assert!(!k.aux_obf());
        assert_eq!(k.aux_device_writes, 0);
        assert_eq!(k.last_aux_device_write, None);
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
