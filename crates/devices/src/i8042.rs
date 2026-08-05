//! IBM PC AT 8042 / PS/2 controller register bank (ports `0x60` / `0x64`).
//!
//! # Spec refs
//!
//! - OSDev Wiki: [I8042 PS/2 Controller](https://wiki.osdev.org/I8042_PS/2_Controller)
//!   — data/status/command ports, status OBF/IBF, controller self-test `0xAA`→`0x55`,
//!   configuration byte via `0x20`/`0x60` (bit0 = first-port IRQ enable → IRQ1),
//!   disable/enable first port `0xAD`/`0xAE`.
//! - OSDev Wiki: [PS/2 Keyboard](https://wiki.osdev.org/PS/2_Keyboard) — host→device
//!   commands written to data port `0x60` when the controller is not expecting a
//!   command-byte parameter: `0xFF` Reset (ACK `0xFA`, BAT `0xAA`), `0xF2` Get
//!   Keyboard ID (ACK then ID bytes, typically `0xAB` `0x83` for MF2), `0xF4`/
//!   `0xF5` Enable/Disable Scanning (ACK `0xFA`), `0xED` Set LEDs (+ LED mask),
//!   `0xF3` Set Typematic Rate/Delay (+ rate/delay byte), `0xF0` Get/Set
//!   Scancode Set (+ sub-command), `0xF6` Set Default, `0xF7`/`0xF8`/`0xF9`/
//!   `0xFA` Set All Keys Typematic/Make-Break/Make/Typematic-Make-Break (ACK),
//!   `0xFB`/`0xFC`/`0xFD` Set Key Type (+ scancode), `0xEE` Echo, `0xFE` Resend
//!   (last byte).
//! - OSDev Wiki: [PS/2 Mouse](https://wiki.osdev.org/PS/2_Mouse) — host→device
//!   commands `0xFF` Reset (ACK `0xFA`, BAT `0xAA`, ID `0x00`), `0xF2` Get Device
//!   ID, `0xF4`/`0xF5` Enable/Disable Data Reporting (ACK `0xFA`), `0xF6` Set
//!   Defaults (ACK; restore defaults — **no BAT**), `0xF3` Set Sample Rate
//!   (+ value), `0xE8` Set Resolution (+ value), `0xE9` Status Request
//!   (ACK + 3-byte status), `0xE6`/`0xE7` Set Scaling 1:1 / 2:1, `0xEA` Set
//!   Stream Mode / `0xF0` Set Remote Mode (ACK; mode flag stored; either clears
//!   wrap when executed), `0xEE` Set Wrap Mode (ACK; wrap flag stored; while
//!   wrap is set, subsequent host→aux bytes via `0xD4` are echoed on AUX OBF
//!   with **no ACK** except Reset Wrap `0xEC` and Reset `0xFF`, which remain
//!   commands — OSDev PS/2 Mouse wrap mode), `0xEC` Reset Wrap Mode (ACK;
//!   clears wrap flag; stream/remote unchanged), `0xEB` Read Data (ACK then one
//!   3-byte movement packet — last inject or zeros; SeaBIOS/OSDev-friendly stub
//!   answers in stream or remote mode); stream-mode **Mouse Packet Format**
//!   (3 bytes: flags/buttons/signs/overflows, X movement, Y movement) when
//!   reporting is enabled.
//! - IBM PC/AT 8042 keyboard-controller programming model (command/status/data);
//!   output-buffer-full with IRQ enable → ISA IRQ1 (8259A master IR1).
//! - IBM PS/2 keyboard-controller port tests / second (auxiliary) port: commands
//!   `0xA7` (disable aux interface), `0xA8` (enable aux interface), `0xA9` (test
//!   aux interface → result byte, `0x00` = no error), `0xAB` (test keyboard
//!   interface → `0x00` = no error; same result codes as `0xA9`), `0xAC`
//!   (diagnostic dump → [`DIAG_DUMP_LEN`] stub bytes on normal OBF), `0xD4`
//!   (write next data-port byte to the aux device); status bit 5 = AUX OBF;
//!   command byte bit 1 = aux interrupt enable (IRQ12 / 8259A slave IR4),
//!   bit 5 = aux clock disable.
//! - `docs/sources.md` (PS/2 and 8042 references), `docs/machine-model-pc-v1.md`,
//!   `plan.md` §15.4.
//!
//! # Scope (this slice)
//!
//! Register bank wired onto `machine-pc::MachineBus` at ports `0x60`/`0x64`:
//! status bits useful for firmware polling, a small documented command subset,
//! output-buffer data path, IRQ1 when config bit0 is set and OBF is set from the
//! keyboard, and a make-code inject path (`inject_scancode`) that respects
//! keyboard clock disable and, when config bit6 is set, applies IBM PC/XT
//! Scan Set 2→Set 1 translation on the way to host OBF (OSDev I8042
//! "Translation" + Andries Brouwer / Gary J. Konzak 8042 table). Instant
//! command completion (IBF never stays set across a status poll).
//!
//! First-port **keyboard** device: when `PendingWrite` is none, a data-port
//! (`0x60`) write is a host→keyboard command (or a pending command parameter).
//! A minimal keyboard stub answers Enable/Disable Scanning (`0xF4`/`0xF5`) with
//! ACK `0xFA`, Get Keyboard ID (`0xF2`) with ACK + MF2 ID `0xAB` `0x83`, Reset
//! (`0xFF`) with ACK + BAT `0xAA`, Set LEDs (`0xED` + mask), Set Typematic
//! Rate/Delay (`0xF3` + byte), and Get/Set Scancode Set (`0xF0` + sub-command)
//! with ACK per byte and stored state, Set Default (`0xF6`) with ACK + restore
//! defaults (no BAT), Set All Keys (`0xF7`/`0xF8`/`0xF9`/`0xFA`) with ACK,
//! Set Key Type (`0xFB`/`0xFC`/`0xFD`) with ACK + one ignored scancode, plus
//! Echo (`0xEE` → `0xEE`) and Resend (`0xFE` → requeue last keyboard OBF byte),
//! on the keyboard OBF (not AUX) → IRQ1 when config bit 0 is set. Multi-byte
//! responses queue like the mouse stub; keyboard clock disable (config bit4)
//! holds presentation until `0xAE`. Other host→kbd bytes are accepted and left
//! unanswered.
//!
//! Controller port tests: `0xA9` (aux) and `0xAB` (keyboard) each answer
//! `0x00` (no error) on the **normal** output buffer (OBF, not AUX OBF).
//! Diagnostic dump `0xAC` returns a fixed [`DIAG_DUMP_LEN`]-byte stub
//! ([`DIAG_DUMP_STUB`] zeros — internal RAM beyond the config byte is not
//! modeled) on normal OBF, one byte per `0x60` read (not gated by keyboard
//! clock disable).
//!
//! Second (auxiliary) PS/2 **port**: `0xA7`/`0xA8` toggle config bit 5, `0xD4`
//! routes the next data-port byte to the aux device when the aux clock is
//! enabled (recorded in [`I8042::last_aux_device_write`] /
//! [`I8042::aux_device_writes`]). When aux clock is disabled (`0xA7` / config
//! bit 5), host→aux bytes via `0xD4` are **queued** (controller still consumes
//! the pending write; mouse stub is not invoked yet — no ACK, no recorded
//! traffic, no IRQ12). Re-enabling the aux clock (`0xA8` or write-config
//! clearing bit5) **flushes** the FIFO to the mouse stub in order. Raw
//! [`I8042::inject_aux_byte`] still **drops** while the clock is disabled.
//! A minimal PS/2 **mouse stub** answers the common identify/reset/enable
//! commands with ACK/`0xFA` (and BAT/ID where required) on AUX OBF → IRQ12 when
//! config bit 1 is set. Parameter commands
//! (`0xF3`/`0xE8`/`0xE9`/`0xE6`/`0xE7`/`0xEA`/`0xF0`/`0xEE`/`0xEC`/`0xEB`/`0xF6`)
//! store rate/resolution/scaling/mode/wrap and answer Status Request / Read Data
//! with the OSDev packets; while wrap is set, non-`0xEC`/`0xFF` host→aux bytes
//! are echoed (no ACK); Reset and Set Defaults restore defaults (stream mode,
//! wrap off) — Set Defaults ACKs only (no BAT/ID).
//! [`I8042::inject_mouse_packet`] queues a standard 3-byte movement packet when
//! data reporting is enabled (`0xF4`) and remembers last dx/dy/buttons for
//! Read Data (`0xEB`); while reporting is disabled (`0xF5` / Reset / Set Defaults)
//! injects are **dropped** (not deferred). Packet bytes and command responses
//! already accepted present on AUX OBF one at a time (held while aux clock
//! disabled; `0xA8` flushes) → IRQ12 when config bit 1 is set. The buffered
//! byte's source selects the line: keyboard data drives IRQ1 only, aux data
//! IRQ12 only. [`I8042::inject_aux_byte`] remains available for raw test
//! injection.
//!
//! # Unsupported (explicit)
//!
//! - Wheel / 5-button (IntelliMouse) extensions / full remote protocol beyond
//!   Read Data `0xEB` (other host→aux bytes outside wrap are recorded,
//!   unanswered)
//! - Full AT keyboard protocol beyond the ACK/param/Resend/Set-All/Set-Key-Type
//!   subset above (per-key set-3 type state not modeled beyond ACK; LED mask /
//!   scancode-set / typematic are stored only — no host-visible LED hardware;
//!   inject still emits the existing Set2-oriented stream regardless of stored
//!   set)
//! - Interface-test electrical fault codes beyond the success stub `0x00` for
//!   `0xA9`/`0xAB`; authentic AT ASCII/scancode dump encoding for `0xAC`

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
/// Controller command: test first PS/2 port (keyboard) → result byte on data port.
///
/// Spec: OSDev I8042 — same result codes as `0xA9`; `0x00` = no error.
pub const CMD_TEST_KBD: u8 = 0xAB;
/// Controller command: diagnostic dump (read internal RAM) → multi-byte on data port.
///
/// Spec: OSDev I8042 — `0xAC` Diagnostic dump; response size/content is
/// controller-specific. This emulator returns [`DIAG_DUMP_LEN`] fixed stub bytes
/// ([`DIAG_DUMP_STUB`]) on normal OBF.
pub const CMD_DIAG_DUMP: u8 = 0xAC;
/// Controller command: disable first PS/2 port (keyboard clock inhibit).
pub const CMD_DISABLE_KBD: u8 = 0xAD;
/// Controller command: enable first PS/2 port.
pub const CMD_ENABLE_KBD: u8 = 0xAE;
/// Controller command: read output port → response on data port.
pub const CMD_READ_OUTPUT_PORT: u8 = 0xD0;
/// Controller command: write next data-port byte to output port (A20 bit1;
/// system-reset bit0 active-low → [`I8042::take_system_reset_request`]).
pub const CMD_WRITE_OUTPUT_PORT: u8 = 0xD1;
/// Controller command: write next data-port byte to the auxiliary device.
pub const CMD_WRITE_AUX: u8 = 0xD4;
/// Controller command: pulse output port bits (commonly used as system reset pulse).
/// Spec: OSDev I8042 — `0xFE` on command port `0x64` (not keyboard Resend on `0x60`).
pub const CMD_PULSE_RESET: u8 = 0xFE;

/// Self-test passed response (OSDev / IBM AT).
pub const SELF_TEST_OK: u8 = 0x55;
/// Second-port interface test result: no error (IBM PS/2 `0xA9`).
pub const TEST_AUX_OK: u8 = 0x00;
/// First-port (keyboard) interface test result: no error (IBM PS/2 `0xAB`).
pub const TEST_KBD_OK: u8 = 0x00;

/// Byte count for the [`CMD_DIAG_DUMP`] (`0xAC`) stub response.
///
/// Spec: OSDev I8042 — diagnostic dump reads internal RAM; common 8042 models
/// expose about 16 RAM bytes. Exact silicon maps vary.
pub const DIAG_DUMP_LEN: usize = 16;
/// Fixed diagnostic-dump stub payload (all zeros).
///
/// Documents that controller internal RAM beyond the configuration byte is not
/// modeled; firmware that only checks that `0xAC` produces a bounded OBF stream
/// still observes a SeaBIOS-friendly success path.
pub const DIAG_DUMP_STUB: [u8; DIAG_DUMP_LEN] = [0; DIAG_DUMP_LEN];

/// PS/2 mouse / keyboard ACK response (OSDev PS/2 Keyboard / Mouse).
pub const MOUSE_ACK: u8 = 0xFA;
/// Alias: keyboard command ACK (`0xFA`).
pub const KBD_ACK: u8 = MOUSE_ACK;
/// PS/2 mouse BAT (Basic Assurance Test) passed after Reset `0xFF`.
pub const MOUSE_BAT_OK: u8 = 0xAA;
/// Keyboard BAT passed after Reset `0xFF` (same value as mouse BAT).
pub const KBD_BAT_OK: u8 = MOUSE_BAT_OK;
/// Standard PS/2 mouse device ID (no IntelliMouse / 5-button extensions).
pub const MOUSE_ID_STANDARD: u8 = 0x00;
/// First MF2 keyboard ID byte after Get Keyboard ID (`0xF2`).
pub const KBD_ID_MF2_0: u8 = 0xAB;
/// Second MF2 keyboard ID byte after Get Keyboard ID (`0xF2`).
pub const KBD_ID_MF2_1: u8 = 0x83;

/// PS/2 keyboard command: Reset → ACK, BAT.
pub const KBD_CMD_RESET: u8 = 0xFF;
/// PS/2 keyboard command: Get Keyboard ID → ACK, ID bytes.
pub const KBD_CMD_GET_ID: u8 = 0xF2;
/// PS/2 keyboard command: Enable Scanning → ACK.
pub const KBD_CMD_ENABLE_SCANNING: u8 = 0xF4;
/// PS/2 keyboard command: Disable Scanning → ACK.
pub const KBD_CMD_DISABLE_SCANNING: u8 = 0xF5;
/// PS/2 keyboard command: Set LEDs → ACK; next data-port byte is the LED mask.
pub const KBD_CMD_SET_LEDS: u8 = 0xED;
/// PS/2 keyboard command: Echo → respond with Echo `0xEE` (no separate ACK).
pub const KBD_CMD_ECHO: u8 = 0xEE;
/// PS/2 keyboard command: Get/Set Scancode Set → ACK; next byte is the sub-command.
pub const KBD_CMD_SET_SCANCODE_SET: u8 = 0xF0;
/// PS/2 keyboard command: Set Typematic Rate/Delay → ACK; next byte is the value.
pub const KBD_CMD_SET_TYPEMATIC: u8 = 0xF3;
/// PS/2 keyboard command: Resend → requeue the last keyboard OBF byte.
pub const KBD_CMD_RESEND: u8 = 0xFE;
/// PS/2 keyboard command: Set Default — restore defaults and ACK.
/// Spec: OSDev PS/2 Keyboard — `0xF6` Set Default.
pub const KBD_CMD_SET_DEFAULT: u8 = 0xF6;
/// PS/2 keyboard command: Set All Keys Typematic — ACK (no further params).
/// Spec: OSDev PS/2 Keyboard — `0xF7` Set All Keys Typematic.
pub const KBD_CMD_SET_ALL_TYPEMATIC: u8 = 0xF7;
/// PS/2 keyboard command: Set All Keys Make/Break — ACK (no further params).
/// Spec: OSDev PS/2 Keyboard — `0xF8` Set All Keys Make/Break.
pub const KBD_CMD_SET_ALL_MAKE_BREAK: u8 = 0xF8;
/// PS/2 keyboard command: Set All Keys Make — ACK (no further params).
/// Spec: OSDev PS/2 Keyboard — `0xF9` Set All Keys Make.
pub const KBD_CMD_SET_ALL_MAKE: u8 = 0xF9;
/// PS/2 keyboard command: Set All Keys Typematic/Make/Break — ACK (no further params).
///
/// Spec: OSDev PS/2 Keyboard — `0xFA` Set All Keys Typematic/Make/Break.
/// Note: the host→device opcode value coincides with the device→host ACK byte
/// [`KBD_ACK`]; direction distinguishes them.
pub const KBD_CMD_SET_ALL_TYPEMATIC_MAKE_BREAK: u8 = 0xFA;
/// PS/2 keyboard command: Set Key Type Typematic — ACK + one scancode param.
/// Spec: OSDev PS/2 Keyboard — `0xFB`.
pub const KBD_CMD_SET_KEY_TYPEMATIC: u8 = 0xFB;
/// PS/2 keyboard command: Set Key Type Make/Break — ACK + one scancode param.
/// Spec: OSDev PS/2 Keyboard — `0xFC`.
pub const KBD_CMD_SET_KEY_MAKE_BREAK: u8 = 0xFC;
/// PS/2 keyboard command: Set Key Type Make — ACK + one scancode param.
/// Spec: OSDev PS/2 Keyboard — `0xFD`.
pub const KBD_CMD_SET_KEY_MAKE: u8 = 0xFD;

/// Echo response byte (same value as [`KBD_CMD_ECHO`]).
pub const KBD_ECHO: u8 = 0xEE;

/// Default typematic rate/delay after Reset (OSDev: ~10.9 cps, 500 ms delay).
///
/// Encoding: bits 4:0 = rate index `0x0B`, bits 6:5 = delay `01` → `0x2B`.
pub const KBD_DEFAULT_TYPEMATIC: u8 = 0x2B;

/// Default scancode set after Reset / power-on (OSDev: Set 2 is the default).
pub const KBD_DEFAULT_SCANCODE_SET: u8 = 2;

/// Get/Set Scancode Set sub-command: return the current set number.
pub const KBD_SCANCODE_GET: u8 = 0;

/// Set LEDs mask bits (OSDev PS/2 Keyboard): bit0 Scroll Lock, bit1 Num Lock,
/// bit2 Caps Lock. Stored as given by the host; helpers for tests / callers.
pub const KBD_LED_SCROLL: u8 = 1 << 0;
pub const KBD_LED_NUM: u8 = 1 << 1;
pub const KBD_LED_CAPS: u8 = 1 << 2;
pub const KBD_LED_MASK: u8 = KBD_LED_SCROLL | KBD_LED_NUM | KBD_LED_CAPS;

/// PS/2 mouse command: Reset → ACK, BAT, device ID.
pub const MOUSE_CMD_RESET: u8 = 0xFF;
/// PS/2 mouse command: Get Device ID → ACK, ID.
pub const MOUSE_CMD_GET_DEVICE_ID: u8 = 0xF2;
/// PS/2 mouse command: Enable Data Reporting → ACK.
pub const MOUSE_CMD_ENABLE_REPORTING: u8 = 0xF4;
/// PS/2 mouse command: Disable Data Reporting → ACK.
pub const MOUSE_CMD_DISABLE_REPORTING: u8 = 0xF5;
/// PS/2 mouse command: Set Defaults → ACK and restore defaults (**no BAT**).
///
/// Spec: OSDev PS/2 Mouse — `0xF6` Set Defaults (sample rate / resolution /
/// scaling / reporting off / stream mode; unlike Reset `0xFF`, no BAT/ID).
pub const MOUSE_CMD_SET_DEFAULTS: u8 = 0xF6;
/// PS/2 mouse command: Set Sample Rate → ACK; next `0xD4` byte is the rate.
pub const MOUSE_CMD_SET_SAMPLE_RATE: u8 = 0xF3;
/// PS/2 mouse command: Set Resolution → ACK; next `0xD4` byte is the value.
pub const MOUSE_CMD_SET_RESOLUTION: u8 = 0xE8;
/// PS/2 mouse command: Status Request → ACK + 3-byte status packet.
pub const MOUSE_CMD_STATUS_REQUEST: u8 = 0xE9;
/// PS/2 mouse command: Set Scaling 1:1 → ACK.
pub const MOUSE_CMD_SET_SCALING_1_1: u8 = 0xE6;
/// PS/2 mouse command: Set Scaling 2:1 → ACK.
pub const MOUSE_CMD_SET_SCALING_2_1: u8 = 0xE7;
/// PS/2 mouse Resend — requeue last AUX OBF byte.
/// Spec: OSDev PS/2 Mouse — `0xFE` Resend.
pub const MOUSE_CMD_RESEND: u8 = 0xFE;
/// Spec: OSDev PS/2 Mouse — `0xEA` Set Stream Mode.
pub const MOUSE_CMD_SET_STREAM_MODE: u8 = 0xEA;
/// Spec: OSDev PS/2 Mouse — `0xF0` Set Remote Mode.
pub const MOUSE_CMD_SET_REMOTE_MODE: u8 = 0xF0;
/// Spec: OSDev PS/2 Mouse — `0xEB` Read Data (force one movement packet).
pub const MOUSE_CMD_READ_DATA: u8 = 0xEB;
/// Spec: OSDev PS/2 Mouse — `0xEE` Set Wrap Mode (ACK; store wrap flag).
///
/// Wrap-mode echoing of subsequent host→aux bytes is **deferred** (flag only).
pub const MOUSE_CMD_SET_WRAP_MODE: u8 = 0xEE;
/// Spec: OSDev PS/2 Mouse — `0xEC` Reset Wrap Mode (ACK; clear wrap flag).
///
/// Leaves stream/remote mode unchanged (Set Wrap only toggles the wrap flag).
pub const MOUSE_CMD_RESET_WRAP_MODE: u8 = 0xEC;

/// Default sample rate after Reset (OSDev PS/2 Mouse defaults).
pub const MOUSE_DEFAULT_SAMPLE_RATE: u8 = 100;
/// Default resolution after Reset: `2` = 4 counts/mm (OSDev encoding 0..3).
pub const MOUSE_DEFAULT_RESOLUTION: u8 = 2;

/// Status Request byte1 bit4: scaling is 2:1 when set (OSDev PS/2 Mouse).
pub const MOUSE_STATUS_SCALING: u8 = 1 << 4;
/// Status Request byte1 bit5: data reporting enabled when set.
pub const MOUSE_STATUS_ENABLE: u8 = 1 << 5;
/// Status Request byte1 bit6: remote mode when set (cleared = stream).
pub const MOUSE_STATUS_REMOTE: u8 = 1 << 6;

/// Movement-packet byte0 bit0: left button (OSDev "Mouse Packet Format").
pub const MOUSE_BTN_LEFT: u8 = 1 << 0;
/// Movement-packet byte0 bit1: right button.
pub const MOUSE_BTN_RIGHT: u8 = 1 << 1;
/// Movement-packet byte0 bit2: middle button.
pub const MOUSE_BTN_MIDDLE: u8 = 1 << 2;
/// Movement-packet byte0 bit3: always 1 in a valid standard packet.
pub const MOUSE_PKT_ALWAYS1: u8 = 1 << 3;
/// Movement-packet byte0 bit4: X sign (1 = negative).
pub const MOUSE_PKT_X_SIGN: u8 = 1 << 4;
/// Movement-packet byte0 bit5: Y sign (1 = negative).
pub const MOUSE_PKT_Y_SIGN: u8 = 1 << 5;
/// Movement-packet byte0 bit6: X overflow.
pub const MOUSE_PKT_X_OVERFLOW: u8 = 1 << 6;
/// Movement-packet byte0 bit7: Y overflow.
pub const MOUSE_PKT_Y_OVERFLOW: u8 = 1 << 7;

/// Capacity of the pending aux-device → host response queue
/// (Status Request needs ACK + 3 status bytes; a movement packet is 3 bytes —
/// allow a few packets without dropping).
const AUX_RESP_QUEUE_CAP: usize = 16;

/// Capacity of the pending host → aux-device queue while aux clock is disabled
/// (`0xA7` / config bit5). Overflow silently drops new bytes.
const HOST_AUX_QUEUE_CAP: usize = 16;

/// Capacity of the pending keyboard → host response queue
/// (Get ID needs ACK + 2 ID bytes; Reset needs ACK + BAT).
const KBD_RESP_QUEUE_CAP: usize = 8;

/// Capacity of the pending controller → host response queue
/// (diagnostic dump `0xAC` returns [`DIAG_DUMP_LEN`] bytes).
const CTRL_RESP_QUEUE_CAP: usize = DIAG_DUMP_LEN;

/// Next host→aux byte expected as a parameter for a prior mouse command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MousePendingParam {
    None,
    /// Following `0xF3` — next byte is sample rate.
    SampleRate,
    /// Following `0xE8` — next byte is resolution (0..3).
    Resolution,
}

/// Next host→keyboard data-port byte expected as a parameter for a prior command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KbdPendingParam {
    None,
    /// Following `0xED` — next byte is the LED mask.
    Leds,
    /// Following `0xF3` — next byte is typematic rate/delay.
    Typematic,
    /// Following `0xF0` — next byte is get (`0`) or set (`1`/`2`/`3`).
    ScancodeSet,
    /// Following `0xFB`/`0xFC`/`0xFD` — next byte is a scancode (ignored).
    KeyTypeScancode,
}

/// Output-port bit 0: system reset line (1 = normal / not reset; 0 = assert reset).
///
/// Spec: IBM PC AT / OSDev I8042 — writing this bit clear via `0xD1` latches
/// [`I8042::take_system_reset_request`] (same machine path as pulse-reset `0xFE`).
pub const OUTPUT_PORT_SYSTEM_RESET: u8 = 1 << 0;
/// Output-port bit 1: A20 gate enable (1 = A20 line high / unmasked).
pub const OUTPUT_PORT_A20: u8 = 1 << 1;
/// Power-on / reset default output port: A20 enabled, system-reset line high
/// (classic AT open gate / not in reset). Other bits stored but not claimed.
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
/// Spec: OSDev [I8042 PS/2 Controller](https://wiki.osdev.org/I8042_PS/2_Controller)
/// "Translation" — when set, the controller remaps keyboard Scan Code Set 2
/// into Scan Code Set 1 (IBM PC/XT compatibility) before placing bytes in the
/// host output buffer. When clear, device bytes pass through unchanged.
pub const CFG_TRANSLATE: u8 = 1 << 6;

/// IBM PC/AT 8042 Scan Set 2 → Set 1 translation table (index = device byte).
///
/// Spec: Andries Brouwer, *Keyboard scancodes* §10 "Keyboard-internal
/// scancodes" (<https://kbd-project.org/docs/scancodes/scancodes-10.html>);
/// first half also in Gary J. Konzak, *PC 8042 Controller*. Byte `0xF0` is
/// handled as the Set 2 break prefix (consumed; next byte OR'd with `0x80`)
/// and is never looked up here — the table slot is unused (`0x00` placeholder).
#[rustfmt::skip]
const SET2_TO_SET1: [u8; 256] = [
    //  0x00  01    02    03    04    05    06    07    08    09    0A    0B    0C    0D    0E    0F
    0xFF, 0x43, 0x41, 0x3F, 0x3D, 0x3B, 0x3C, 0x58, 0x64, 0x44, 0x42, 0x40, 0x3E, 0x0F, 0x29, 0x59, // 0x00
    0x65, 0x38, 0x2A, 0x70, 0x1D, 0x10, 0x02, 0x5A, 0x66, 0x71, 0x2C, 0x1F, 0x1E, 0x11, 0x03, 0x5B, // 0x10
    0x67, 0x2E, 0x2D, 0x20, 0x12, 0x05, 0x04, 0x5C, 0x68, 0x39, 0x2F, 0x21, 0x14, 0x13, 0x06, 0x5D, // 0x20
    0x69, 0x31, 0x30, 0x23, 0x22, 0x15, 0x07, 0x5E, 0x6A, 0x72, 0x32, 0x24, 0x16, 0x08, 0x09, 0x5F, // 0x30
    0x6B, 0x33, 0x25, 0x17, 0x18, 0x0B, 0x0A, 0x60, 0x6C, 0x34, 0x35, 0x26, 0x27, 0x19, 0x0C, 0x61, // 0x40
    0x6D, 0x73, 0x28, 0x74, 0x1A, 0x0D, 0x62, 0x6E, 0x3A, 0x36, 0x1C, 0x1B, 0x75, 0x2B, 0x63, 0x76, // 0x50
    0x55, 0x56, 0x77, 0x78, 0x79, 0x7A, 0x0E, 0x7B, 0x7C, 0x4F, 0x7D, 0x4B, 0x47, 0x7E, 0x7F, 0x6F, // 0x60
    0x52, 0x53, 0x50, 0x4C, 0x4D, 0x48, 0x01, 0x45, 0x57, 0x4E, 0x51, 0x4A, 0x37, 0x49, 0x46, 0x54, // 0x70
    0x80, 0x81, 0x82, 0x41, 0x54, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8A, 0x8B, 0x8C, 0x8D, 0x8E, 0x8F, // 0x80
    0x90, 0x91, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0x9B, 0x9C, 0x9D, 0x9E, 0x9F, // 0x90
    0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xAB, 0xAC, 0xAD, 0xAE, 0xAF, // 0xA0
    0xB0, 0xB1, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xBB, 0xBC, 0xBD, 0xBE, 0xBF, // 0xB0
    0xC0, 0xC1, 0xC2, 0xC3, 0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xCB, 0xCC, 0xCD, 0xCE, 0xCF, // 0xC0
    0xD0, 0xD1, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xDB, 0xDC, 0xDD, 0xDE, 0xDF, // 0xD0
    0xE0, 0xE1, 0xE2, 0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xEB, 0xEC, 0xED, 0xEE, 0xEF, // 0xE0
    0x00, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA, 0xFB, 0xFC, 0xFD, 0xFE, 0xFF, // 0xF0
];

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
    /// Controller output port (commands `0xD0` / `0xD1`); bit0 = system reset
    /// (active low), bit1 = A20 gate.
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
    /// Counts of controller pulse-reset commands (`0xFE` on `0x64`) seen.
    ///
    /// Spec: OSDev I8042 — accepted and counted; [`Self::system_reset_pending`]
    /// latches a request for the machine layer (`Machine::reset`).
    pub pulse_reset_commands: u32,
    /// Latched when the host requests system reset via controller pulse-reset
    /// (`0xFE` on `0x64`) or output-port write (`0xD1`) with bit0 clear.
    ///
    /// Spec: OSDev I8042 / IBM PC AT — pulse output lines / system-reset request.
    /// Cleared by [`Self::take_system_reset_request`] or controller [`Self::reset`].
    system_reset_pending: bool,
    /// Last byte the host sent to the auxiliary device via `0xD4`.
    pub last_aux_device_write: Option<u8>,
    /// Count of `0xD4`-routed host→auxiliary-device bytes (tests / diagnostics).
    pub aux_device_writes: u32,
    /// Mouse stub: data reporting enabled (`0xF4` / cleared by `0xF5` or Reset).
    ///
    /// Stored only — this stub does not emit movement packets.
    mouse_reporting: bool,
    /// Mouse sample rate (reports/sec); default 100. Stored by `0xF3` + value.
    mouse_sample_rate: u8,
    /// Mouse resolution encoding 0..3 (1/2/4/8 counts/mm); default 2.
    mouse_resolution: u8,
    /// Mouse scaling 2:1 when true (`0xE7`); 1:1 when false (`0xE6` / Reset).
    mouse_scaling_21: bool,
    /// Mouse remote mode when true (`0xF0`); stream when false (`0xEA` / Reset).
    ///
    /// Spec: OSDev PS/2 Mouse — Set Remote / Set Stream Mode.
    mouse_remote_mode: bool,
    /// Mouse wrap mode when true (`0xEE`); cleared by Reset / Stream / Remote /
    /// Reset Wrap (`0xEC`).
    ///
    /// Spec: OSDev PS/2 Mouse — Set/Reset Wrap Mode. Flag stored; byte echo deferred.
    mouse_wrap_mode: bool,
    /// Last movement deltas/buttons from a successful [`I8042::inject_mouse_packet`].
    ///
    /// Spec: OSDev PS/2 Mouse — Read Data (`0xEB`) returns this packet (or zeros
    /// if none). Cleared by mouse / controller Reset.
    mouse_last_dx: i16,
    mouse_last_dy: i16,
    mouse_last_buttons: u8,
    /// Awaiting sample-rate or resolution parameter byte after `0xF3` / `0xE8`.
    mouse_pending_param: MousePendingParam,
    /// Pending mouse → host response bytes waiting for an empty output buffer
    /// (and an enabled aux clock) before presentation on AUX OBF.
    aux_resp: [u8; AUX_RESP_QUEUE_CAP],
    aux_resp_len: u8,
    /// Host→aux bytes accepted via `0xD4` while aux clock was disabled; flushed
    /// to the mouse stub when the clock is re-enabled (`0xA8` / config clear bit5).
    host_aux: [u8; HOST_AUX_QUEUE_CAP],
    host_aux_len: u8,
    /// Keyboard stub: scanning enabled (`0xF4` / cleared by `0xF5`; Reset → on).
    ///
    /// When false, [`I8042::inject_scancode`] drops make/break traffic.
    kbd_scanning: bool,
    /// Keyboard LED mask (Scroll/Num/Caps); stored by `0xED` + value. Reset → 0.
    kbd_leds: u8,
    /// Keyboard typematic rate/delay byte; stored by `0xF3` + value.
    kbd_typematic: u8,
    /// Current scancode set (1/2/3); stored by `0xF0` + value. Default 2.
    ///
    /// Stored only — [`I8042::inject_scancode`] still uses the existing
    /// Set2-oriented stream (+ optional controller Set2→Set1 translation).
    kbd_scancode_set: u8,
    /// Awaiting LED / typematic / scancode-set parameter after `0xED` / `0xF3` / `0xF0`.
    kbd_pending_param: KbdPendingParam,
    /// Pending keyboard → host response bytes waiting for an empty output buffer
    /// (and an enabled keyboard clock) before presentation on keyboard OBF.
    kbd_resp: [u8; KBD_RESP_QUEUE_CAP],
    kbd_resp_len: u8,
    /// Pending controller → host response bytes (e.g. diagnostic dump `0xAC`)
    /// waiting for an empty output buffer. Presented via [`Self::push_output`]
    /// (normal OBF, not AUX; not gated by keyboard/aux clock disable; does not
    /// update [`Self::kbd_last_byte`]).
    ctrl_resp: [u8; CTRL_RESP_QUEUE_CAP],
    ctrl_resp_len: u8,
    /// Last byte presented on the keyboard OBF path (ACK/ID/Echo/BAT/scancode).
    ///
    /// Spec: OSDev PS/2 Keyboard — Resend (`0xFE`) requeues this byte. Starts at
    /// `0` until the first keyboard-path presentation; keyboard Reset / controller
    /// reset clears it back to `0`. Controller responses via [`Self::push_output`]
    /// (self-test, read-config, diagnostic dump, …) do **not** update this field.
    kbd_last_byte: u8,
    /// Last byte presented on AUX OBF (for mouse Resend).
    mouse_last_byte: u8,
    /// Set 2 break prefix (`0xF0`) seen while config bit6 translation is on;
    /// the next keyboard byte is translated then OR'd with `0x80` (Set 1 break).
    translate_pending_break: bool,
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
            pulse_reset_commands: 0,
            system_reset_pending: false,
            last_aux_device_write: None,
            aux_device_writes: 0,
            mouse_reporting: false,
            mouse_sample_rate: MOUSE_DEFAULT_SAMPLE_RATE,
            mouse_resolution: MOUSE_DEFAULT_RESOLUTION,
            mouse_scaling_21: false,
            mouse_remote_mode: false,
            mouse_wrap_mode: false,
            mouse_last_dx: 0,
            mouse_last_dy: 0,
            mouse_last_buttons: 0,
            mouse_pending_param: MousePendingParam::None,
            aux_resp: [0; AUX_RESP_QUEUE_CAP],
            aux_resp_len: 0,
            host_aux: [0; HOST_AUX_QUEUE_CAP],
            host_aux_len: 0,
            kbd_scanning: true,
            kbd_leds: 0,
            kbd_typematic: KBD_DEFAULT_TYPEMATIC,
            kbd_scancode_set: KBD_DEFAULT_SCANCODE_SET,
            kbd_pending_param: KbdPendingParam::None,
            kbd_resp: [0; KBD_RESP_QUEUE_CAP],
            kbd_resp_len: 0,
            ctrl_resp: [0; CTRL_RESP_QUEUE_CAP],
            ctrl_resp_len: 0,
            kbd_last_byte: 0,
            mouse_last_byte: 0,
            translate_pending_break: false,
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
        self.pulse_reset_commands = 0;
        self.system_reset_pending = false;
        self.last_aux_device_write = None;
        self.aux_device_writes = 0;
        self.reset_mouse_defaults();
        self.aux_resp = [0; AUX_RESP_QUEUE_CAP];
        self.aux_resp_len = 0;
        self.host_aux = [0; HOST_AUX_QUEUE_CAP];
        self.host_aux_len = 0;
        self.reset_kbd_defaults();
        self.kbd_resp = [0; KBD_RESP_QUEUE_CAP];
        self.kbd_resp_len = 0;
        self.ctrl_resp = [0; CTRL_RESP_QUEUE_CAP];
        self.ctrl_resp_len = 0;
        self.translate_pending_break = false;
    }

    /// Restore keyboard stub defaults (scanning, LEDs, typematic, scancode set).
    ///
    /// Spec: OSDev PS/2 Keyboard — Reset returns the keyboard to power-on
    /// defaults with scanning enabled, LEDs off, default typematic rate/delay,
    /// and scancode set 2. Also clears last-byte tracking used by Resend
    /// (`0xFE`); subsequent ACK/BAT presentation updates it again.
    fn reset_kbd_defaults(&mut self) {
        self.kbd_scanning = true;
        self.kbd_leds = 0;
        self.kbd_typematic = KBD_DEFAULT_TYPEMATIC;
        self.kbd_scancode_set = KBD_DEFAULT_SCANCODE_SET;
        self.kbd_pending_param = KbdPendingParam::None;
        self.kbd_last_byte = 0;
    }

    /// Restore mouse stub defaults (sample rate / resolution / scaling / reporting / mode).
    ///
    /// Spec: OSDev PS/2 Mouse — Reset (`0xFF`) and Set Defaults (`0xF6`) return
    /// the device to power-on defaults (100 reports/sec, 4 counts/mm, scaling
    /// 1:1, data reporting disabled, stream mode, wrap off). Controller
    /// `I8042::reset` uses the same defaults. Set Defaults ACKs without BAT.
    fn reset_mouse_defaults(&mut self) {
        self.mouse_reporting = false;
        self.mouse_sample_rate = MOUSE_DEFAULT_SAMPLE_RATE;
        self.mouse_resolution = MOUSE_DEFAULT_RESOLUTION;
        self.mouse_scaling_21 = false;
        self.mouse_remote_mode = false;
        self.mouse_wrap_mode = false;
        self.mouse_last_dx = 0;
        self.mouse_last_dy = 0;
        self.mouse_last_buttons = 0;
        self.mouse_pending_param = MousePendingParam::None;
        self.mouse_last_byte = 0;
    }

    pub fn reset(&mut self) {
        self.apply_reset_defaults();
    }

    /// Take a latched system-reset request (`0xFE` on `0x64`, or `0xD1` write
    /// with output-port bit0 clear).
    ///
    /// Spec: OSDev I8042 / IBM PC AT — controller pulse-reset or output-port
    /// system-reset line. Returns `true` once per latch; the machine layer
    /// should then run its CPU/device reset path (e.g. `Machine::reset`).
    /// Distinct from keyboard Resend `0xFE` on data port `0x60`.
    pub fn take_system_reset_request(&mut self) -> bool {
        let pending = self.system_reset_pending;
        self.system_reset_pending = false;
        pending
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
    /// Used by tests and keyboard inject. Tracks the byte as the last keyboard
    /// OBF response for Resend (`0xFE`). Returns true if IRQ1 had a rising
    /// edge (false→true) as a result.
    pub fn place_output(&mut self, value: u8) -> bool {
        let prev = self.irq1_line();
        self.present_kbd_byte(value);
        !prev && self.irq1_line()
    }

    /// Last byte presented on the keyboard OBF path (for Resend / tests).
    pub fn kbd_last_byte(&self) -> u8 {
        self.kbd_last_byte
    }

    /// Inject a keyboard scan-code byte into the host output buffer (device → host).
    ///
    /// Spec: OSDev I8042 / IBM PC AT — when the first-port clock is disabled
    /// (config bit4), the keyboard interface ignores device traffic. When
    /// enabled, a byte is placed in the output buffer (OBF) and may raise IRQ1
    /// if config INT1 is set.
    ///
    /// Translation (config bit6): when set, the byte is treated as Scan Code
    /// Set 2 from the device and remapped to Scan Code Set 1 for the host
    /// (OSDev I8042 "Translation"; Brouwer/Konzak table). Set 2 break prefix
    /// `0xF0` is consumed (no OBF byte) and causes the next keyboard byte to
    /// be translated then OR'd with `0x80`. Extended prefix `0xE0` passes
    /// through as `0xE0`. When bit6 is clear, the raw byte is placed unchanged
    /// (any pending break flag is discarded).
    ///
    /// Returns true if IRQ1 had a rising edge (same as [`Self::place_output`]).
    /// Returns false when the clock is disabled, when scanning is disabled
    /// (`0xF5`), or when a translate-mode `0xF0` break prefix is consumed
    /// without presenting a host byte.
    pub fn inject_scancode(&mut self, make_code: u8) -> bool {
        if self.keyboard_clock_disabled() || !self.kbd_scanning {
            return false;
        }
        let host_byte = if self.config & CFG_TRANSLATE != 0 {
            self.translate_set2_to_set1(make_code)
        } else {
            self.translate_pending_break = false;
            Some(make_code)
        };
        match host_byte {
            Some(b) => self.place_output(b),
            None => false,
        }
    }

    /// Apply IBM PC/AT 8042 Set 2 → Set 1 translation for one device byte.
    ///
    /// Spec: Andries Brouwer §10 — `0xF0` becomes "OR next with `0x80`"; other
    /// bytes are looked up in the controller translation table (`SET2_TO_SET1`).
    /// Returns `None` when the byte is consumed without a host OBF write.
    fn translate_set2_to_set1(&mut self, device_byte: u8) -> Option<u8> {
        // Set 2 break prefix: remember and emit nothing (OSDev / Brouwer).
        if device_byte == 0xF0 {
            self.translate_pending_break = true;
            return None;
        }
        let mut host = SET2_TO_SET1[device_byte as usize];
        if self.translate_pending_break {
            host |= 0x80;
            self.translate_pending_break = false;
        }
        Some(host)
    }

    /// Whether the keyboard stub has scanning enabled (`0xF4` / not `0xF5`).
    ///
    /// Spec: OSDev PS/2 Keyboard — Enable/Disable Scanning. When disabled,
    /// [`Self::inject_scancode`] drops make/break traffic.
    pub fn kbd_scanning_enabled(&self) -> bool {
        self.kbd_scanning
    }

    /// Current keyboard LED mask (set by `0xED` + value; default 0 / all off).
    ///
    /// Spec: OSDev PS/2 Keyboard — [`KBD_LED_SCROLL`] / [`KBD_LED_NUM`] /
    /// [`KBD_LED_CAPS`]. Stored only; no host-visible LED hardware. Upper bits
    /// beyond [`KBD_LED_MASK`] are retained if the host wrote them.
    pub fn kbd_leds(&self) -> u8 {
        self.kbd_leds
    }

    /// Whether any of Scroll/Num/Caps LED bits are set in the stored mask.
    pub fn kbd_leds_any(&self) -> bool {
        (self.kbd_leds & KBD_LED_MASK) != 0
    }

    /// Current typematic rate/delay byte (set by `0xF3` + value; default
    /// [`KBD_DEFAULT_TYPEMATIC`]).
    pub fn kbd_typematic(&self) -> u8 {
        self.kbd_typematic
    }

    /// Current scancode set number (1/2/3; set by `0xF0` + value; default
    /// [`KBD_DEFAULT_SCANCODE_SET`]).
    ///
    /// Spec: OSDev PS/2 Keyboard — Get/Set Scancode Set. Stored only; does not
    /// change [`Self::inject_scancode`] encoding (Set2 stream + optional bit6
    /// translation). Get-current responses are the raw set number (not the
    /// translated `0x43`/`0x41`/`0x3F` values the 8042 would produce when
    /// config bit6 is set).
    pub fn kbd_scancode_set(&self) -> u8 {
        self.kbd_scancode_set
    }

    /// Whether the mouse stub has data reporting enabled (`0xF4` / not `0xF5`).
    ///
    /// Spec: OSDev PS/2 Mouse — Enable/Disable Data Reporting. When enabled,
    /// [`Self::inject_mouse_packet`] may queue stream-mode movement packets.
    pub fn mouse_reporting_enabled(&self) -> bool {
        self.mouse_reporting
    }

    /// Encode and queue a standard 3-byte PS/2 mouse movement packet.
    ///
    /// Spec: OSDev [PS/2 Mouse](https://wiki.osdev.org/PS/2_Mouse) "Mouse Packet
    /// Format" — byte0: bit0 Left / bit1 Right / bit2 Middle / bit3 always 1 /
    /// bit4 X sign / bit5 Y sign / bit6 X overflow / bit7 Y overflow; byte1 X
    /// movement; byte2 Y movement. Deltas use the 9-bit signed range
    /// (−256…+255); values outside that set the corresponding overflow bit and
    /// clamp the movement byte.
    ///
    /// Returns `true` if IRQ12 had a rising edge as a result of presenting the
    /// first byte. Returns `false` (and **drops** the packet) when data
    /// reporting is disabled (`0xF5` / Reset default), when the aux response
    /// queue cannot hold three more bytes, or when no IRQ12 edge occurred.
    ///
    /// Aux clock disable (config bit5) holds queued bytes until `0xA8` (same as
    /// command responses); it does not drop an accepted packet.
    pub fn inject_mouse_packet(&mut self, dx: i16, dy: i16, buttons: u8) -> bool {
        if !self.mouse_reporting {
            return false;
        }
        if (self.aux_resp_len as usize) + 3 > AUX_RESP_QUEUE_CAP {
            return false;
        }
        // Remember for Read Data (`0xEB`) even after the stream packet is drained.
        self.mouse_last_dx = dx;
        self.mouse_last_dy = dy;
        self.mouse_last_buttons = buttons;
        let packet = encode_mouse_packet(dx, dy, buttons);
        let prev = self.irq12_line();
        self.push_aux_bytes(&packet);
        self.flush_aux_response_queue();
        !prev && self.irq12_line()
    }

    /// Current mouse sample rate (set by `0xF3` + value; default 100).
    pub fn mouse_sample_rate(&self) -> u8 {
        self.mouse_sample_rate
    }

    /// Current mouse resolution encoding 0..3 (set by `0xE8` + value; default 2).
    pub fn mouse_resolution(&self) -> u8 {
        self.mouse_resolution
    }

    /// Whether scaling 2:1 is active (`0xE7`); false means 1:1 (`0xE6` / Reset).
    pub fn mouse_scaling_21(&self) -> bool {
        self.mouse_scaling_21
    }

    /// Whether remote mode is active (`0xF0`); false means stream (`0xEA` / Reset).
    ///
    /// Spec: OSDev PS/2 Mouse — Set Remote Mode / Set Stream Mode.
    pub fn mouse_remote_mode(&self) -> bool {
        self.mouse_remote_mode
    }

    /// Whether wrap mode is active (`0xEE`); cleared by Reset / Reset Wrap
    /// (`0xEC`), and by Stream / Remote / Set Defaults when those commands are
    /// executed (they are only echoed while wrap is active).
    ///
    /// Spec: OSDev PS/2 Mouse — Set/Reset Wrap Mode; while set, host→aux bytes
    /// other than `0xEC`/`0xFF` are echoed on AUX OBF (no ACK).
    pub fn mouse_wrap_mode(&self) -> bool {
        self.mouse_wrap_mode
    }

    /// Build Status Request byte 1 from current mouse stub state.
    ///
    /// Spec: OSDev [PS/2 Mouse](https://wiki.osdev.org/PS/2_Mouse) Status Request
    /// — bit0 Right, bit1 Middle, bit2 Left, bit3 always 0, bit4 Scaling (2:1),
    /// bit5 Enable (reporting), bit6 Mode (remote when set; stream when clear),
    /// bit7 always 0. Buttons are never pressed in this stub.
    fn mouse_status_byte1(&self) -> u8 {
        let mut b = 0u8;
        if self.mouse_scaling_21 {
            b |= MOUSE_STATUS_SCALING;
        }
        if self.mouse_reporting {
            b |= MOUSE_STATUS_ENABLE;
        }
        if self.mouse_remote_mode {
            b |= MOUSE_STATUS_REMOTE;
        }
        b
    }

    /// Inject an auxiliary-device (second PS/2 port) byte into the output buffer.
    ///
    /// Spec: IBM PS/2 keyboard controller — aux data sets OBF *and* AUX OBF
    /// (status bit 5) and raises IRQ12 when config bit 1 is set. The aux clock
    /// disable (config bit 5) inhibits the interface, so the byte is dropped.
    ///
    /// Used by the mouse stub and by tests. Returns true if IRQ12 had a rising
    /// edge (false→true) as a result.
    pub fn inject_aux_byte(&mut self, value: u8) -> bool {
        if self.aux_clock_disabled() {
            return false;
        }
        let prev = self.irq12_line();
        self.mouse_last_byte = value;
        self.output = Some(value);
        self.output_from_aux = true;
        !prev && self.irq12_line()
    }

    fn push_output(&mut self, value: u8) {
        self.output = Some(value);
        self.output_from_aux = false;
    }

    /// Present a keyboard-path byte on OBF and record it for Resend (`0xFE`).
    ///
    /// Spec: OSDev PS/2 Keyboard — Resend requeues the last byte delivered on
    /// the keyboard output path (ACK, ID, Echo, BAT, inject scancodes, …).
    /// Controller-only responses use [`Self::push_output`] and do not update
    /// [`Self::kbd_last_byte`].
    fn present_kbd_byte(&mut self, value: u8) {
        self.kbd_last_byte = value;
        self.push_output(value);
    }

    /// Pop the output buffer, clearing OBF and AUX OBF (`0x60` read), then
    /// present the next queued controller, keyboard, or mouse response byte if any.
    fn take_output(&mut self) -> Option<u8> {
        let v = self.output.take();
        self.output_from_aux = false;
        self.flush_ctrl_response_queue();
        self.flush_kbd_response_queue();
        self.flush_aux_response_queue();
        v
    }

    /// Append bytes to the controller → host queue (does not present).
    fn push_ctrl_bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            if (self.ctrl_resp_len as usize) < CTRL_RESP_QUEUE_CAP {
                self.ctrl_resp[self.ctrl_resp_len as usize] = b;
                self.ctrl_resp_len = self.ctrl_resp_len.saturating_add(1);
            }
        }
    }

    /// Queue a controller command response (replaces any pending ctrl queue) and
    /// present the first byte when the buffer is free.
    ///
    /// Spec: OSDev I8042 — controller responses (self-test, port tests, dump)
    /// use the normal output buffer and are not inhibited by keyboard/aux clock
    /// disable bits.
    fn begin_ctrl_response(&mut self, bytes: &[u8]) {
        self.ctrl_resp_len = 0;
        self.push_ctrl_bytes(bytes);
        self.flush_ctrl_response_queue();
    }

    fn pop_ctrl_response(&mut self) -> Option<u8> {
        if self.ctrl_resp_len == 0 {
            return None;
        }
        let b = self.ctrl_resp[0];
        let n = self.ctrl_resp_len as usize;
        self.ctrl_resp.copy_within(1..n, 0);
        self.ctrl_resp_len -= 1;
        Some(b)
    }

    /// Present the next queued controller response on normal OBF when the
    /// buffer is empty (not gated by keyboard/aux clock).
    fn flush_ctrl_response_queue(&mut self) {
        if self.output.is_some() {
            return;
        }
        if let Some(b) = self.pop_ctrl_response() {
            self.push_output(b);
        }
    }

    /// Append bytes to the keyboard → host queue (does not present).
    fn push_kbd_bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            if (self.kbd_resp_len as usize) < KBD_RESP_QUEUE_CAP {
                self.kbd_resp[self.kbd_resp_len as usize] = b;
                self.kbd_resp_len = self.kbd_resp_len.saturating_add(1);
            }
        }
    }

    /// Queue a keyboard command response (replaces any pending kbd queue) and
    /// present the first byte when the buffer is free and the keyboard clock
    /// is enabled.
    fn begin_kbd_response(&mut self, bytes: &[u8]) {
        self.kbd_resp_len = 0;
        self.push_kbd_bytes(bytes);
        self.flush_kbd_response_queue();
    }

    fn pop_kbd_response(&mut self) -> Option<u8> {
        if self.kbd_resp_len == 0 {
            return None;
        }
        let b = self.kbd_resp[0];
        let n = self.kbd_resp_len as usize;
        self.kbd_resp.copy_within(1..n, 0);
        self.kbd_resp_len -= 1;
        Some(b)
    }

    /// Present the next queued keyboard response on keyboard OBF when the
    /// buffer is empty and the keyboard clock is enabled.
    fn flush_kbd_response_queue(&mut self) {
        if self.output.is_some() || self.keyboard_clock_disabled() {
            return;
        }
        if let Some(b) = self.pop_kbd_response() {
            self.present_kbd_byte(b);
        }
    }

    /// Append bytes to the aux → host queue (does not present).
    fn push_aux_bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            if (self.aux_resp_len as usize) < AUX_RESP_QUEUE_CAP {
                self.aux_resp[self.aux_resp_len as usize] = b;
                self.aux_resp_len = self.aux_resp_len.saturating_add(1);
            }
        }
    }

    /// Queue a mouse command response (replaces any pending aux queue) and
    /// present the first byte when the buffer is free.
    fn begin_mouse_response(&mut self, bytes: &[u8]) {
        self.aux_resp_len = 0;
        self.push_aux_bytes(bytes);
        self.flush_aux_response_queue();
    }

    fn pop_aux_response(&mut self) -> Option<u8> {
        if self.aux_resp_len == 0 {
            return None;
        }
        let b = self.aux_resp[0];
        let n = self.aux_resp_len as usize;
        self.aux_resp.copy_within(1..n, 0);
        self.aux_resp_len -= 1;
        Some(b)
    }

    /// Present the next queued aux response on AUX OBF when the buffer is empty
    /// and the aux clock is enabled.
    fn flush_aux_response_queue(&mut self) {
        if self.output.is_some() || self.aux_clock_disabled() {
            return;
        }
        if let Some(b) = self.pop_aux_response() {
            self.mouse_last_byte = b;
            self.output = Some(b);
            self.output_from_aux = true;
        }
    }

    /// Queue a host→aux byte accepted while the aux clock was disabled.
    ///
    /// Spec: OSDev I8042 — disabled second-port clock inhibits the interface;
    /// this emulator defers `0xD4` bytes in a FIFO until the clock is re-enabled.
    /// Overflow silently drops the new byte.
    fn enqueue_host_aux(&mut self, byte: u8) {
        if (self.host_aux_len as usize) < HOST_AUX_QUEUE_CAP {
            self.host_aux[self.host_aux_len as usize] = byte;
            self.host_aux_len = self.host_aux_len.saturating_add(1);
        }
    }

    fn pop_host_aux(&mut self) -> Option<u8> {
        if self.host_aux_len == 0 {
            return None;
        }
        let b = self.host_aux[0];
        let n = self.host_aux_len as usize;
        self.host_aux.copy_within(1..n, 0);
        self.host_aux_len -= 1;
        Some(b)
    }

    /// Deliver queued host→aux bytes to the mouse stub (aux clock must be enabled).
    fn flush_host_aux_queue(&mut self) {
        if self.aux_clock_disabled() {
            return;
        }
        while let Some(b) = self.pop_host_aux() {
            self.handle_aux_device_byte(b);
        }
    }

    /// Aux clock just transitioned disabled→enabled: deliver deferred host→aux
    /// bytes, then present any held device→host responses.
    fn on_aux_clock_enabled(&mut self) {
        self.flush_host_aux_queue();
        self.flush_aux_response_queue();
    }

    /// Handle a host→auxiliary-device byte routed by controller command `0xD4`.
    ///
    /// Spec: OSDev [PS/2 Mouse](https://wiki.osdev.org/PS/2_Mouse) "Mouse
    /// Commands" + IBM PS/2 KBC `0xD4` routing. Caller must only invoke this when
    /// the aux clock is enabled (or when flushing the host→aux queue after
    /// re-enable). Supported stub commands answer with ACK/`0xFA` (and BAT/ID /
    /// status bytes where required) on AUX OBF. `0xF3` / `0xE8` arm a one-byte
    /// parameter expected on the next `0xD4`.
    fn handle_aux_device_byte(&mut self, cmd: u8) {
        self.last_aux_device_write = Some(cmd);
        self.aux_device_writes = self.aux_device_writes.saturating_add(1);

        // Spec: OSDev PS/2 Mouse wrap mode — while wrap is set, every host→aux
        // byte is echoed on AUX OBF (no ACK) except Reset Wrap (`0xEC`) and
        // Reset (`0xFF`), which remain commands.
        if self.mouse_wrap_mode && cmd != MOUSE_CMD_RESET_WRAP_MODE && cmd != MOUSE_CMD_RESET {
            self.begin_mouse_response(&[cmd]);
            return;
        }

        match self.mouse_pending_param {
            MousePendingParam::SampleRate => {
                self.mouse_pending_param = MousePendingParam::None;
                self.mouse_sample_rate = cmd;
                self.begin_mouse_response(&[MOUSE_ACK]);
                return;
            }
            MousePendingParam::Resolution => {
                self.mouse_pending_param = MousePendingParam::None;
                // Spec: resolution argument is 0..3; store as given (firmware probe).
                self.mouse_resolution = cmd;
                self.begin_mouse_response(&[MOUSE_ACK]);
                return;
            }
            MousePendingParam::None => {}
        }

        match cmd {
            MOUSE_CMD_RESET => {
                // Spec: Reset → ACK, BAT OK, standard mouse ID; restore defaults.
                self.reset_mouse_defaults();
                self.begin_mouse_response(&[MOUSE_ACK, MOUSE_BAT_OK, MOUSE_ID_STANDARD]);
            }
            MOUSE_CMD_GET_DEVICE_ID => {
                self.begin_mouse_response(&[MOUSE_ACK, MOUSE_ID_STANDARD]);
            }
            MOUSE_CMD_ENABLE_REPORTING => {
                self.mouse_reporting = true;
                self.begin_mouse_response(&[MOUSE_ACK]);
            }
            MOUSE_CMD_DISABLE_REPORTING => {
                self.mouse_reporting = false;
                self.begin_mouse_response(&[MOUSE_ACK]);
            }
            MOUSE_CMD_SET_DEFAULTS => {
                // Spec: OSDev PS/2 Mouse — Set Defaults (`0xF6`) → ACK and
                // restore power-on defaults; unlike Reset, no BAT/ID.
                self.reset_mouse_defaults();
                self.begin_mouse_response(&[MOUSE_ACK]);
            }
            MOUSE_CMD_SET_SAMPLE_RATE => {
                // Spec: Set Sample Rate — ACK, then accept rate on next `0xD4`.
                self.mouse_pending_param = MousePendingParam::SampleRate;
                self.begin_mouse_response(&[MOUSE_ACK]);
            }
            MOUSE_CMD_SET_RESOLUTION => {
                // Spec: Set Resolution — ACK, then accept value on next `0xD4`.
                self.mouse_pending_param = MousePendingParam::Resolution;
                self.begin_mouse_response(&[MOUSE_ACK]);
            }
            MOUSE_CMD_STATUS_REQUEST => {
                // Spec: Status Request → ACK + flags + resolution + sample rate.
                let status1 = self.mouse_status_byte1();
                let res = self.mouse_resolution;
                let rate = self.mouse_sample_rate;
                self.begin_mouse_response(&[MOUSE_ACK, status1, res, rate]);
            }
            MOUSE_CMD_SET_SCALING_1_1 => {
                self.mouse_scaling_21 = false;
                self.begin_mouse_response(&[MOUSE_ACK]);
            }
            MOUSE_CMD_SET_SCALING_2_1 => {
                self.mouse_scaling_21 = true;
                self.begin_mouse_response(&[MOUSE_ACK]);
            }
            MOUSE_CMD_RESEND => {
                // Spec: OSDev PS/2 Mouse — Resend last byte on AUX OBF.
                let last = self.mouse_last_byte;
                self.begin_mouse_response(&[last]);
            }
            MOUSE_CMD_SET_STREAM_MODE => {
                // Spec: OSDev PS/2 Mouse — Set Stream Mode (`0xEA`) → ACK;
                // clears remote and wrap.
                self.mouse_remote_mode = false;
                self.mouse_wrap_mode = false;
                self.begin_mouse_response(&[MOUSE_ACK]);
            }
            MOUSE_CMD_SET_REMOTE_MODE => {
                // Spec: OSDev PS/2 Mouse — Set Remote Mode (`0xF0`) → ACK;
                // clears wrap.
                self.mouse_remote_mode = true;
                self.mouse_wrap_mode = false;
                self.begin_mouse_response(&[MOUSE_ACK]);
            }
            MOUSE_CMD_SET_WRAP_MODE => {
                // Spec: OSDev PS/2 Mouse — Set Wrap Mode (`0xEE`) → ACK + flag.
                // Subsequent host→aux bytes are echoed until Reset Wrap / Reset.
                self.mouse_wrap_mode = true;
                self.begin_mouse_response(&[MOUSE_ACK]);
            }
            MOUSE_CMD_RESET_WRAP_MODE => {
                // Spec: OSDev PS/2 Mouse — Reset Wrap Mode (`0xEC`) → ACK;
                // clear wrap; stream/remote mode unchanged.
                self.mouse_wrap_mode = false;
                self.begin_mouse_response(&[MOUSE_ACK]);
            }
            MOUSE_CMD_READ_DATA => {
                // Spec: OSDev PS/2 Mouse — Read Data (`0xEB`) → ACK then one
                // 3-byte movement packet (last inject, or zeros). Answer in both
                // remote and stream mode (SeaBIOS/OSDev-friendly stub).
                let packet = encode_mouse_packet(
                    self.mouse_last_dx,
                    self.mouse_last_dy,
                    self.mouse_last_buttons,
                );
                self.begin_mouse_response(&[MOUSE_ACK, packet[0], packet[1], packet[2]]);
            }
            _ => {
                // Unsupported mouse command: recorded, no response (see module docs).
            }
        }
    }

    /// Handle a host→keyboard byte written to the data port with no pending
    /// controller command parameter.
    ///
    /// Spec: OSDev [PS/2 Keyboard](https://wiki.osdev.org/PS/2_Keyboard) —
    /// commands on `0x60` when the controller is not expecting a write-config /
    /// write-output / write-aux parameter. Supported stub commands answer with
    /// ACK/`0xFA` (and BAT/ID/set number where required) on keyboard OBF.
    /// `0xED` / `0xF3` / `0xF0` arm a one-byte parameter expected on the next
    /// data-port write. Echo (`0xEE`) answers with `0xEE` (no separate ACK).
    /// Resend (`0xFE`) requeues [`Self::kbd_last_byte`] (honest default `0`
    /// until the first keyboard OBF presentation).
    fn handle_kbd_device_byte(&mut self, cmd: u8) {
        match self.kbd_pending_param {
            KbdPendingParam::Leds => {
                self.kbd_pending_param = KbdPendingParam::None;
                self.kbd_leds = cmd;
                self.begin_kbd_response(&[KBD_ACK]);
                return;
            }
            KbdPendingParam::Typematic => {
                self.kbd_pending_param = KbdPendingParam::None;
                self.kbd_typematic = cmd;
                self.begin_kbd_response(&[KBD_ACK]);
                return;
            }
            KbdPendingParam::ScancodeSet => {
                self.kbd_pending_param = KbdPendingParam::None;
                match cmd {
                    KBD_SCANCODE_GET => {
                        // Spec: get current → ACK then set number (raw 1/2/3).
                        let set = self.kbd_scancode_set;
                        self.begin_kbd_response(&[KBD_ACK, set]);
                    }
                    1..=3 => {
                        self.kbd_scancode_set = cmd;
                        self.begin_kbd_response(&[KBD_ACK]);
                    }
                    _ => {
                        // Invalid set number: accepted, no response (see module docs).
                    }
                }
                return;
            }
            KbdPendingParam::KeyTypeScancode => {
                // Spec: OSDev — Set Key Type takes one scancode; ACK and ignore.
                self.kbd_pending_param = KbdPendingParam::None;
                let _scancode = cmd;
                self.begin_kbd_response(&[KBD_ACK]);
                return;
            }
            KbdPendingParam::None => {}
        }

        match cmd {
            KBD_CMD_RESET => {
                // Spec: Reset → ACK, BAT OK; restore scanning / LEDs / typematic / set.
                self.reset_kbd_defaults();
                self.begin_kbd_response(&[KBD_ACK, KBD_BAT_OK]);
            }
            KBD_CMD_GET_ID => {
                // Spec: Get Keyboard ID → ACK then MF2 ID bytes `0xAB` `0x83`.
                self.begin_kbd_response(&[KBD_ACK, KBD_ID_MF2_0, KBD_ID_MF2_1]);
            }
            KBD_CMD_ENABLE_SCANNING => {
                self.kbd_scanning = true;
                self.begin_kbd_response(&[KBD_ACK]);
            }
            KBD_CMD_DISABLE_SCANNING => {
                self.kbd_scanning = false;
                self.begin_kbd_response(&[KBD_ACK]);
            }
            KBD_CMD_SET_LEDS => {
                // Spec: Set LEDs — ACK, then accept LED mask on next data write.
                self.kbd_pending_param = KbdPendingParam::Leds;
                self.begin_kbd_response(&[KBD_ACK]);
            }
            KBD_CMD_SET_TYPEMATIC => {
                // Spec: Set Typematic Rate/Delay — ACK, then accept value next.
                self.kbd_pending_param = KbdPendingParam::Typematic;
                self.begin_kbd_response(&[KBD_ACK]);
            }
            KBD_CMD_SET_SCANCODE_SET => {
                // Spec: Get/Set Scancode Set — ACK, then accept sub-command next.
                self.kbd_pending_param = KbdPendingParam::ScancodeSet;
                self.begin_kbd_response(&[KBD_ACK]);
            }
            KBD_CMD_ECHO => {
                // Spec: Echo — respond with Echo `0xEE` (not a separate ACK).
                self.begin_kbd_response(&[KBD_ECHO]);
            }
            KBD_CMD_RESEND => {
                // Spec: OSDev PS/2 Keyboard — Resend last byte on keyboard OBF.
                // Distinct from controller pulse-reset `0xFE` on port `0x64`.
                let last = self.kbd_last_byte;
                self.begin_kbd_response(&[last]);
            }
            KBD_CMD_SET_DEFAULT => {
                // Spec: OSDev PS/2 Keyboard — Set Default (`0xF6`): restore
                // power-on defaults (scanning on, LEDs 0, typematic default,
                // scancode set 2) and ACK. Unlike Reset, no BAT `0xAA` byte.
                self.reset_kbd_defaults();
                self.begin_kbd_response(&[KBD_ACK]);
            }
            KBD_CMD_SET_ALL_TYPEMATIC => {
                // Spec: OSDev PS/2 Keyboard — Set All Keys Typematic (`0xF7`):
                // ACK only; key-type state not modeled beyond acceptance.
                self.begin_kbd_response(&[KBD_ACK]);
            }
            KBD_CMD_SET_ALL_MAKE_BREAK => {
                // Spec: OSDev PS/2 Keyboard — Set All Keys Make/Break (`0xF8`).
                self.begin_kbd_response(&[KBD_ACK]);
            }
            KBD_CMD_SET_ALL_MAKE => {
                // Spec: OSDev PS/2 Keyboard — Set All Keys Make (`0xF9`).
                self.begin_kbd_response(&[KBD_ACK]);
            }
            KBD_CMD_SET_ALL_TYPEMATIC_MAKE_BREAK => {
                // Spec: OSDev PS/2 Keyboard — Set All Keys Typematic/Make/Break
                // (`0xFA`): ACK only; key-type state not modeled beyond acceptance.
                // Opcode value equals [`KBD_ACK`]; this is the host→device command.
                self.begin_kbd_response(&[KBD_ACK]);
            }
            KBD_CMD_SET_KEY_TYPEMATIC | KBD_CMD_SET_KEY_MAKE_BREAK | KBD_CMD_SET_KEY_MAKE => {
                // Spec: OSDev — ACK, then accept one scancode param (ignored).
                self.kbd_pending_param = KbdPendingParam::KeyTypeScancode;
                self.begin_kbd_response(&[KBD_ACK]);
            }
            _ => {
                // Unsupported keyboard command: accepted, no ACK (see module docs).
            }
        }
    }
}

/// Build a standard 3-byte stream-mode mouse packet (OSDev Mouse Packet Format).
fn encode_mouse_packet(dx: i16, dy: i16, buttons: u8) -> [u8; 3] {
    let (dx_b, x_sign, x_ovf) = pack_mouse_axis(dx);
    let (dy_b, y_sign, y_ovf) = pack_mouse_axis(dy);
    let mut flags =
        MOUSE_PKT_ALWAYS1 | (buttons & (MOUSE_BTN_LEFT | MOUSE_BTN_RIGHT | MOUSE_BTN_MIDDLE));
    if x_sign {
        flags |= MOUSE_PKT_X_SIGN;
    }
    if y_sign {
        flags |= MOUSE_PKT_Y_SIGN;
    }
    if x_ovf {
        flags |= MOUSE_PKT_X_OVERFLOW;
    }
    if y_ovf {
        flags |= MOUSE_PKT_Y_OVERFLOW;
    }
    [flags, dx_b, dy_b]
}

/// Pack one movement axis into (byte, sign, overflow) for the 9-bit signed range.
fn pack_mouse_axis(delta: i16) -> (u8, bool, bool) {
    if delta > 255 {
        (0xFF, false, true)
    } else if delta < -256 {
        (0x00, true, true)
    } else {
        (delta as u8, delta < 0, false)
    }
}

impl I8042 {
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
                // Spec: re-enable aux clock — deliver queued host→aux `0xD4`
                // bytes, then present any held mouse responses.
                self.on_aux_clock_enabled();
            }
            CMD_TEST_AUX => {
                // Spec: IBM PS/2 — controller response on the normal output
                // buffer (OBF, not AUX OBF); 0x00 = no error detected.
                self.push_output(TEST_AUX_OK);
            }
            CMD_TEST_KBD => {
                // Spec: OSDev I8042 / IBM PS/2 — test first (keyboard) port;
                // same result codes as 0xA9; response on normal OBF (not AUX).
                self.push_output(TEST_KBD_OK);
            }
            CMD_DIAG_DUMP => {
                // Spec: OSDev I8042 — `0xAC` diagnostic dump (internal RAM).
                // Stub: fixed DIAG_DUMP_LEN zero bytes on normal OBF (not AUX),
                // one per 0x60 read; not gated by keyboard clock disable.
                self.begin_ctrl_response(&DIAG_DUMP_STUB);
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
                // Spec: re-enable keyboard clock — present any held kbd responses.
                self.flush_kbd_response_queue();
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
            CMD_PULSE_RESET => {
                // Spec: OSDev I8042 — pulse output lines / system-reset request.
                // Latch for Machine::reset; distinct from keyboard Resend on 0x60.
                self.pulse_reset_commands = self.pulse_reset_commands.saturating_add(1);
                self.system_reset_pending = true;
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
                        let was_disabled = self.aux_clock_disabled();
                        self.config = v;
                        self.pending = PendingWrite::None;
                        // Spec: clearing config bit5 re-enables aux clock —
                        // same flush as `0xA8`.
                        if was_disabled && !self.aux_clock_disabled() {
                            self.on_aux_clock_enabled();
                        }
                    }
                    PendingWrite::OutputPort => {
                        // Spec: IBM PC AT / OSDev I8042 — bit1 = A20 gate;
                        // bit0 = system reset (active low). Writing bit0 clear
                        // latches the same system-reset request as pulse-reset
                        // `0xFE` on `0x64` (Machine::service_8042_pulse_reset).
                        self.output_port = v;
                        if v & OUTPUT_PORT_SYSTEM_RESET == 0 {
                            self.system_reset_pending = true;
                        }
                        self.pending = PendingWrite::None;
                    }
                    PendingWrite::AuxDevice => {
                        // Spec: IBM PS/2 `0xD4` / OSDev I8042 — when aux clock
                        // is enabled, the byte is sent to the aux device (mouse
                        // stub). When config bit5 / `0xA7` disables the aux
                        // clock, the interface is inhibited: queue the byte for
                        // delivery on re-enable (`0xA8` / config clear bit5).
                        // The pending write is still consumed. Device→host
                        // responses already queued remain held until enable.
                        if self.aux_clock_disabled() {
                            self.enqueue_host_aux(v);
                        } else {
                            self.handle_aux_device_byte(v);
                        }
                        self.pending = PendingWrite::None;
                    }
                    PendingWrite::None => {
                        // Spec: OSDev PS/2 Keyboard — data-port write with no
                        // pending controller parameter is a host→keyboard command.
                        self.handle_kbd_device_byte(v);
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

    /// Spec: unsupported host→keyboard commands (e.g. `0xF1`) are accepted with
    /// no ACK; controller config/output buffer unchanged.
    #[test]
    fn data_write_unsupported_kbd_command_no_ack() {
        let mut k = I8042::new();
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_ENABLE_KBD));
        let before = k.clone();
        k.port_write(I8042_DATA, 1, 0xF1); // unimplemented keyboard opcode
        assert_eq!(k.config, before.config);
        assert_eq!(k.output_buffer(), None);
        assert_eq!(k.status() & STATUS_OBF, 0);
    }

    /// Spec: OSDev PS/2 Keyboard — Set All Keys Typematic (`0xF7`) → ACK `0xFA`.
    #[test]
    fn kbd_set_all_typematic_f7_acks() {
        let mut k = I8042::new();
        write_kbd(&mut k, KBD_CMD_SET_ALL_TYPEMATIC);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        assert_eq!(k.status() & STATUS_OBF, 0);
    }

    /// Spec: OSDev PS/2 Keyboard — Set All Keys Make/Break (`0xF8`) + Make (`0xF9`).
    #[test]
    fn kbd_set_all_make_break_f8_and_make_f9_ack() {
        let mut k = I8042::new();
        write_kbd(&mut k, KBD_CMD_SET_ALL_MAKE_BREAK);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        write_kbd(&mut k, KBD_CMD_SET_ALL_MAKE);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        assert_eq!(k.status() & STATUS_OBF, 0);
    }

    /// Spec: OSDev PS/2 Keyboard — Set All Keys Typematic/Make/Break (`0xFA`) → ACK.
    ///
    /// Opcode `0xFA` equals the ACK response value; host→device vs device→host
    /// distinguishes them. Get Scancode Set (`0xF0`+`0`) already exists.
    #[test]
    fn kbd_set_all_typematic_make_break_fa_acks() {
        let mut k = I8042::new();
        write_kbd(&mut k, KBD_CMD_SET_ALL_TYPEMATIC_MAKE_BREAK);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        assert_eq!(k.status() & STATUS_OBF, 0);
        assert_eq!(k.status() & STATUS_AUX_OBF, 0);
        // Defaults unchanged (ACK-only; no key-type state modeled).
        assert!(k.kbd_scanning_enabled());
        assert_eq!(k.kbd_typematic(), KBD_DEFAULT_TYPEMATIC);
        assert_eq!(k.kbd_scancode_set(), KBD_DEFAULT_SCANCODE_SET);
    }

    /// Spec: OSDev PS/2 Keyboard — Set Key Type `0xFB`/`0xFC`/`0xFD` ACK + scancode.
    #[test]
    fn kbd_set_key_type_fb_fc_fd_ack_with_scancode_param() {
        let mut k = I8042::new();
        for cmd in [
            KBD_CMD_SET_KEY_TYPEMATIC,
            KBD_CMD_SET_KEY_MAKE_BREAK,
            KBD_CMD_SET_KEY_MAKE,
        ] {
            write_kbd(&mut k, cmd);
            assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
            write_kbd(&mut k, 0x1C); // ignored scancode (A make)
            assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        }
        assert_eq!(k.status() & STATUS_OBF, 0);
    }

    /// Spec: OSDev PS/2 Keyboard — Set Default (`0xF6`) restores defaults and
    /// ACKs without a BAT byte (unlike Reset `0xFF`).
    #[test]
    fn kbd_set_default_f6_acks_and_restores_defaults() {
        let mut k = I8042::new();
        // Dirty state away from defaults.
        write_kbd(&mut k, KBD_CMD_DISABLE_SCANNING);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        write_kbd(&mut k, KBD_CMD_SET_LEDS);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        write_kbd(&mut k, 0x07);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        assert!(!k.kbd_scanning_enabled());
        assert_eq!(k.kbd_leds(), 0x07);

        write_kbd(&mut k, KBD_CMD_SET_DEFAULT);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        assert!(k.kbd_scanning_enabled(), "Set Default re-enables scanning");
        assert_eq!(k.kbd_leds(), 0);
        assert_eq!(k.kbd_typematic(), KBD_DEFAULT_TYPEMATIC);
        assert_eq!(k.kbd_scancode_set(), KBD_DEFAULT_SCANCODE_SET);
        // No BAT on OBF after Set Default.
        assert_eq!(k.status() & STATUS_OBF, 0);
    }

    /// Spec: OSDev PS/2 Keyboard — Resend (`0xFE`) requeues the last keyboard
    /// OBF byte. After Get ID, `kbd_last_byte` updates as each response byte is
    /// presented on OBF (FA on OBF → last FA; reading FA auto-presents AB →
    /// last AB; reading AB presents 83 → last 83).
    #[test]
    fn kbd_resend_fe_requeues_last_get_id_byte() {
        let mut k = I8042::new();
        write_kbd(&mut k, KBD_CMD_GET_ID);
        // FA presented on OBF — last is ACK before host read.
        assert_eq!(k.kbd_last_byte(), KBD_ACK);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        // Reading FA presents AB → last updates to first ID byte.
        assert_eq!(k.kbd_last_byte(), KBD_ID_MF2_0);
        assert_eq!(read_kbd_byte(&mut k), KBD_ID_MF2_0);
        assert_eq!(k.kbd_last_byte(), KBD_ID_MF2_1);
        assert_eq!(read_kbd_byte(&mut k), KBD_ID_MF2_1);
        assert_eq!(k.kbd_last_byte(), KBD_ID_MF2_1);
        assert_eq!(k.status() & STATUS_OBF, 0);

        // Resend requeues the last presented ID byte (`0x83`).
        write_kbd(&mut k, KBD_CMD_RESEND);
        assert_eq!(read_kbd_byte(&mut k), KBD_ID_MF2_1);

        // After a one-byte ACK response, Resend returns FA again.
        write_kbd(&mut k, KBD_CMD_ENABLE_SCANNING);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        assert_eq!(k.kbd_last_byte(), KBD_ACK);
        write_kbd(&mut k, KBD_CMD_RESEND);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);
    }

    /// Spec: OSDev PS/2 Keyboard — after Echo (`0xEE`), Resend returns `0xEE`
    /// again. With no prior keyboard OBF byte, Resend honestly requeues `0x00`.
    #[test]
    fn kbd_resend_fe_after_echo_and_default_zero() {
        let mut k = I8042::new();
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_ENABLE_KBD));
        assert_eq!(k.kbd_last_byte(), 0);
        k.port_write(I8042_DATA, 1, u32::from(KBD_CMD_RESEND));
        assert_eq!(read_kbd_byte(&mut k), 0x00);

        write_kbd(&mut k, KBD_CMD_ECHO);
        assert_eq!(read_kbd_byte(&mut k), KBD_ECHO);
        assert_eq!(k.kbd_last_byte(), KBD_ECHO);
        write_kbd(&mut k, KBD_CMD_RESEND);
        assert_eq!(read_kbd_byte(&mut k), KBD_ECHO);
        write_kbd(&mut k, KBD_CMD_RESEND);
        assert_eq!(read_kbd_byte(&mut k), KBD_ECHO);
    }

    /// Spec: OSDev I8042 — controller command `0xFE` on port `0x64` is pulse-
    /// reset (counted + latches system-reset request), not keyboard Resend on
    /// `0x60`.
    #[test]
    fn controller_cmd_fe_on_64_counts_pulse_reset() {
        let mut k = I8042::new();
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_ENABLE_KBD));
        write_kbd(&mut k, KBD_CMD_ECHO);
        assert_eq!(read_kbd_byte(&mut k), KBD_ECHO);
        let before_unsup = k.unsupported_commands;
        let before_pulse = k.pulse_reset_commands;
        assert!(!k.take_system_reset_request());
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_PULSE_RESET));
        assert_eq!(k.pulse_reset_commands, before_pulse + 1);
        assert_eq!(k.unsupported_commands, before_unsup);
        assert_eq!(k.status() & STATUS_OBF, 0);
        assert_eq!(k.kbd_last_byte(), KBD_ECHO); // keyboard last byte untouched
        assert!(k.take_system_reset_request());
        assert!(!k.take_system_reset_request());
    }

    /// Spec: OSDev PS/2 Keyboard — Resend `0xFE` on data port does not latch
    /// controller pulse-reset / system-reset.
    #[test]
    fn kbd_resend_fe_on_60_does_not_latch_system_reset() {
        let mut k = I8042::new();
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_ENABLE_KBD));
        write_kbd(&mut k, KBD_CMD_ECHO);
        assert_eq!(read_kbd_byte(&mut k), KBD_ECHO);
        let before_pulse = k.pulse_reset_commands;
        write_kbd(&mut k, KBD_CMD_RESEND);
        assert_eq!(read_kbd_byte(&mut k), KBD_ECHO);
        assert_eq!(k.pulse_reset_commands, before_pulse);
        assert!(!k.take_system_reset_request());
    }

    /// Spec: OSDev PS/2 Keyboard — Reset (`0xFF`) clears last-byte tracking to
    /// `0`, then ACK/BAT presentation updates it (after ACK last=`FA`, after
    /// BAT last=`AA`).
    #[test]
    fn kbd_reset_ff_clears_last_byte_then_bat_updates() {
        let mut k = I8042::new();
        write_kbd(&mut k, KBD_CMD_ECHO);
        assert_eq!(read_kbd_byte(&mut k), KBD_ECHO);
        assert_eq!(k.kbd_last_byte(), KBD_ECHO);

        write_kbd(&mut k, KBD_CMD_RESET);
        // Reset cleared last to 0; ACK presentation sets it to FA.
        assert_eq!(k.kbd_last_byte(), KBD_ACK);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        assert_eq!(read_kbd_byte(&mut k), KBD_BAT_OK);
        assert_eq!(k.kbd_last_byte(), KBD_BAT_OK);
        write_kbd(&mut k, KBD_CMD_RESEND);
        assert_eq!(read_kbd_byte(&mut k), KBD_BAT_OK);
    }

    /// Spec: OSDev PS/2 Keyboard — Echo (`0xEE`) → Echo response `0xEE`.
    #[test]
    fn kbd_echo_ee_returns_echo() {
        let mut k = I8042::new();
        write_kbd(&mut k, KBD_CMD_ECHO);
        assert_eq!(read_kbd_byte(&mut k), KBD_ECHO);
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);
    }

    /// Spec: OSDev PS/2 Keyboard — Get/Set Scancode Set (`0xF0`): command ACK,
    /// sub-command `0` → ACK then current set (default 2).
    #[test]
    fn kbd_scancode_set_f0_get_returns_ack_and_default_set_2() {
        let mut k = I8042::new();
        assert_eq!(k.kbd_scancode_set(), KBD_DEFAULT_SCANCODE_SET);

        write_kbd(&mut k, KBD_CMD_SET_SCANCODE_SET);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);

        k.port_write(I8042_DATA, 1, u32::from(KBD_SCANCODE_GET));
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        assert_eq!(read_kbd_byte(&mut k), KBD_DEFAULT_SCANCODE_SET);
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);
    }

    /// Spec: OSDev PS/2 Keyboard — `0xF0` + set number (1/2/3) → ACK and store;
    /// subsequent get returns the stored set.
    #[test]
    fn kbd_scancode_set_f0_set_and_get_round_trip() {
        let mut k = I8042::new();

        write_kbd(&mut k, KBD_CMD_SET_SCANCODE_SET);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        k.port_write(I8042_DATA, 1, 1);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        assert_eq!(k.kbd_scancode_set(), 1);

        write_kbd(&mut k, KBD_CMD_SET_SCANCODE_SET);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        k.port_write(I8042_DATA, 1, 3);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        assert_eq!(k.kbd_scancode_set(), 3);

        write_kbd(&mut k, KBD_CMD_SET_SCANCODE_SET);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        k.port_write(I8042_DATA, 1, u32::from(KBD_SCANCODE_GET));
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        assert_eq!(read_kbd_byte(&mut k), 3);
    }

    /// Spec: Reset restores scancode set to default 2 (OSDev power-on).
    #[test]
    fn kbd_reset_restores_scancode_set_default() {
        let mut k = I8042::new();
        write_kbd(&mut k, KBD_CMD_SET_SCANCODE_SET);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        k.port_write(I8042_DATA, 1, 1);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        assert_eq!(k.kbd_scancode_set(), 1);

        write_kbd(&mut k, KBD_CMD_RESET);
        assert_eq!(k.kbd_scancode_set(), KBD_DEFAULT_SCANCODE_SET);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        assert_eq!(read_kbd_byte(&mut k), KBD_BAT_OK);
    }

    /// Spec: after Set Scancode Set ACK, the next data-port byte is the
    /// sub-command (even if its value coincides with a command opcode).
    #[test]
    fn kbd_scancode_set_param_consumes_next_data_byte() {
        let mut k = I8042::new();
        write_kbd(&mut k, KBD_CMD_SET_SCANCODE_SET);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);

        // 0xFF would be Reset if mis-handled as a command — invalid set → no BAT.
        k.port_write(I8042_DATA, 1, 0xFF);
        assert_eq!(k.status() & STATUS_OBF, 0);
        assert_eq!(k.kbd_scancode_set(), KBD_DEFAULT_SCANCODE_SET);
        assert!(k.kbd_scanning_enabled());
    }

    /// Spec: OSDev PS/2 Keyboard — Set LEDs (`0xED`) → ACK `0xFA`, then LED
    /// parameter byte → ACK; store ScrollLock/NumLock/CapsLock bits.
    #[test]
    fn kbd_set_leds_ed_acks_and_stores_mask() {
        let mut k = I8042::new();
        assert_eq!(k.kbd_leds(), 0);

        write_kbd(&mut k, KBD_CMD_SET_LEDS);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);

        // bit0 ScrollLock | bit1 NumLock | bit2 CapsLock
        let leds = KBD_LED_SCROLL | KBD_LED_NUM | KBD_LED_CAPS;
        k.port_write(I8042_DATA, 1, u32::from(leds));
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        assert_eq!(k.kbd_leds(), leds);
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);
        assert!(!k.irq12_line());
    }

    /// Spec: OSDev PS/2 Keyboard — Set Typematic Rate/Delay (`0xF3`) → ACK,
    /// then rate/delay parameter → ACK; store the byte.
    #[test]
    fn kbd_set_typematic_f3_acks_and_stores_value() {
        let mut k = I8042::new();
        assert_eq!(k.kbd_typematic(), KBD_DEFAULT_TYPEMATIC);

        write_kbd(&mut k, KBD_CMD_SET_TYPEMATIC);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);

        k.port_write(I8042_DATA, 1, 0x00); // fastest rate / shortest delay encoding
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        assert_eq!(k.kbd_typematic(), 0x00);
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);
    }

    /// Spec: Reset restores LED mask, typematic, and scancode-set defaults
    /// (OSDev power-on).
    #[test]
    fn kbd_reset_restores_leds_and_typematic_defaults() {
        let mut k = I8042::new();
        write_kbd(&mut k, KBD_CMD_SET_LEDS);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        k.port_write(I8042_DATA, 1, u32::from(KBD_LED_CAPS));
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        assert_eq!(k.kbd_leds(), KBD_LED_CAPS);

        write_kbd(&mut k, KBD_CMD_SET_TYPEMATIC);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        k.port_write(I8042_DATA, 1, 0x7F);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        assert_eq!(k.kbd_typematic(), 0x7F);

        write_kbd(&mut k, KBD_CMD_SET_SCANCODE_SET);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        k.port_write(I8042_DATA, 1, 1);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        assert_eq!(k.kbd_scancode_set(), 1);

        write_kbd(&mut k, KBD_CMD_RESET);
        assert_eq!(k.kbd_leds(), 0);
        assert_eq!(k.kbd_typematic(), KBD_DEFAULT_TYPEMATIC);
        assert_eq!(k.kbd_scancode_set(), KBD_DEFAULT_SCANCODE_SET);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        assert_eq!(read_kbd_byte(&mut k), KBD_BAT_OK);
    }

    /// Spec: after Set LEDs ACK, the next data-port byte is the LED parameter
    /// (even if its value coincides with a command opcode).
    #[test]
    fn kbd_set_leds_param_consumes_next_data_byte() {
        let mut k = I8042::new();
        write_kbd(&mut k, KBD_CMD_SET_LEDS);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);

        // 0xFF would be Reset if mis-handled as a command.
        k.port_write(I8042_DATA, 1, 0xFF);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        assert_eq!(k.kbd_leds(), 0xFF);
        assert!(k.kbd_scanning_enabled());
        assert_eq!(k.status() & STATUS_OBF, 0); // no BAT — not a Reset
    }

    /// Helper: enable first port then send one host→keyboard byte on `0x60`.
    fn write_kbd(k: &mut I8042, byte: u8) {
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_ENABLE_KBD));
        k.port_write(I8042_DATA, 1, u32::from(byte));
    }

    /// Drain one keyboard (non-AUX) response byte.
    fn read_kbd_byte(k: &mut I8042) -> u8 {
        assert_ne!(k.status() & STATUS_OBF, 0);
        assert_eq!(k.status() & STATUS_AUX_OBF, 0);
        assert!(!k.aux_obf());
        k.port_read(I8042_DATA, 1) as u8
    }

    /// Spec: OSDev PS/2 Keyboard — Enable Scanning (`0xF4`) / Disable Scanning
    /// (`0xF5`) written to `0x60` (no pending controller write) → ACK `0xFA`.
    #[test]
    fn kbd_enable_f4_and_disable_f5_ack_and_store_scanning_flag() {
        let mut k = I8042::new();
        assert!(k.kbd_scanning_enabled());

        write_kbd(&mut k, KBD_CMD_DISABLE_SCANNING);
        assert!(!k.kbd_scanning_enabled());
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        assert_eq!(k.status() & STATUS_OBF, 0);

        write_kbd(&mut k, KBD_CMD_ENABLE_SCANNING);
        assert!(k.kbd_scanning_enabled());
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        assert_eq!(k.status() & STATUS_OBF, 0);
    }

    /// Spec: OSDev PS/2 Keyboard — Get Keyboard ID (`0xF2`) → ACK `0xFA` then
    /// MF2 identification bytes `0xAB` `0x83` on keyboard OBF (not AUX).
    #[test]
    fn kbd_get_id_f2_returns_ack_and_mf2_id_bytes() {
        let mut k = I8042::new();
        write_kbd(&mut k, KBD_CMD_GET_ID);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        assert_eq!(read_kbd_byte(&mut k), KBD_ID_MF2_0);
        assert_eq!(read_kbd_byte(&mut k), KBD_ID_MF2_1);
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);
        assert!(!k.irq12_line());
    }

    /// Spec: OSDev PS/2 Keyboard — Reset (`0xFF`) → ACK `0xFA` then BAT `0xAA`;
    /// restores scanning enabled.
    #[test]
    fn kbd_reset_ff_returns_ack_and_bat_restores_scanning() {
        let mut k = I8042::new();
        write_kbd(&mut k, KBD_CMD_DISABLE_SCANNING);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        assert!(!k.kbd_scanning_enabled());

        write_kbd(&mut k, KBD_CMD_RESET);
        assert!(k.kbd_scanning_enabled());
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        assert_eq!(read_kbd_byte(&mut k), KBD_BAT_OK);
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);
        assert!(!k.irq1_line()); // INT1 clear by default after enable-only path
    }

    /// Spec: keyboard ACK path raises IRQ1 when config INT1 is set; never IRQ12.
    #[test]
    fn kbd_ack_with_int1_asserts_irq1_not_irq12() {
        let mut k = I8042::new();
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_ENABLE_KBD));
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_CONFIG));
        k.port_write(I8042_DATA, 1, u32::from(CFG_INT1 | CFG_INT12));

        k.port_write(I8042_DATA, 1, u32::from(KBD_CMD_ENABLE_SCANNING));
        assert!(k.irq1_line());
        assert!(!k.irq12_line());
        assert_eq!(k.status() & STATUS_AUX_OBF, 0);
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, KBD_ACK);
        assert!(!k.irq1_line());
    }

    /// Spec: controller pending writes (`0x60` write-config) do not route to the
    /// keyboard device — config byte is stored, no keyboard ACK.
    #[test]
    fn controller_write_config_not_routed_to_kbd_device() {
        let mut k = I8042::new();
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_ENABLE_KBD));
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_CONFIG));
        // 0xF4 would be Enable Scanning if mis-routed to the keyboard.
        k.port_write(I8042_DATA, 1, u32::from(KBD_CMD_ENABLE_SCANNING));
        assert_eq!(k.config, KBD_CMD_ENABLE_SCANNING);
        assert_eq!(k.status() & STATUS_OBF, 0);
        assert_eq!(k.output_buffer(), None);
    }

    /// Spec: `0xD4` aux routing still owns the next `0x60` write — keyboard stub
    /// must not steal mouse Reset / ACK path.
    #[test]
    fn aux_d4_reset_not_stolen_by_kbd_stub() {
        let mut k = I8042::new();
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_ENABLE_KBD));
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_AUX));
        k.port_write(I8042_DATA, 1, u32::from(MOUSE_CMD_RESET));
        assert_eq!(k.aux_device_writes, 1);
        assert_ne!(k.status() & STATUS_AUX_OBF, 0);
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, MOUSE_ACK);
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, MOUSE_BAT_OK);
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, MOUSE_ID_STANDARD);
    }

    /// Spec: keyboard clock disable holds command responses until `0xAE`.
    #[test]
    fn kbd_response_held_while_clock_disabled_flushed_on_enable() {
        let mut k = I8042::new();
        assert!(k.keyboard_clock_disabled());
        k.port_write(I8042_DATA, 1, u32::from(KBD_CMD_GET_ID));
        assert_eq!(k.status() & STATUS_OBF, 0);

        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_ENABLE_KBD));
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        assert_eq!(read_kbd_byte(&mut k), KBD_ID_MF2_0);
        assert_eq!(read_kbd_byte(&mut k), KBD_ID_MF2_1);
    }

    /// Spec: Disable Scanning drops injected make-codes until Enable Scanning.
    #[test]
    fn inject_scancode_dropped_when_scanning_disabled() {
        let mut k = I8042::new();
        write_kbd(&mut k, KBD_CMD_DISABLE_SCANNING);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        assert!(!k.inject_scancode(0x1C));
        assert_eq!(k.status() & STATUS_OBF, 0);

        write_kbd(&mut k, KBD_CMD_ENABLE_SCANNING);
        assert_eq!(read_kbd_byte(&mut k), KBD_ACK);
        k.inject_scancode(0x1C);
        assert_ne!(k.status() & STATUS_OBF, 0);
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, 0x1E); // Set2→Set1 'A'
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

    /// Spec: IBM PC AT / OSDev I8042 — output-port bit0 is system reset (active low).
    /// Writing `0xD1` then a data byte with bit0 clear latches the same
    /// [`I8042::take_system_reset_request`] path as controller pulse-reset `0xFE`.
    #[test]
    fn d1_output_port_bit0_low_latches_system_reset() {
        let mut k = I8042::new();
        let before_pulse = k.pulse_reset_commands;
        assert!(!k.take_system_reset_request());

        // 0xDE: A20 on (bit1), system-reset line low (bit0 clear).
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_OUTPUT_PORT));
        k.port_write(I8042_DATA, 1, 0xDE);
        assert_eq!(k.output_port, 0xDE);
        assert!(k.a20_enabled());
        assert_eq!(k.pulse_reset_commands, before_pulse); // not the 0xFE command
        assert!(k.take_system_reset_request());
        assert!(!k.take_system_reset_request());
    }

    /// Spec: IBM PC AT — output-port writes with bit0 set do not pulse system reset.
    #[test]
    fn d1_output_port_bit0_high_does_not_latch_system_reset() {
        let mut k = I8042::new();
        assert!(!k.take_system_reset_request());

        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_OUTPUT_PORT));
        k.port_write(I8042_DATA, 1, 0xDD); // A20 off, bit0 still high
        assert_eq!(k.output_port, 0xDD);
        assert!(!k.take_system_reset_request());

        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_OUTPUT_PORT));
        k.port_write(I8042_DATA, 1, 0xDF); // default: A20 on, bit0 high
        assert_eq!(k.output_port, 0xDF);
        assert!(!k.take_system_reset_request());
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
    /// Default reset leaves translate on, so Set 2 `0x1C` (A) → Set 1 `0x1E`.
    #[test]
    fn inject_scancode_sets_obf_when_kbd_enabled() {
        let mut k = I8042::new();
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_ENABLE_KBD));
        assert!(!k.keyboard_clock_disabled());
        assert_ne!(k.config & CFG_TRANSLATE, 0);
        // INT1 clear: inject still fills OBF; no IRQ rising edge.
        assert!(!k.inject_scancode(0x1C)); // Set 2 make-code 'A'
        assert_ne!(k.status() & STATUS_OBF, 0);
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, 0x1E); // Set 1 'A'
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
        assert!(k.inject_scancode(0x1C)); // Set 2 'A'
        assert!(k.irq1_line());
        assert_ne!(k.status() & STATUS_OBF, 0);
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, 0x1E); // Set 1 'A'
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

    /// Spec: OSDev I8042 / IBM PS/2 KBC — aux clock disable (`0xA7` / config bit5)
    /// inhibits immediate host→aux delivery. Bytes via `0xD4` are **queued** (not
    /// delivered to the mouse stub yet): no recorded aux traffic, no ACK / AUX
    /// OBF, and no spurious IRQ12 while disabled. The controller still consumes
    /// the pending `0xD4` write. `0xA8` flushes the queue to the mouse stub.
    #[test]
    fn write_aux_d4_queued_when_aux_clock_disabled_via_a7_flushed_on_a8() {
        let mut k = I8042::new();
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_CONFIG));
        k.port_write(I8042_DATA, 1, u32::from(CFG_INT12));
        assert!(k.irq12_enabled());

        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_DISABLE_AUX));
        assert!(k.aux_clock_disabled());

        // Reset would normally ACK+BAT+ID on AUX OBF and raise IRQ12 with INT12.
        write_aux(&mut k, MOUSE_CMD_RESET);
        assert_eq!(k.aux_device_writes, 0);
        assert_eq!(k.last_aux_device_write, None);
        assert!(!k.mouse_reporting_enabled());
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);
        assert!(!k.irq12_line());
        assert_eq!(k.output_buffer(), None);

        // Pending `0xD4` was consumed: next data write is keyboard-bound again.
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_ENABLE_KBD));
        k.port_write(I8042_DATA, 1, 0xF1);
        assert_eq!(k.aux_device_writes, 0);
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);

        // Spec: `0xA8` re-enables aux clock and delivers queued host→aux bytes.
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_ENABLE_AUX));
        assert!(!k.aux_clock_disabled());
        assert_eq!(k.last_aux_device_write, Some(MOUSE_CMD_RESET));
        assert_eq!(k.aux_device_writes, 1);
        assert!(k.irq12_line());
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert_eq!(read_aux_byte(&mut k), MOUSE_BAT_OK);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ID_STANDARD);
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);
    }

    /// Spec: OSDev I8042 — config bit5 set via write-config queues host→aux
    /// `0xD4` (same inhibit as `0xA7`); clearing bit5 via write-config flushes.
    #[test]
    fn write_aux_d4_queued_when_aux_clock_disabled_via_config_flushed_on_enable() {
        let mut k = I8042::new();
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_CONFIG));
        k.port_write(I8042_DATA, 1, u32::from(CFG_INT12 | CFG_AUX_CLOCK_DISABLE));
        assert!(k.aux_clock_disabled());

        write_aux(&mut k, MOUSE_CMD_GET_DEVICE_ID);
        assert_eq!(k.aux_device_writes, 0);
        assert_eq!(k.last_aux_device_write, None);
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);
        assert!(!k.irq12_line());

        // Clear bit5 via write-config — flush queued Get Device ID.
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_CONFIG));
        k.port_write(I8042_DATA, 1, u32::from(CFG_INT12));
        assert!(!k.aux_clock_disabled());
        assert_eq!(k.last_aux_device_write, Some(MOUSE_CMD_GET_DEVICE_ID));
        assert_eq!(k.aux_device_writes, 1);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ID_STANDARD);
    }

    /// Spec: OSDev I8042 / PS/2 Mouse — with aux clock enabled (`0xA8` / default),
    /// host→aux Reset (`0xFF`) via `0xD4` answers ACK + BAT + ID on AUX OBF
    /// immediately (no queue).
    #[test]
    fn write_aux_d4_reset_works_when_aux_clock_enabled() {
        let mut k = I8042::new();
        assert!(!k.aux_clock_disabled());
        write_aux(&mut k, MOUSE_CMD_RESET);
        assert_eq!(k.last_aux_device_write, Some(MOUSE_CMD_RESET));
        assert_eq!(k.aux_device_writes, 1);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert_eq!(read_aux_byte(&mut k), MOUSE_BAT_OK);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ID_STANDARD);
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);
    }

    /// Spec: OSDev I8042 — multi-byte host→aux (Set Sample Rate + value) queued
    /// while clock disabled is delivered in order on `0xA8`.
    #[test]
    fn write_aux_d4_sample_rate_param_queued_while_disabled_flushed_on_a8() {
        let mut k = I8042::new();
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_DISABLE_AUX));
        write_aux(&mut k, MOUSE_CMD_SET_SAMPLE_RATE);
        write_aux(&mut k, 200);
        assert_eq!(k.aux_device_writes, 0);
        assert_eq!(k.mouse_sample_rate(), MOUSE_DEFAULT_SAMPLE_RATE);

        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_ENABLE_AUX));
        assert_eq!(k.aux_device_writes, 2);
        assert_eq!(k.mouse_sample_rate(), 200);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK); // ACK for 0xF3
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK); // ACK for rate
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

    /// Spec: OSDev I8042 / IBM PS/2 KBC — `0xAB` (test first/keyboard PS/2 port)
    /// returns a result byte (`0x00` = no error). Same presentation as `0xA9`:
    /// controller response on normal OBF, not AUX OBF.
    #[test]
    fn test_kbd_ab_returns_00_on_normal_output_buffer() {
        let mut k = I8042::new();
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_TEST_KBD));
        assert_ne!(k.status() & STATUS_OBF, 0);
        assert_eq!(k.status() & STATUS_AUX_OBF, 0);
        assert!(!k.aux_obf());
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, TEST_KBD_OK);
        assert_eq!(k.status() & STATUS_OBF, 0);
        assert_eq!(k.unsupported_commands, 0);
    }

    /// Spec: OSDev I8042 — `0xAC` diagnostic dump (read internal RAM). Stub
    /// returns [`DIAG_DUMP_LEN`] (`16`) fixed zero bytes ([`DIAG_DUMP_STUB`]) on
    /// normal OBF (not AUX), one byte per `0x60` read. Must present even when
    /// the keyboard clock is disabled (reset default config bit4).
    #[test]
    fn diag_dump_ac_returns_16_zero_bytes_on_normal_output_buffer() {
        let mut k = I8042::new();
        assert!(k.keyboard_clock_disabled());
        assert_eq!(DIAG_DUMP_STUB.len(), DIAG_DUMP_LEN);
        assert_eq!(DIAG_DUMP_LEN, 16);
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_DIAG_DUMP));
        for (i, &expected) in DIAG_DUMP_STUB.iter().enumerate() {
            assert_ne!(k.status() & STATUS_OBF, 0, "byte {i}: OBF expected");
            assert_eq!(k.status() & STATUS_AUX_OBF, 0, "byte {i}: not AUX OBF");
            assert!(!k.aux_obf(), "byte {i}: aux_obf clear");
            assert_eq!(
                k.port_read(I8042_DATA, 1) as u8,
                expected,
                "byte {i} of diagnostic dump"
            );
        }
        assert_eq!(k.status() & STATUS_OBF, 0);
        assert_eq!(k.unsupported_commands, 0);
    }

    /// Spec: OSDev I8042 / IBM PS/2 KBC — `0xD4` routes the next data-port byte to
    /// the auxiliary device. Unsupported mouse commands are recorded / counted
    /// with no device response; the following data-port write goes back to the
    /// keyboard path (unsupported kbd cmds remain unanswered).
    #[test]
    fn write_aux_d4_routes_next_data_byte_to_aux_device() {
        let mut k = I8042::new();
        assert_eq!(k.aux_device_writes, 0);
        assert_eq!(k.last_aux_device_write, None);

        // 0xEF = unimplemented mouse opcode; recorded, no ACK.
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_AUX));
        k.port_write(I8042_DATA, 1, 0xEF);
        assert_eq!(k.aux_device_writes, 1);
        assert_eq!(k.last_aux_device_write, Some(0xEF));
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);
        assert!(!k.irq12_line());

        // Next data byte is keyboard-bound again: aux state untouched; an
        // unsupported kbd opcode (`0xF1`) produces no ACK / no OBF.
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_ENABLE_KBD));
        k.port_write(I8042_DATA, 1, 0xF1);
        assert_eq!(k.aux_device_writes, 1);
        assert_eq!(k.last_aux_device_write, Some(0xEF));
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);
        assert_eq!(k.unsupported_commands, 0);
    }

    /// Spec: OSDev PS/2 Mouse — Set Wrap Mode (`0xEE`) via `0xD4` → ACK on AUX
    /// OBF; stores wrap-mode flag. Subsequent host→aux bytes are echoed (see
    /// wrap-echo tests) except Reset Wrap / Reset.
    #[test]
    fn mouse_set_wrap_mode_ee_acks() {
        let mut k = I8042::new();
        assert!(!k.mouse_wrap_mode());
        write_aux(&mut k, MOUSE_CMD_SET_WRAP_MODE);
        assert!(k.mouse_wrap_mode());
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);
    }

    /// Spec: OSDev PS/2 Mouse wrap mode — after Set Wrap ACK, host→aux data
    /// bytes that are not Reset Wrap (`0xEC`) or Reset (`0xFF`) are echoed on
    /// AUX OBF with no ACK prefix.
    #[test]
    fn mouse_wrap_mode_echoes_host_bytes_without_ack() {
        let mut k = I8042::new();
        write_aux(&mut k, MOUSE_CMD_SET_WRAP_MODE);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert!(k.mouse_wrap_mode());

        write_aux(&mut k, 0x12);
        assert!(k.mouse_wrap_mode());
        assert_eq!(
            read_aux_byte(&mut k),
            0x12,
            "wrap mode echoes the byte itself (no ACK)"
        );
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);

        write_aux(&mut k, 0xAB);
        assert_eq!(read_aux_byte(&mut k), 0xAB);
        assert!(k.mouse_wrap_mode());
    }

    /// Spec: OSDev PS/2 Mouse wrap mode — even bytes that are valid commands
    /// when out of wrap (Enable Reporting, Set Wrap again, Status Request) are
    /// only echoed while wrap is active.
    #[test]
    fn mouse_wrap_mode_echoes_command_like_bytes() {
        let mut k = I8042::new();
        write_aux(&mut k, MOUSE_CMD_SET_WRAP_MODE);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);

        write_aux(&mut k, MOUSE_CMD_ENABLE_REPORTING);
        assert!(!k.mouse_reporting_enabled(), "wrap echo must not Enable");
        assert_eq!(read_aux_byte(&mut k), MOUSE_CMD_ENABLE_REPORTING);

        write_aux(&mut k, MOUSE_CMD_SET_WRAP_MODE);
        assert!(k.mouse_wrap_mode());
        assert_eq!(
            read_aux_byte(&mut k),
            MOUSE_CMD_SET_WRAP_MODE,
            "0xEE while wrap is echoed, not re-ACKed as Set Wrap"
        );

        write_aux(&mut k, MOUSE_CMD_STATUS_REQUEST);
        assert_eq!(read_aux_byte(&mut k), MOUSE_CMD_STATUS_REQUEST);
        assert_eq!(
            k.status() & (STATUS_OBF | STATUS_AUX_OBF),
            0,
            "no Status Request payload after wrap echo"
        );
    }

    /// Spec: OSDev PS/2 Mouse wrap mode — after Reset Wrap, normal command ACK
    /// path resumes (no more echo).
    #[test]
    fn mouse_wrap_mode_echo_stops_after_reset_wrap() {
        let mut k = I8042::new();
        write_aux(&mut k, MOUSE_CMD_SET_WRAP_MODE);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        write_aux(&mut k, 0x55);
        assert_eq!(read_aux_byte(&mut k), 0x55);

        write_aux(&mut k, MOUSE_CMD_RESET_WRAP_MODE);
        assert!(!k.mouse_wrap_mode());
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);

        write_aux(&mut k, MOUSE_CMD_GET_DEVICE_ID);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ID_STANDARD);
    }

    /// Spec: OSDev PS/2 Mouse — Reset restores defaults including wrap off.
    #[test]
    fn mouse_reset_clears_wrap_mode() {
        let mut k = I8042::new();
        write_aux(&mut k, MOUSE_CMD_SET_WRAP_MODE);
        assert!(k.mouse_wrap_mode());
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);

        write_aux(&mut k, MOUSE_CMD_RESET);
        assert!(!k.mouse_wrap_mode());
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert_eq!(read_aux_byte(&mut k), MOUSE_BAT_OK);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ID_STANDARD);
    }

    /// Spec: OSDev PS/2 Mouse wrap mode — Set Stream Mode (`0xEA`) while wrap
    /// is active is echoed (not executed); wrap stays set. After Reset Wrap,
    /// Stream Mode ACKs normally.
    #[test]
    fn mouse_wrap_mode_echoes_set_stream_mode() {
        let mut k = I8042::new();
        write_aux(&mut k, MOUSE_CMD_SET_WRAP_MODE);
        assert!(k.mouse_wrap_mode());
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);

        write_aux(&mut k, MOUSE_CMD_SET_STREAM_MODE);
        assert!(
            k.mouse_wrap_mode(),
            "0xEA while wrap is echoed, not cleared"
        );
        assert_eq!(read_aux_byte(&mut k), MOUSE_CMD_SET_STREAM_MODE);

        write_aux(&mut k, MOUSE_CMD_RESET_WRAP_MODE);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert!(!k.mouse_wrap_mode());

        write_aux(&mut k, MOUSE_CMD_SET_STREAM_MODE);
        assert!(!k.mouse_wrap_mode());
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
    }

    /// Spec: OSDev PS/2 Mouse wrap mode — Set Remote Mode (`0xF0`) while wrap
    /// is active is echoed (not executed); wrap stays set and remote is not
    /// entered.
    #[test]
    fn mouse_wrap_mode_echoes_set_remote_mode() {
        let mut k = I8042::new();
        write_aux(&mut k, MOUSE_CMD_SET_WRAP_MODE);
        assert!(k.mouse_wrap_mode());
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);

        write_aux(&mut k, MOUSE_CMD_SET_REMOTE_MODE);
        assert!(k.mouse_wrap_mode());
        assert!(
            !k.mouse_remote_mode(),
            "0xF0 while wrap must not enter remote"
        );
        assert_eq!(read_aux_byte(&mut k), MOUSE_CMD_SET_REMOTE_MODE);
    }

    /// Spec: OSDev PS/2 Mouse — Reset Wrap Mode (`0xEC`) via `0xD4` → ACK on AUX
    /// OBF; clears wrap-mode flag. Stream/remote mode is unchanged.
    #[test]
    fn mouse_reset_wrap_mode_ec_acks_and_clears_wrap() {
        let mut k = I8042::new();
        write_aux(&mut k, MOUSE_CMD_SET_WRAP_MODE);
        assert!(k.mouse_wrap_mode());
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);

        write_aux(&mut k, MOUSE_CMD_RESET_WRAP_MODE);
        assert_eq!(k.last_aux_device_write, Some(MOUSE_CMD_RESET_WRAP_MODE));
        assert!(!k.mouse_wrap_mode());
        assert!(!k.mouse_remote_mode());
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);
    }

    /// Spec: OSDev PS/2 Mouse — Reset Wrap Mode while already out of wrap still
    /// ACKs; does not disturb remote mode.
    #[test]
    fn mouse_reset_wrap_mode_acks_when_already_clear_preserves_remote() {
        let mut k = I8042::new();
        write_aux(&mut k, MOUSE_CMD_SET_REMOTE_MODE);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert!(k.mouse_remote_mode());
        assert!(!k.mouse_wrap_mode());

        write_aux(&mut k, MOUSE_CMD_RESET_WRAP_MODE);
        assert!(!k.mouse_wrap_mode());
        assert!(k.mouse_remote_mode());
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
    }

    /// Spec: OSDev PS/2 Mouse — Set Stream Mode (`0xEA`) via `0xD4` → ACK.
    #[test]
    fn mouse_set_stream_mode_ea_acks() {
        let mut k = I8042::new();
        write_aux(&mut k, MOUSE_CMD_SET_STREAM_MODE);
        assert!(!k.mouse_remote_mode());
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);
    }

    /// Spec: OSDev PS/2 Mouse — Set Remote Mode (`0xF0`) via `0xD4` → ACK on AUX
    /// OBF; stores remote-mode flag (Status Request byte1 bit6).
    #[test]
    fn mouse_set_remote_mode_f0_acks() {
        let mut k = I8042::new();
        assert!(!k.mouse_remote_mode());
        write_aux(&mut k, MOUSE_CMD_SET_REMOTE_MODE);
        assert!(k.mouse_remote_mode());
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);

        write_aux(&mut k, MOUSE_CMD_STATUS_REQUEST);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert_eq!(read_aux_byte(&mut k), MOUSE_STATUS_REMOTE);
        let _ = read_aux_byte(&mut k); // res
        let _ = read_aux_byte(&mut k); // rate
    }

    /// Spec: OSDev PS/2 Mouse — Read Data (`0xEB`) via `0xD4` → ACK then one
    /// 3-byte movement packet. With no prior inject, the packet is zeros
    /// (always-1 flag only). Works after Set Remote Mode (`0xF0`).
    #[test]
    fn mouse_read_data_eb_acks_then_zero_packet_after_remote() {
        let mut k = I8042::new();
        write_aux(&mut k, MOUSE_CMD_SET_REMOTE_MODE);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert!(k.mouse_remote_mode());

        write_aux(&mut k, MOUSE_CMD_READ_DATA);
        assert_eq!(k.last_aux_device_write, Some(MOUSE_CMD_READ_DATA));
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert_eq!(read_aux_byte(&mut k), MOUSE_PKT_ALWAYS1);
        assert_eq!(read_aux_byte(&mut k), 0);
        assert_eq!(read_aux_byte(&mut k), 0);
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);
    }

    /// Spec: OSDev PS/2 Mouse — Read Data forces a packet; this stub also ACKs
    /// + packet in stream mode (SeaBIOS/OSDev-friendly).
    #[test]
    fn mouse_read_data_eb_acks_then_packet_in_stream_mode() {
        let mut k = I8042::new();
        assert!(!k.mouse_remote_mode());
        write_aux(&mut k, MOUSE_CMD_READ_DATA);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert_eq!(read_aux_byte(&mut k), MOUSE_PKT_ALWAYS1);
        assert_eq!(read_aux_byte(&mut k), 0);
        assert_eq!(read_aux_byte(&mut k), 0);
    }

    /// Spec: OSDev PS/2 Mouse — Read Data returns the last injected movement
    /// (dx/dy/buttons) after stream packet is drained.
    #[test]
    fn mouse_read_data_eb_uses_last_injected_packet() {
        let mut k = I8042::new();
        write_aux(&mut k, MOUSE_CMD_ENABLE_REPORTING);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert!(!k.inject_mouse_packet(5, -3, MOUSE_BTN_LEFT));
        // Drain stream-mode packet.
        assert_eq!(
            read_aux_byte(&mut k),
            MOUSE_PKT_ALWAYS1 | MOUSE_BTN_LEFT | MOUSE_PKT_Y_SIGN
        );
        assert_eq!(read_aux_byte(&mut k), 5u8);
        assert_eq!(read_aux_byte(&mut k), (-3i8) as u8);

        write_aux(&mut k, MOUSE_CMD_SET_REMOTE_MODE);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);

        write_aux(&mut k, MOUSE_CMD_READ_DATA);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert_eq!(
            read_aux_byte(&mut k),
            MOUSE_PKT_ALWAYS1 | MOUSE_BTN_LEFT | MOUSE_PKT_Y_SIGN
        );
        assert_eq!(read_aux_byte(&mut k), 5u8);
        assert_eq!(read_aux_byte(&mut k), (-3i8) as u8);
    }

    /// Spec: OSDev PS/2 Mouse — Reset clears remembered movement; next Read Data
    /// returns a zero packet.
    #[test]
    fn mouse_reset_clears_last_packet_for_read_data() {
        let mut k = I8042::new();
        write_aux(&mut k, MOUSE_CMD_ENABLE_REPORTING);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert!(!k.inject_mouse_packet(7, 1, MOUSE_BTN_RIGHT));
        let _ = read_aux_byte(&mut k);
        let _ = read_aux_byte(&mut k);
        let _ = read_aux_byte(&mut k);

        write_aux(&mut k, MOUSE_CMD_RESET);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert_eq!(read_aux_byte(&mut k), MOUSE_BAT_OK);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ID_STANDARD);

        write_aux(&mut k, MOUSE_CMD_READ_DATA);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert_eq!(read_aux_byte(&mut k), MOUSE_PKT_ALWAYS1);
        assert_eq!(read_aux_byte(&mut k), 0);
        assert_eq!(read_aux_byte(&mut k), 0);
    }

    /// Spec: OSDev PS/2 Mouse — Reset restores stream mode (clears remote).
    #[test]
    fn mouse_reset_clears_remote_mode_to_stream() {
        let mut k = I8042::new();
        write_aux(&mut k, MOUSE_CMD_SET_REMOTE_MODE);
        assert!(k.mouse_remote_mode());
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);

        write_aux(&mut k, MOUSE_CMD_RESET);
        assert!(!k.mouse_remote_mode());
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert_eq!(read_aux_byte(&mut k), MOUSE_BAT_OK);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ID_STANDARD);
    }

    /// Spec: OSDev PS/2 Mouse — Set Stream Mode (`0xEA`) clears remote mode.
    #[test]
    fn mouse_set_stream_mode_clears_remote() {
        let mut k = I8042::new();
        write_aux(&mut k, MOUSE_CMD_SET_REMOTE_MODE);
        assert!(k.mouse_remote_mode());
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);

        write_aux(&mut k, MOUSE_CMD_SET_STREAM_MODE);
        assert!(!k.mouse_remote_mode());
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
    }

    /// Spec: OSDev PS/2 Mouse — Resend (`0xFE`) via `0xD4` requeues last AUX byte.
    #[test]
    fn mouse_resend_fe_requeues_last_aux_byte() {
        let mut k = I8042::new();
        write_aux(&mut k, MOUSE_CMD_GET_DEVICE_ID);
        assert_eq!(read_aux_byte(&mut k), KBD_ACK);
        assert_eq!(read_aux_byte(&mut k), 0x00); // device ID
        write_aux(&mut k, MOUSE_CMD_RESEND);
        assert_eq!(read_aux_byte(&mut k), 0x00, "resend last AUX byte (ID)");
    }

    /// Helper: send one host→aux byte via controller command `0xD4`.
    fn write_aux(k: &mut I8042, byte: u8) {
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_AUX));
        k.port_write(I8042_DATA, 1, u32::from(byte));
    }

    /// Drain one aux response: OBF+AUX OBF set, return data-port byte.
    fn read_aux_byte(k: &mut I8042) -> u8 {
        assert_ne!(k.status() & STATUS_OBF, 0);
        assert_ne!(k.status() & STATUS_AUX_OBF, 0);
        assert!(k.aux_obf());
        k.port_read(I8042_DATA, 1) as u8
    }

    /// Spec: OSDev "Mouse Commands" / PS/2 mouse — Reset (`0xFF`) answers ACK
    /// `0xFA`, then BAT `0xAA`, then device ID `0x00` (standard PS/2 mouse).
    /// Responses arrive on AUX OBF via the existing `0xD4` routing path.
    #[test]
    fn mouse_reset_ff_returns_ack_bat_and_device_id() {
        let mut k = I8042::new();
        write_aux(&mut k, MOUSE_CMD_RESET);
        assert_eq!(k.last_aux_device_write, Some(MOUSE_CMD_RESET));
        assert_eq!(k.aux_device_writes, 1);
        assert!(!k.mouse_reporting_enabled());

        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert_eq!(read_aux_byte(&mut k), MOUSE_BAT_OK);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ID_STANDARD);
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);
        assert!(!k.irq1_line());
    }

    /// Spec: OSDev PS/2 mouse — Get Device ID (`0xF2`) → ACK `0xFA` then ID `0x00`.
    #[test]
    fn mouse_get_device_id_f2_returns_ack_and_id() {
        let mut k = I8042::new();
        write_aux(&mut k, MOUSE_CMD_GET_DEVICE_ID);
        assert_eq!(k.last_aux_device_write, Some(MOUSE_CMD_GET_DEVICE_ID));
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ID_STANDARD);
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);
    }

    /// Spec: OSDev PS/2 mouse — Enable Data Reporting (`0xF4`) → ACK `0xFA`;
    /// Disable Data Reporting (`0xF5`) → ACK `0xFA`. Stub stores the enabled
    /// flag only (no movement stream).
    #[test]
    fn mouse_enable_f4_and_disable_f5_ack_and_store_reporting_flag() {
        let mut k = I8042::new();
        assert!(!k.mouse_reporting_enabled());

        write_aux(&mut k, MOUSE_CMD_ENABLE_REPORTING);
        assert!(k.mouse_reporting_enabled());
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);

        write_aux(&mut k, MOUSE_CMD_DISABLE_REPORTING);
        assert!(!k.mouse_reporting_enabled());
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);
    }

    /// Spec: OSDev PS/2 mouse — Reset clears data reporting (disabled after BAT).
    #[test]
    fn mouse_reset_clears_reporting_enabled() {
        let mut k = I8042::new();
        write_aux(&mut k, MOUSE_CMD_ENABLE_REPORTING);
        assert!(k.mouse_reporting_enabled());
        let _ = read_aux_byte(&mut k); // ACK

        write_aux(&mut k, MOUSE_CMD_RESET);
        assert!(!k.mouse_reporting_enabled());
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert_eq!(read_aux_byte(&mut k), MOUSE_BAT_OK);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ID_STANDARD);
    }

    /// Spec: OSDev PS/2 Mouse "Mouse Commands" — Set Sample Rate (`0xF3`) ACKs,
    /// then the next `0xD4` byte is the rate (stored) and is also ACKed.
    #[test]
    fn mouse_set_sample_rate_f3_acks_and_stores_value() {
        let mut k = I8042::new();
        assert_eq!(k.mouse_sample_rate(), MOUSE_DEFAULT_SAMPLE_RATE);

        write_aux(&mut k, MOUSE_CMD_SET_SAMPLE_RATE);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        write_aux(&mut k, 80);
        assert_eq!(k.mouse_sample_rate(), 80);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);
    }

    /// Spec: OSDev PS/2 Mouse — Set Resolution (`0xE8`) ACKs, then stores the
    /// next `0xD4` byte (0..3 encoding) and ACKs.
    #[test]
    fn mouse_set_resolution_e8_acks_and_stores_value() {
        let mut k = I8042::new();
        assert_eq!(k.mouse_resolution(), MOUSE_DEFAULT_RESOLUTION);

        write_aux(&mut k, MOUSE_CMD_SET_RESOLUTION);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        write_aux(&mut k, 3); // 8 counts/mm
        assert_eq!(k.mouse_resolution(), 3);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
    }

    /// Spec: OSDev PS/2 Mouse Status Request (`0xE9`) → ACK `0xFA` then a 3-byte
    /// status packet: flags (bit4 scaling, bit5 enable, bit6 remote), resolution,
    /// sample rate. Buttons stay clear; mode stays stream (bit6=0).
    #[test]
    fn mouse_status_request_e9_returns_ack_and_three_status_bytes() {
        let mut k = I8042::new();
        // Defaults: scaling 1:1, reporting off, res=2, rate=100.
        write_aux(&mut k, MOUSE_CMD_STATUS_REQUEST);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        let flags = read_aux_byte(&mut k);
        assert_eq!(flags, 0x00); // scaling/enable/remote/buttons clear
        assert_eq!(flags & MOUSE_STATUS_REMOTE, 0); // stub is always stream mode
        assert_eq!(read_aux_byte(&mut k), MOUSE_DEFAULT_RESOLUTION);
        assert_eq!(read_aux_byte(&mut k), MOUSE_DEFAULT_SAMPLE_RATE);
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);

        // Change params + enable reporting; status must reflect them.
        write_aux(&mut k, MOUSE_CMD_SET_SAMPLE_RATE);
        let _ = read_aux_byte(&mut k);
        write_aux(&mut k, 200);
        let _ = read_aux_byte(&mut k);
        write_aux(&mut k, MOUSE_CMD_SET_RESOLUTION);
        let _ = read_aux_byte(&mut k);
        write_aux(&mut k, 1);
        let _ = read_aux_byte(&mut k);
        write_aux(&mut k, MOUSE_CMD_SET_SCALING_2_1);
        let _ = read_aux_byte(&mut k);
        write_aux(&mut k, MOUSE_CMD_ENABLE_REPORTING);
        let _ = read_aux_byte(&mut k);

        write_aux(&mut k, MOUSE_CMD_STATUS_REQUEST);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert_eq!(
            read_aux_byte(&mut k),
            MOUSE_STATUS_SCALING | MOUSE_STATUS_ENABLE
        );
        assert_eq!(read_aux_byte(&mut k), 1);
        assert_eq!(read_aux_byte(&mut k), 200);
    }

    /// Spec: OSDev PS/2 Mouse — Set Scaling 1:1 (`0xE6`) / 2:1 (`0xE7`) → ACK;
    /// Status Request bit4 follows the stored scaling.
    #[test]
    fn mouse_set_scaling_e6_e7_ack_and_affect_status() {
        let mut k = I8042::new();
        assert!(!k.mouse_scaling_21());

        write_aux(&mut k, MOUSE_CMD_SET_SCALING_2_1);
        assert!(k.mouse_scaling_21());
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);

        write_aux(&mut k, MOUSE_CMD_STATUS_REQUEST);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert_eq!(read_aux_byte(&mut k), MOUSE_STATUS_SCALING);
        let _ = read_aux_byte(&mut k); // res
        let _ = read_aux_byte(&mut k); // rate

        write_aux(&mut k, MOUSE_CMD_SET_SCALING_1_1);
        assert!(!k.mouse_scaling_21());
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);

        write_aux(&mut k, MOUSE_CMD_STATUS_REQUEST);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert_eq!(read_aux_byte(&mut k), 0x00);
        let _ = read_aux_byte(&mut k);
        let _ = read_aux_byte(&mut k);
    }

    /// Spec: OSDev PS/2 Mouse — Reset restores sample rate / resolution / scaling
    /// defaults (100, 2, 1:1) in addition to clearing data reporting.
    #[test]
    fn mouse_reset_restores_parameter_defaults() {
        let mut k = I8042::new();
        write_aux(&mut k, MOUSE_CMD_SET_SAMPLE_RATE);
        let _ = read_aux_byte(&mut k);
        write_aux(&mut k, 40);
        let _ = read_aux_byte(&mut k);
        write_aux(&mut k, MOUSE_CMD_SET_RESOLUTION);
        let _ = read_aux_byte(&mut k);
        write_aux(&mut k, 0);
        let _ = read_aux_byte(&mut k);
        write_aux(&mut k, MOUSE_CMD_SET_SCALING_2_1);
        let _ = read_aux_byte(&mut k);
        write_aux(&mut k, MOUSE_CMD_ENABLE_REPORTING);
        let _ = read_aux_byte(&mut k);

        assert_eq!(k.mouse_sample_rate(), 40);
        assert_eq!(k.mouse_resolution(), 0);
        assert!(k.mouse_scaling_21());
        assert!(k.mouse_reporting_enabled());

        write_aux(&mut k, MOUSE_CMD_RESET);
        assert_eq!(k.mouse_sample_rate(), MOUSE_DEFAULT_SAMPLE_RATE);
        assert_eq!(k.mouse_resolution(), MOUSE_DEFAULT_RESOLUTION);
        assert!(!k.mouse_scaling_21());
        assert!(!k.mouse_reporting_enabled());
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert_eq!(read_aux_byte(&mut k), MOUSE_BAT_OK);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ID_STANDARD);
    }

    /// Spec: OSDev PS/2 Mouse — Set Defaults (`0xF6`) ACKs and restores sample /
    /// resolution / scaling / reporting / stream defaults **without** BAT/ID
    /// (unlike Reset `0xFF`).
    #[test]
    fn mouse_set_defaults_f6_acks_and_restores_defaults_without_bat() {
        let mut k = I8042::new();
        write_aux(&mut k, MOUSE_CMD_SET_SAMPLE_RATE);
        let _ = read_aux_byte(&mut k);
        write_aux(&mut k, 40);
        let _ = read_aux_byte(&mut k);
        write_aux(&mut k, MOUSE_CMD_SET_RESOLUTION);
        let _ = read_aux_byte(&mut k);
        write_aux(&mut k, 0);
        let _ = read_aux_byte(&mut k);
        write_aux(&mut k, MOUSE_CMD_SET_SCALING_2_1);
        let _ = read_aux_byte(&mut k);
        write_aux(&mut k, MOUSE_CMD_SET_REMOTE_MODE);
        let _ = read_aux_byte(&mut k);
        // Enter wrap briefly then leave via Reset Wrap — while wrap is active,
        // host→aux bytes (including Enable/Set Defaults) are echoed, not run.
        write_aux(&mut k, MOUSE_CMD_SET_WRAP_MODE);
        let _ = read_aux_byte(&mut k);
        assert!(k.mouse_wrap_mode());
        write_aux(&mut k, MOUSE_CMD_RESET_WRAP_MODE);
        let _ = read_aux_byte(&mut k);
        assert!(!k.mouse_wrap_mode());
        write_aux(&mut k, MOUSE_CMD_ENABLE_REPORTING);
        let _ = read_aux_byte(&mut k);

        assert_eq!(k.mouse_sample_rate(), 40);
        assert_eq!(k.mouse_resolution(), 0);
        assert!(k.mouse_scaling_21());
        assert!(k.mouse_remote_mode());
        assert!(!k.mouse_wrap_mode());
        assert!(k.mouse_reporting_enabled());

        write_aux(&mut k, MOUSE_CMD_SET_DEFAULTS);
        assert_eq!(k.last_aux_device_write, Some(MOUSE_CMD_SET_DEFAULTS));
        assert_eq!(k.mouse_sample_rate(), MOUSE_DEFAULT_SAMPLE_RATE);
        assert_eq!(k.mouse_resolution(), MOUSE_DEFAULT_RESOLUTION);
        assert!(!k.mouse_scaling_21());
        assert!(!k.mouse_reporting_enabled());
        assert!(!k.mouse_remote_mode(), "Set Defaults restores stream mode");
        assert!(!k.mouse_wrap_mode(), "Set Defaults leaves wrap off");
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        // No BAT/ID on OBF after Set Defaults.
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);
    }

    /// Spec: OSDev PS/2 Mouse — after Set Defaults, Status Request reflects
    /// default flags (reporting off, stream, scaling 1:1), resolution, rate.
    #[test]
    fn mouse_set_defaults_status_request_shows_defaults() {
        let mut k = I8042::new();
        write_aux(&mut k, MOUSE_CMD_SET_SAMPLE_RATE);
        let _ = read_aux_byte(&mut k);
        write_aux(&mut k, 80);
        let _ = read_aux_byte(&mut k);
        write_aux(&mut k, MOUSE_CMD_SET_RESOLUTION);
        let _ = read_aux_byte(&mut k);
        write_aux(&mut k, 3);
        let _ = read_aux_byte(&mut k);
        write_aux(&mut k, MOUSE_CMD_SET_SCALING_2_1);
        let _ = read_aux_byte(&mut k);
        write_aux(&mut k, MOUSE_CMD_SET_REMOTE_MODE);
        let _ = read_aux_byte(&mut k);
        write_aux(&mut k, MOUSE_CMD_ENABLE_REPORTING);
        let _ = read_aux_byte(&mut k);

        write_aux(&mut k, MOUSE_CMD_SET_DEFAULTS);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);

        write_aux(&mut k, MOUSE_CMD_STATUS_REQUEST);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert_eq!(read_aux_byte(&mut k), 0x00); // stream, reporting off, scaling 1:1
        assert_eq!(read_aux_byte(&mut k), MOUSE_DEFAULT_RESOLUTION);
        assert_eq!(read_aux_byte(&mut k), MOUSE_DEFAULT_SAMPLE_RATE);
    }

    /// Spec: OSDev PS/2 Mouse "Mouse Packet Format" — when data reporting is
    /// enabled (`0xF4`), [`I8042::inject_mouse_packet`] queues a standard 3-byte
    /// stream-mode packet (flags/buttons/signs/overflows, dx, dy) on AUX OBF,
    /// one byte at a time.
    #[test]
    fn mouse_inject_packet_streams_three_bytes_when_reporting_enabled() {
        let mut k = I8042::new();
        write_aux(&mut k, MOUSE_CMD_ENABLE_REPORTING);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert!(k.mouse_reporting_enabled());

        // dx=+5, dy=-3, left button → flags: always1 | left | Y sign.
        assert!(!k.inject_mouse_packet(5, -3, MOUSE_BTN_LEFT));
        assert_eq!(
            read_aux_byte(&mut k),
            MOUSE_PKT_ALWAYS1 | MOUSE_BTN_LEFT | MOUSE_PKT_Y_SIGN
        );
        assert_eq!(read_aux_byte(&mut k), 5u8);
        assert_eq!(read_aux_byte(&mut k), (-3i8) as u8);
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);
    }

    /// Spec: OSDev PS/2 Mouse — Disable Data Reporting (`0xF5`) stops movement
    /// packets; this stub **drops** [`I8042::inject_mouse_packet`] while
    /// reporting is off (does not queue for later).
    #[test]
    fn mouse_inject_packet_dropped_when_reporting_disabled() {
        let mut k = I8042::new();
        assert!(!k.mouse_reporting_enabled());
        assert!(!k.inject_mouse_packet(1, 1, 0));
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);

        write_aux(&mut k, MOUSE_CMD_ENABLE_REPORTING);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        write_aux(&mut k, MOUSE_CMD_DISABLE_REPORTING);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        assert!(!k.mouse_reporting_enabled());
        assert!(!k.inject_mouse_packet(2, 2, MOUSE_BTN_RIGHT));
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);
    }

    /// Spec: IBM PS/2 KBC + OSDev packet stream — first packet byte on AUX OBF
    /// with config INT12 raises IRQ12; each `0x60` read clears then presents the
    /// next queued byte (IRQ12 re-asserts while INT12 remains enabled).
    #[test]
    fn mouse_inject_packet_with_int12_asserts_irq12_per_byte() {
        let mut k = I8042::new();
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_CONFIG));
        k.port_write(I8042_DATA, 1, u32::from(CFG_INT12));

        write_aux(&mut k, MOUSE_CMD_ENABLE_REPORTING);
        assert!(k.irq12_line());
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, MOUSE_ACK);
        assert!(!k.irq12_line());

        assert!(k.inject_mouse_packet(-1, 0, 0));
        assert!(k.irq12_line());
        assert_ne!(k.status() & STATUS_AUX_OBF, 0);
        assert_eq!(
            k.port_read(I8042_DATA, 1) as u8,
            MOUSE_PKT_ALWAYS1 | MOUSE_PKT_X_SIGN
        );
        // Next byte presented immediately → IRQ12 still high.
        assert!(k.irq12_line());
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, 0xFFu8); // dx = -1
        assert!(k.irq12_line());
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, 0);
        assert!(!k.irq12_line());
    }

    /// Spec: OSDev PS/2 Mouse packet — overflow bits set when |delta| exceeds the
    /// 9-bit movement range (X/Y > 255 or < -256).
    #[test]
    fn mouse_inject_packet_sets_overflow_flags_outside_9bit_range() {
        let mut k = I8042::new();
        write_aux(&mut k, MOUSE_CMD_ENABLE_REPORTING);
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);

        assert!(!k.inject_mouse_packet(300, -300, 0));
        let flags = read_aux_byte(&mut k);
        assert_eq!(
            flags
                & (MOUSE_PKT_ALWAYS1
                    | MOUSE_PKT_X_OVERFLOW
                    | MOUSE_PKT_Y_OVERFLOW
                    | MOUSE_PKT_Y_SIGN),
            MOUSE_PKT_ALWAYS1 | MOUSE_PKT_X_OVERFLOW | MOUSE_PKT_Y_OVERFLOW | MOUSE_PKT_Y_SIGN
        );
        assert_eq!(flags & MOUSE_PKT_X_SIGN, 0);
        let _dx = read_aux_byte(&mut k);
        let _dy = read_aux_byte(&mut k);
    }

    /// Spec: IBM PS/2 KBC — mouse stub responses set AUX OBF and raise IRQ12
    /// when config bit1 (INT12) is set; data-port read clears both.
    #[test]
    fn mouse_ack_with_int12_asserts_irq12_cleared_by_read() {
        let mut k = I8042::new();
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_CONFIG));
        k.port_write(I8042_DATA, 1, u32::from(CFG_INT12));

        write_aux(&mut k, MOUSE_CMD_ENABLE_REPORTING);
        assert!(k.irq12_line());
        assert!(!k.irq1_line());
        assert_ne!(k.status() & STATUS_AUX_OBF, 0);
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, MOUSE_ACK);
        assert!(!k.irq12_line());
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);
    }

    /// Spec: aux clock disable (config bit5) inhibits presenting mouse responses
    /// already accepted while the clock was enabled; enabling the port flushes
    /// the held device→host queue. (Host→aux `0xD4` while disabled is queued —
    /// see `write_aux_d4_queued_when_aux_clock_disabled_via_a7_flushed_on_a8`.)
    #[test]
    fn mouse_response_held_while_aux_clock_disabled_flushed_on_enable() {
        let mut k = I8042::new();
        // Deliver while clock enabled so the stub runs; ACK presents, ID queued.
        write_aux(&mut k, MOUSE_CMD_GET_DEVICE_ID);
        assert_eq!(k.last_aux_device_write, Some(MOUSE_CMD_GET_DEVICE_ID));
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_DISABLE_AUX));
        assert_eq!(read_aux_byte(&mut k), MOUSE_ACK);
        // Remaining ID stays queued while clock is disabled.
        assert_eq!(k.status() & (STATUS_OBF | STATUS_AUX_OBF), 0);

        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_ENABLE_AUX));
        assert_eq!(read_aux_byte(&mut k), MOUSE_ID_STANDARD);
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

        // Translate off (config has only INT bits): raw byte passthrough.
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
        // Aux clock enabled (reset default): deliver Reset so traffic is recorded.
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_AUX));
        k.port_write(I8042_DATA, 1, 0xFF);
        assert_eq!(k.aux_device_writes, 1);
        k.inject_aux_byte(0x08);
        assert!(k.aux_obf());
        k.reset();
        assert_eq!(k, I8042::new());
        assert!(!k.aux_obf());
        assert_eq!(k.aux_device_writes, 0);
        assert_eq!(k.last_aux_device_write, None);
    }

    /// Spec: OSDev I8042 "Translation" + Brouwer §10 — config bit6 clear:
    /// device bytes pass through unchanged on the keyboard OBF path.
    #[test]
    fn inject_scancode_passthrough_when_translate_clear() {
        let mut k = I8042::new();
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_ENABLE_KBD));
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_CONFIG));
        k.port_write(I8042_DATA, 1, 0x00);
        assert_eq!(k.config & CFG_TRANSLATE, 0);
        // Set 2 'A' (0x1C) is not remapped when translation is off.
        k.inject_scancode(0x1C);
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, 0x1C);
        k.inject_scancode(0xF0);
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, 0xF0);
    }

    /// Spec: OSDev I8042 "Translation" + Brouwer/Konzak table — config bit6 set:
    /// common Set 2 make codes become Set 1 on host OBF.
    #[test]
    fn inject_scancode_translates_set2_make_to_set1_when_bit6_set() {
        let mut k = I8042::new();
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_ENABLE_KBD));
        assert_ne!(k.config & CFG_TRANSLATE, 0); // reset default: translate on

        // Set 2 → Set 1 (Brouwer table / OSDev PS/2 Keyboard set tables):
        // A 1C→1E, Enter 5A→1C, Esc 76→01, Space 29→39, 1 16→02, Q 15→10.
        for &(set2, set1) in &[
            (0x1Cu8, 0x1Eu8),
            (0x5A, 0x1C),
            (0x76, 0x01),
            (0x29, 0x39),
            (0x16, 0x02),
            (0x15, 0x10),
        ] {
            k.inject_scancode(set2);
            assert_eq!(
                k.port_read(I8042_DATA, 1) as u8,
                set1,
                "Set2 {set2:#04x} should become Set1 {set1:#04x}"
            );
        }
    }

    /// Spec: Brouwer §10 — Set 2 break is `F0` + code; 8042 consumes `F0` and
    /// ORs `0x80` onto the translated next byte (Set 1 break = make|0x80).
    #[test]
    fn inject_scancode_translates_set2_break_f0_prefix() {
        let mut k = I8042::new();
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_ENABLE_KBD));
        assert_ne!(k.config & CFG_TRANSLATE, 0);

        // Set 2 break for 'A': F0 1C → host Set 1 break 0x9E (0x1E|0x80).
        assert!(!k.inject_scancode(0xF0)); // consumed; no OBF
        assert_eq!(k.status() & STATUS_OBF, 0);
        assert_eq!(k.output_buffer(), None);

        assert!(!k.inject_scancode(0x1C)); // INT1 clear → no IRQ edge
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, 0x9E);
    }

    /// Spec: OSDev PS/2 Keyboard + Brouwer — extended keys keep `E0` prefix;
    /// the following Set 2 code is translated (Right arrow Set2 E0 74 → Set1 E0 4D).
    #[test]
    fn inject_scancode_translates_extended_e0_prefix() {
        let mut k = I8042::new();
        k.port_write(I8042_STATUS_CMD, 1, u32::from(CMD_ENABLE_KBD));
        assert_ne!(k.config & CFG_TRANSLATE, 0);

        // Make: E0 74 → E0 4D
        k.inject_scancode(0xE0);
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, 0xE0);
        k.inject_scancode(0x74);
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, 0x4D);

        // Break: E0 F0 74 → E0 CD (0x4D|0x80)
        k.inject_scancode(0xE0);
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, 0xE0);
        assert!(!k.inject_scancode(0xF0));
        assert_eq!(k.status() & STATUS_OBF, 0);
        k.inject_scancode(0x74);
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, 0xCD);
    }

    /// Spec: translation applies only on the keyboard inject path; aux/mouse
    /// bytes are never Set2→Set1 remapped.
    #[test]
    fn aux_inject_not_translated_by_config_bit6() {
        let mut k = I8042::new();
        assert_ne!(k.config & CFG_TRANSLATE, 0);
        k.inject_aux_byte(0x1C);
        assert_eq!(k.port_read(I8042_DATA, 1) as u8, 0x1C);
    }
}
