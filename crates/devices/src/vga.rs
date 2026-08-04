//! VGA color text-mode frame buffer MMIO stub (physical `0xB8000`) plus CRTC
//! index/data port stub (`0x3D4`/`0x3D5`), Sequencer index/data stub
//! (`0x3C4`/`0x3C5`), Graphics Controller index/data stub (`0x3CE`/`0x3CF`),
//! Attribute Controller address/data flip-flop stub (`0x3C0`/`0x3C1` + Input
//! Status #1 flip-flop reset and status bits at `0x3DA`/`0x3BA` via Misc IOAS),
//! and Miscellaneous Output Register stub (`0x3C2` write / `0x3CC` readback).
//!
//! # Spec refs
//!
//! - IBM VGA / classic PC: color text frame buffer at physical `0xB8000`,
//!   80×25 cells, 2 bytes per cell (ASCII character, attribute).
//! - OSDev Text UI — memory layout for mode 03h text (char at even offset,
//!   attribute at odd); window commonly treated as 32 KiB (`0xB8000`–`0xBFFFF`).
//! - OSDev VGA Hardware / FreeVGA CRT Controller — color CRTC Address Register
//!   at `0x3D4`, Data Register at `0x3D5`; standard VGA has 25 CRTC registers
//!   (indexes `0x00`–`0x18`).
//! - OSDev VGA Hardware / FreeVGA Sequencer Registers — Address `0x3C4`, Data
//!   `0x3C5`; indexes `0x00`–`0x04` (Reset, Clocking Mode, Map Mask, Character
//!   Map Select, Memory Mode).
//! - OSDev VGA Hardware / FreeVGA Graphics Registers — Address `0x3CE`, Data
//!   `0x3CF`; indexes `0x00`–`0x08` (Set/Reset, Enable Set/Reset, Color Compare,
//!   Data Rotate, Read Map Select, Graphics Mode, Miscellaneous, Color Don't
//!   Care, Bit Mask).
//! - OSDev VGA Hardware / FreeVGA Attribute Controller Registers — Address/Data
//!   at `0x3C0` (flip-flop), Data Read at `0x3C1`; indexes `0x00`–`0x14`
//!   (palette `0x00`–`0x0F`, Mode Control `0x10`, Overscan `0x11`, Color Plane
//!   Enable `0x12`, Horizontal PEL Panning `0x13`, Color Select `0x14`). Reading
//!   Input Status #1 (color `0x3DA` / mono `0x3BA`) resets the flip-flop to
//!   address state.
//! - OSDev VGA Hardware / FreeVGA External Registers — Input Status #1 read at
//!   `0x3DA` (color) / `0x3BA` (mono): bit0 Display Disabled (inverted
//!   display-enable; set during horizontal or vertical retrace), bit3 Vertical
//!   Retrace.
//! - OSDev VGA Hardware / FreeVGA / IBM VGA Miscellaneous Output Register —
//!   write port `0x3C2`, readback port `0x3CC` (write-only at `0x3C2`); bit0
//!   I/O Address Select (IOAS): `1` = color CRTC/status map (`0x3D4`/`0x3D5`,
//!   Input Status #1 `0x3DA`); `0` = mono map (CRTC `0x3B4`/`0x3B5`, Input
//!   Status #1 `0x3BA`).
//! - `docs/machine-model-pc-v1.md`, `plan.md` §15.6 / §21 VGA text mode.
//!
//! # Scope (this slice)
//!
//! - 32 KiB text plane buffer at `VGA_TEXT_BASE`…`VGA_TEXT_END`
//! - Byte R/W; reset fills first 80×25 with space + attribute `0x07`
//! - Helpers for tests (`char_at` / `attr_at` / `put_char`)
//! - CRTC index/data noop: latch index on `0x3D4`, store/read register file on
//!   `0x3D5` (no timing, cursor render, or protect-bit enforcement)
//! - Sequencer index/data noop: latch index on `0x3C4`, store/read register file
//!   on `0x3C5` with mode-03h-class reset defaults (no timing/plane side effects)
//! - Graphics Controller index/data noop: latch index on `0x3CE`, store/read
//!   register file on `0x3CF` with mode-03h-class reset defaults (no write-mode /
//!   map / bitmask side effects)
//! - Attribute Controller noop: address/data flip-flop on `0x3C0`, data read on
//!   `0x3C1`, flip-flop reset via Input Status #1 (active IOAS map) (no palette /
//!   mode-control / render side effects)
//! - Input Status #1: ATC flip-flop reset + deterministic display-enable /
//!   vertical-retrace status bits (read-phase counter); port selected by Misc
//!   Output IOAS (`0x3DA` color / `0x3BA` mono)
//! - Misc Output store/readback (`0x3C2`/`0x3CC`); IOAS bit remaps Input Status
//!   #1 ownership only (not CRTC mono map, clock, or RAM-enable)
//!
//! # Unsupported (explicit)
//!
//! - ATC / Sequencer / GC timing, palette→DAC, blink, PEL pan, plane-enable,
//!   map-mask, write-mode, read-map, or bitmask side effects on the text plane
//! - CRTC-timed Input Status #1 accuracy, vertical-retrace IRQ, Feature Control
//!   diagnostic bits
//! - CRTC protect bit (index `0x11` bit7), mono CRTC map at `0x3B4`/`0x3B5`
//! - Misc Output bit side effects beyond Input Status #1 IOAS (clock select,
//!   RAM enable)
//! - Planar graphics, VBE, host canvas rendering, dirty tracking
//! - Font ROM, hardware cursor position driven from CRTC into the plane

use crate::PortDevice;

/// Physical base of color text frame buffer (IBM VGA mode 03h).
pub const VGA_TEXT_BASE: u64 = 0x000B_8000;
/// Exclusive end of the 32 KiB text plane (`0xB8000`–`0xBFFFF`).
pub const VGA_TEXT_END: u64 = 0x000C_0000;
/// Bytes in the text plane window.
pub const VGA_TEXT_SIZE: usize = (VGA_TEXT_END - VGA_TEXT_BASE) as usize;
/// Columns in default 80×25 text mode.
pub const VGA_TEXT_COLS: usize = 80;
/// Rows in default 80×25 text mode.
pub const VGA_TEXT_ROWS: usize = 25;
/// Bytes per character cell (char + attribute).
pub const VGA_CELL_BYTES: usize = 2;
/// Default attribute: light gray on black (classic BIOS text).
pub const VGA_DEFAULT_ATTR: u8 = 0x07;
/// Default fill character (ASCII space).
pub const VGA_DEFAULT_CHAR: u8 = b' ';

/// Color CRTC Address (index) Register. Spec: OSDev VGA Hardware / FreeVGA.
pub const VGA_CRTC_INDEX: u16 = 0x3D4;
/// Color CRTC Data Register.
pub const VGA_CRTC_DATA: u16 = 0x3D5;
/// Number of standard VGA CRTC registers (indexes `0x00`–`0x18`).
pub const VGA_CRTC_REG_COUNT: usize = 0x19;

/// Sequencer Address (index) Register.
///
/// Spec: FreeVGA / OSDev VGA Hardware / IBM VGA — Sequencer Address at `0x3C4`,
/// Data at `0x3C5`.
pub const VGA_SEQ_INDEX: u16 = 0x3C4;
/// Sequencer Data Register.
pub const VGA_SEQ_DATA: u16 = 0x3C5;
/// Number of standard VGA Sequencer registers (indexes `0x00`–`0x04`).
pub const VGA_SEQ_REG_COUNT: usize = 5;

/// Mode-03h-class Sequencer reset defaults (store/readback only).
///
/// Spec: FreeVGA / IBM VGA alphanumeric programming SeaBIOS probes —
/// Reset `0x03` (both reset bits clear → run), Clocking Mode `0x00`,
/// Map Mask `0x03` (planes 0+1), Character Map Select `0x00`,
/// Memory Mode `0x02` (extended memory enable; odd/even + chain-4 clear).
pub const VGA_SEQ_DEFAULTS: [u8; VGA_SEQ_REG_COUNT] = [0x03, 0x00, 0x03, 0x00, 0x02];

/// Miscellaneous Output Register write port.
///
/// Spec: OSDev VGA Hardware / FreeVGA — MOR is write-only at `0x3C2`;
/// readback is at `0x3CC`.
pub const VGA_MISC_OUTPUT_WRITE: u16 = 0x3C2;
/// Miscellaneous Output Register readback port (`0x3CC`).
pub const VGA_MISC_OUTPUT_READ: u16 = 0x3CC;
/// Reset / BIOS text-mode-ish Misc Output default (`0x67`).
///
/// Spec: FreeVGA / OSDev Misc Output — common mode-03h programming selects
/// color CRTC map (IOAS), RAM enable, and 25 MHz-class clock select bits
/// (`0x67` = color + enable RAM + typical text-mode polarity/clock nibble).
pub const VGA_MISC_OUTPUT_DEFAULT: u8 = 0x67;
/// Miscellaneous Output bit0 — I/O Address Select (IOAS).
///
/// Spec: FreeVGA / IBM VGA Misc Output — `1` = color I/O map (`0x3Dx` CRTC +
/// Input Status #1 `0x3DA`); `0` = mono I/O map (`0x3Bx` CRTC + Input Status #1
/// `0x3BA`). This stub remaps Input Status #1 ownership only.
pub const VGA_MISC_IOAS: u8 = 0x01;

/// Graphics Controller Address (index) Register.
///
/// Spec: FreeVGA / OSDev VGA Hardware / IBM VGA — Graphics Address at `0x3CE`,
/// Data at `0x3CF`.
pub const VGA_GC_INDEX: u16 = 0x3CE;
/// Graphics Controller Data Register.
pub const VGA_GC_DATA: u16 = 0x3CF;
/// Number of standard VGA Graphics Controller registers (indexes `0x00`–`0x08`).
pub const VGA_GC_REG_COUNT: usize = 9;

/// Mode-03h-class Graphics Controller reset defaults (store/readback only).
///
/// Spec: FreeVGA Graphics Registers / OSDev VGA Hardware / IBM VGA mode-03h —
/// SeaBIOS-class text programming: Set/Reset `0x00`, Enable Set/Reset `0x00`,
/// Color Compare `0x00`, Data Rotate `0x00`, Read Map Select `0x00`,
/// Graphics Mode `0x10` (host odd/even), Miscellaneous `0x0E` (odd/even +
/// memory map `B8000`), Color Don't Care `0x00`, Bit Mask `0xFF`.
pub const VGA_GC_DEFAULTS: [u8; VGA_GC_REG_COUNT] =
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x0E, 0x00, 0xFF];

/// Attribute Controller Address/Data Register (flip-flop).
///
/// Spec: FreeVGA Attribute Controller Registers / Accessing the Attribute
/// Registers / IBM VGA — write index then data to `0x3C0`; read index from
/// `0x3C0`; read data from `0x3C1`.
pub const VGA_ATC_ADDRESS_DATA: u16 = 0x3C0;
/// Attribute Controller Data Read Register.
pub const VGA_ATC_DATA_READ: u16 = 0x3C1;
/// Color Input Status #1 — reading resets the ATC address/data flip-flop and
/// returns display/retrace status bits when Misc Output IOAS selects color.
///
/// Spec: FreeVGA External Registers / IBM VGA / OSDev VGA Hardware — port
/// `0x3DA` (color); mono alias is [`VGA_INPUT_STATUS_1_MONO`].
pub const VGA_INPUT_STATUS_1: u16 = 0x3DA;
/// Mono Input Status #1 alias — same status model + ATC flip-flop reset when
/// Misc Output IOAS selects mono.
///
/// Spec: FreeVGA External Registers / IBM VGA / Misc Output bit0 (IOAS) —
/// mono I/O map places Input Status #1 at `0x3BA`.
pub const VGA_INPUT_STATUS_1_MONO: u16 = 0x3BA;
/// Input Status #1 bit0 — Display Disabled (inverted display-enable).
///
/// Spec: FreeVGA External Registers — set during horizontal or vertical retrace.
pub const VGA_STATUS1_DD: u8 = 0x01;
/// Input Status #1 bit3 — Vertical Retrace.
///
/// Spec: FreeVGA External Registers — set during the vertical retrace interval.
pub const VGA_STATUS1_VR: u8 = 0x08;
/// Period of the deterministic Input Status #1 read-phase counter.
///
/// Model choice (documented, not CRTC-timed): even phases are display-active
/// (`DD=0`, `VR=0`); odd phases are vertical retrace (`DD=1`, `VR=1`). Advances
/// only on active-map status reads (`0x3DA` or `0x3BA` per IOAS) so SeaBIOS-style
/// wait-for-VR / wait-for-end-VR loops terminate without a machine tick hook.
pub const VGA_STATUS1_PHASE_PERIOD: u8 = 2;
/// Number of standard VGA Attribute Controller registers (`0x00`–`0x14`).
pub const VGA_ATC_REG_COUNT: usize = 0x15;
/// PAS bit in the Attribute Address register (bit 5).
pub const VGA_ATC_PAS: u8 = 0x20;
/// Reset / BIOS text-mode-ish Attribute Address default (`PAS=1`, index 0).
///
/// Spec: Ralf Brown Interrupt List / IBM VGA — index register often left with
/// Palette Address Source set after mode programming.
pub const VGA_ATC_INDEX_DEFAULT: u8 = VGA_ATC_PAS;

/// Mode-03h-class Attribute Controller reset defaults (store/readback only).
///
/// Spec: FreeVGA / IBM VGA / Abrash mode-set palette — internal palette
/// `00/01/02/03/04/05/14/07/38/39/3A/3B/3C/3D/3E/3F`; Mode Control `0x0C`
/// (BLINK|LGE, alphanumeric); Overscan `0x00`; Color Plane Enable `0x0F`;
/// Horizontal PEL Panning `0x08`; Color Select `0x00`.
pub const VGA_ATC_DEFAULTS: [u8; VGA_ATC_REG_COUNT] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x14, 0x07, 0x38, 0x39, 0x3A, 0x3B, 0x3C, 0x3D, 0x3E, 0x3F,
    0x0C, 0x00, 0x0F, 0x08, 0x00,
];

/// Color text-mode frame buffer + CRTC + Sequencer + GC + ATC + Misc stubs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VgaText {
    /// Raw plane bytes (char/attr interleaved).
    pub mem: Vec<u8>,
    /// Latched CRTC index (written via `0x3D4`).
    pub crtc_index: u8,
    /// CRTC register file (noop store/readback).
    pub crtc_regs: [u8; VGA_CRTC_REG_COUNT],
    /// Latched Sequencer index (written via `0x3C4`).
    pub seq_index: u8,
    /// Sequencer register file (noop store/readback).
    pub seq_regs: [u8; VGA_SEQ_REG_COUNT],
    /// Latched Graphics Controller index (written via `0x3CE`).
    pub gc_index: u8,
    /// Graphics Controller register file (noop store/readback).
    pub gc_regs: [u8; VGA_GC_REG_COUNT],
    /// Attribute Address register (bits 4:0 index, bit 5 PAS).
    pub atc_index: u8,
    /// Attribute Controller register file (noop store/readback).
    pub atc_regs: [u8; VGA_ATC_REG_COUNT],
    /// ATC flip-flop: `false` = next `0x3C0` write is address; `true` = data.
    pub atc_flip_flop_data: bool,
    /// Deterministic Input Status #1 phase (advances on each active-map status
    /// read — `0x3DA` or `0x3BA` per Misc IOAS).
    ///
    /// See [`VGA_STATUS1_PHASE_PERIOD`] for the active/retrace model.
    pub status1_phase: u8,
    /// Miscellaneous Output Register (store via `0x3C2`, read via `0x3CC`).
    ///
    /// Bit0 ([`VGA_MISC_IOAS`]) selects Input Status #1 port ownership.
    pub misc_output: u8,
}

impl Default for VgaText {
    fn default() -> Self {
        Self::new()
    }
}

impl VgaText {
    pub fn new() -> Self {
        let mut v = Self {
            mem: vec![0; VGA_TEXT_SIZE],
            crtc_index: 0,
            crtc_regs: [0; VGA_CRTC_REG_COUNT],
            seq_index: 0,
            seq_regs: VGA_SEQ_DEFAULTS,
            gc_index: 0,
            gc_regs: VGA_GC_DEFAULTS,
            atc_index: VGA_ATC_INDEX_DEFAULT,
            atc_regs: VGA_ATC_DEFAULTS,
            atc_flip_flop_data: false,
            status1_phase: 0,
            misc_output: VGA_MISC_OUTPUT_DEFAULT,
        };
        v.reset();
        v
    }

    /// Reset text plane: 80×25 → space/`0x07`; remainder cleared; CRTC cleared;
    /// Sequencer restored to [`VGA_SEQ_DEFAULTS`]; Graphics Controller restored
    /// to [`VGA_GC_DEFAULTS`]; Attribute Controller restored to
    /// [`VGA_ATC_DEFAULTS`] with flip-flop in address state; Input Status #1
    /// phase cleared; Misc Output restored to [`VGA_MISC_OUTPUT_DEFAULT`].
    ///
    /// Spec: IBM VGA text — cells are (char, attr) pairs starting at `0xB8000`.
    pub fn reset(&mut self) {
        self.mem.fill(0);
        let cells = VGA_TEXT_COLS * VGA_TEXT_ROWS;
        for i in 0..cells {
            let off = i * VGA_CELL_BYTES;
            self.mem[off] = VGA_DEFAULT_CHAR;
            self.mem[off + 1] = VGA_DEFAULT_ATTR;
        }
        self.crtc_index = 0;
        self.crtc_regs = [0; VGA_CRTC_REG_COUNT];
        self.seq_index = 0;
        self.seq_regs = VGA_SEQ_DEFAULTS;
        self.gc_index = 0;
        self.gc_regs = VGA_GC_DEFAULTS;
        self.atc_index = VGA_ATC_INDEX_DEFAULT;
        self.atc_regs = VGA_ATC_DEFAULTS;
        self.atc_flip_flop_data = false;
        self.status1_phase = 0;
        self.misc_output = VGA_MISC_OUTPUT_DEFAULT;
    }

    /// True if `addr` (after A20) falls in the text plane.
    pub fn owns_addr(addr: u64) -> bool {
        (VGA_TEXT_BASE..VGA_TEXT_END).contains(&addr)
    }

    /// True when Misc Output IOAS selects the color I/O map (bit0 = 1).
    ///
    /// Spec: FreeVGA / IBM VGA Misc Output — IOAS selects `0x3Dx` vs `0x3Bx`.
    pub fn misc_ioas_color(&self) -> bool {
        self.misc_output & VGA_MISC_IOAS != 0
    }

    /// True if this device owns the I/O port (color CRTC + Sequencer + GC + ATC
    /// + Input Status #1 at the IOAS-selected address + Misc).
    ///
    /// Spec: FreeVGA / IBM — Input Status #1 is `0x3DA` when IOAS=1 (color) and
    /// `0x3BA` when IOAS=0 (mono). CRTC remains color `0x3D4`/`0x3D5` in this
    /// stub (mono CRTC remap is unsupported).
    pub fn owns_port(&self, port: u16) -> bool {
        match port {
            VGA_INPUT_STATUS_1 => self.misc_ioas_color(),
            VGA_INPUT_STATUS_1_MONO => !self.misc_ioas_color(),
            VGA_CRTC_INDEX
            | VGA_CRTC_DATA
            | VGA_SEQ_INDEX
            | VGA_SEQ_DATA
            | VGA_GC_INDEX
            | VGA_GC_DATA
            | VGA_ATC_ADDRESS_DATA
            | VGA_ATC_DATA_READ
            | VGA_MISC_OUTPUT_WRITE
            | VGA_MISC_OUTPUT_READ => true,
            _ => false,
        }
    }

    pub fn read_u8(&self, addr: u64) -> Option<u8> {
        if !Self::owns_addr(addr) {
            return None;
        }
        let off = (addr - VGA_TEXT_BASE) as usize;
        Some(self.mem[off])
    }

    pub fn write_u8(&mut self, addr: u64, val: u8) -> bool {
        if !Self::owns_addr(addr) {
            return false;
        }
        let off = (addr - VGA_TEXT_BASE) as usize;
        self.mem[off] = val;
        true
    }

    fn cell_offset(row: usize, col: usize) -> Option<usize> {
        if row >= VGA_TEXT_ROWS || col >= VGA_TEXT_COLS {
            return None;
        }
        Some((row * VGA_TEXT_COLS + col) * VGA_CELL_BYTES)
    }

    pub fn char_at(&self, row: usize, col: usize) -> Option<u8> {
        let off = Self::cell_offset(row, col)?;
        Some(self.mem[off])
    }

    pub fn attr_at(&self, row: usize, col: usize) -> Option<u8> {
        let off = Self::cell_offset(row, col)?;
        Some(self.mem[off + 1])
    }

    pub fn put_char(&mut self, row: usize, col: usize, ch: u8, attr: u8) -> bool {
        let Some(off) = Self::cell_offset(row, col) else {
            return false;
        };
        self.mem[off] = ch;
        self.mem[off + 1] = attr;
        true
    }

    fn crtc_index_masked(index: u8) -> Option<usize> {
        let i = usize::from(index);
        if i < VGA_CRTC_REG_COUNT {
            Some(i)
        } else {
            None
        }
    }

    fn write_crtc_index(&mut self, value: u8) {
        self.crtc_index = value;
    }

    fn write_crtc_data(&mut self, value: u8) {
        if let Some(i) = Self::crtc_index_masked(self.crtc_index) {
            self.crtc_regs[i] = value;
        }
    }

    fn read_crtc_index(&self) -> u8 {
        self.crtc_index
    }

    fn read_crtc_data(&self) -> u8 {
        Self::crtc_index_masked(self.crtc_index)
            .map(|i| self.crtc_regs[i])
            .unwrap_or(0)
    }

    fn seq_index_masked(index: u8) -> Option<usize> {
        let i = usize::from(index);
        if i < VGA_SEQ_REG_COUNT {
            Some(i)
        } else {
            None
        }
    }

    fn write_seq_index(&mut self, value: u8) {
        self.seq_index = value;
    }

    fn write_seq_data(&mut self, value: u8) {
        // Store only — map-mask / memory-mode / clocking bits are not enforced.
        if let Some(i) = Self::seq_index_masked(self.seq_index) {
            self.seq_regs[i] = value;
        }
    }

    fn read_seq_index(&self) -> u8 {
        self.seq_index
    }

    fn read_seq_data(&self) -> u8 {
        Self::seq_index_masked(self.seq_index)
            .map(|i| self.seq_regs[i])
            .unwrap_or(0)
    }

    fn gc_index_masked(index: u8) -> Option<usize> {
        let i = usize::from(index);
        if i < VGA_GC_REG_COUNT {
            Some(i)
        } else {
            None
        }
    }

    fn write_gc_index(&mut self, value: u8) {
        self.gc_index = value;
    }

    fn write_gc_data(&mut self, value: u8) {
        // Store only — write-mode / map / bitmask bits are not enforced.
        if let Some(i) = Self::gc_index_masked(self.gc_index) {
            self.gc_regs[i] = value;
        }
    }

    fn read_gc_index(&self) -> u8 {
        self.gc_index
    }

    fn read_gc_data(&self) -> u8 {
        Self::gc_index_masked(self.gc_index)
            .map(|i| self.gc_regs[i])
            .unwrap_or(0)
    }

    fn write_misc_output(&mut self, value: u8) {
        // Store; IOAS (bit0) remaps Input Status #1 ownership. Clock select and
        // RAM-enable bits are not enforced; mono CRTC `0x3B4`/`0x3B5` is not.
        self.misc_output = value;
    }

    fn read_misc_output(&self) -> u8 {
        self.misc_output
    }

    fn atc_index_masked(index: u8) -> Option<usize> {
        let i = usize::from(index & 0x1F);
        if i < VGA_ATC_REG_COUNT {
            Some(i)
        } else {
            None
        }
    }

    /// Reset ATC flip-flop so the next `0x3C0` write is an address byte.
    ///
    /// Spec: FreeVGA Accessing the Attribute Registers — read Input Status #1.
    fn reset_atc_flip_flop(&mut self) {
        self.atc_flip_flop_data = false;
    }

    fn write_atc_address_data(&mut self, value: u8) {
        if !self.atc_flip_flop_data {
            // Address write: bits 4:0 = index, bit 5 = PAS; flip to data.
            self.atc_index = value & 0x3F;
            self.atc_flip_flop_data = true;
        } else {
            // Data write for latched index; flip back to address.
            // Store only — palette / mode-control / plane bits are not enforced.
            if let Some(i) = Self::atc_index_masked(self.atc_index) {
                self.atc_regs[i] = value;
            }
            self.atc_flip_flop_data = false;
        }
    }

    fn read_atc_address(&self) -> u8 {
        // Spec: VGA — `0x3C0` read returns the Attribute Address register
        // (index + PAS); does not toggle the flip-flop.
        self.atc_index
    }

    fn read_atc_data(&self) -> u8 {
        // Spec: FreeVGA — `0x3C1` read returns selected data; no flip-flop toggle.
        Self::atc_index_masked(self.atc_index)
            .map(|i| self.atc_regs[i])
            .unwrap_or(0)
    }

    fn read_input_status_1(&mut self) -> u8 {
        // Spec: FreeVGA Accessing the Attribute Registers / External Registers —
        // read resets ATC flip-flop to address state and returns status bits.
        self.reset_atc_flip_flop();

        // Deterministic read-phase model (not CRTC-timed): even → display active
        // (DD=0, VR=0); odd → vertical retrace with display disabled (DD|VR).
        // Advances only here so firmware poll loops terminate without a tick.
        let phase = self.status1_phase;
        self.status1_phase = (self.status1_phase + 1) % VGA_STATUS1_PHASE_PERIOD;
        if phase.is_multiple_of(2) {
            0
        } else {
            VGA_STATUS1_DD | VGA_STATUS1_VR
        }
    }
}

impl PortDevice for VgaText {
    fn port_read(&mut self, port: u16, _size: u8) -> u32 {
        match port {
            VGA_CRTC_INDEX => u32::from(self.read_crtc_index()),
            VGA_CRTC_DATA => u32::from(self.read_crtc_data()),
            VGA_SEQ_INDEX => u32::from(self.read_seq_index()),
            VGA_SEQ_DATA => u32::from(self.read_seq_data()),
            VGA_GC_INDEX => u32::from(self.read_gc_index()),
            VGA_GC_DATA => u32::from(self.read_gc_data()),
            VGA_ATC_ADDRESS_DATA => u32::from(self.read_atc_address()),
            VGA_ATC_DATA_READ => u32::from(self.read_atc_data()),
            VGA_INPUT_STATUS_1 if self.misc_ioas_color() => u32::from(self.read_input_status_1()),
            VGA_INPUT_STATUS_1_MONO if !self.misc_ioas_color() => {
                u32::from(self.read_input_status_1())
            }
            // Spec: FreeVGA / OSDev — `0x3C2` is write-only; read is undefined.
            // Stub returns open-bus-style `0xFF` (use `0x3CC` for readback).
            VGA_MISC_OUTPUT_WRITE => 0xFF,
            VGA_MISC_OUTPUT_READ => u32::from(self.read_misc_output()),
            _ => 0xFFFF_FFFF,
        }
    }

    fn port_write(&mut self, port: u16, size: u8, value: u32) {
        match port {
            VGA_CRTC_INDEX => {
                // Spec: OSDev VGA Hardware — some guests write index+data as a
                // single word to 0x3D4 (low = index, high = data).
                if size >= 2 {
                    self.write_crtc_index(value as u8);
                    self.write_crtc_data((value >> 8) as u8);
                } else {
                    self.write_crtc_index(value as u8);
                }
            }
            VGA_CRTC_DATA => self.write_crtc_data(value as u8),
            VGA_SEQ_INDEX => {
                // Mirror CRTC: 16-bit write to 0x3C4 (lo=index, hi=data).
                if size >= 2 {
                    self.write_seq_index(value as u8);
                    self.write_seq_data((value >> 8) as u8);
                } else {
                    self.write_seq_index(value as u8);
                }
            }
            VGA_SEQ_DATA => self.write_seq_data(value as u8),
            VGA_GC_INDEX => {
                // Mirror CRTC/Sequencer: 16-bit write to 0x3CE (lo=index, hi=data).
                if size >= 2 {
                    self.write_gc_index(value as u8);
                    self.write_gc_data((value >> 8) as u8);
                } else {
                    self.write_gc_index(value as u8);
                }
            }
            VGA_GC_DATA => self.write_gc_data(value as u8),
            VGA_ATC_ADDRESS_DATA => {
                // Spec: FreeVGA — consecutive writes to 0x3C0 are index then data.
                // A 16-bit OUT writes lo=index then hi=data through the flip-flop.
                if size >= 2 {
                    self.write_atc_address_data(value as u8);
                    self.write_atc_address_data((value >> 8) as u8);
                } else {
                    self.write_atc_address_data(value as u8);
                }
            }
            // Spec: FreeVGA — `0x3C1` is data-read; writes ignored.
            VGA_ATC_DATA_READ => {}
            // Spec: FreeVGA — Input Status #1 is read-only; writes ignored.
            VGA_INPUT_STATUS_1 | VGA_INPUT_STATUS_1_MONO => {}
            VGA_MISC_OUTPUT_WRITE => self.write_misc_output(value as u8),
            // Spec: FreeVGA / OSDev — `0x3CC` is read-only; writes ignored.
            VGA_MISC_OUTPUT_READ => {}
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_fills_80x25_space_attr07() {
        // Spec: IBM VGA text — default light-gray-on-black cells.
        let v = VgaText::new();
        assert_eq!(v.mem.len(), VGA_TEXT_SIZE);
        assert_eq!(v.char_at(0, 0), Some(b' '));
        assert_eq!(v.attr_at(0, 0), Some(0x07));
        assert_eq!(v.char_at(24, 79), Some(b' '));
        assert_eq!(v.attr_at(24, 79), Some(0x07));
        // Beyond 80×25 within the 32 KiB plane remains 0 after reset.
        assert_eq!(v.mem[80 * 25 * 2], 0);
        assert_eq!(v.crtc_index, 0);
        assert_eq!(v.crtc_regs, [0; VGA_CRTC_REG_COUNT]);
        assert_eq!(v.seq_regs, VGA_SEQ_DEFAULTS);
        assert_eq!(v.gc_regs, VGA_GC_DEFAULTS);
        assert_eq!(v.atc_index, VGA_ATC_INDEX_DEFAULT);
        assert_eq!(v.atc_regs, VGA_ATC_DEFAULTS);
        assert!(!v.atc_flip_flop_data);
        assert_eq!(v.misc_output, VGA_MISC_OUTPUT_DEFAULT);
    }

    #[test]
    fn b8000_char_attr_round_trip() {
        let mut v = VgaText::new();
        assert!(v.write_u8(VGA_TEXT_BASE, b'H'));
        assert!(v.write_u8(VGA_TEXT_BASE + 1, 0x1E));
        assert_eq!(v.read_u8(VGA_TEXT_BASE), Some(b'H'));
        assert_eq!(v.read_u8(VGA_TEXT_BASE + 1), Some(0x1E));
        assert_eq!(v.char_at(0, 0), Some(b'H'));
        assert_eq!(v.attr_at(0, 0), Some(0x1E));
    }

    #[test]
    fn attr_byte_is_odd_offset() {
        // Spec: OSDev Text UI — attribute is the odd byte of the cell.
        let mut v = VgaText::new();
        v.put_char(0, 0, b'A', 0x2F);
        assert_eq!(v.read_u8(0xB8001), Some(0x2F));
    }

    #[test]
    fn outside_window_not_owned() {
        let v = VgaText::new();
        assert!(!VgaText::owns_addr(0xA0000));
        assert!(!VgaText::owns_addr(0xC0000));
        assert!(VgaText::owns_addr(0xB8000));
        assert!(VgaText::owns_addr(0xBFFFF));
        assert!(!VgaText::owns_addr(0xC0000));
        assert_eq!(v.read_u8(0xA0000), None);
        assert!(!{
            let mut v = VgaText::new();
            v.write_u8(0xC0000, 0x55)
        });
    }

    #[test]
    fn crtc_index_data_round_trip() {
        // Spec: OSDev VGA Hardware / FreeVGA — write index 0x3D4, data 0x3D5.
        let mut v = VgaText::new();
        assert!(v.owns_port(VGA_CRTC_INDEX));
        assert!(v.owns_port(VGA_CRTC_DATA));
        assert!(!v.owns_port(0x3B4));
        v.port_write(VGA_CRTC_INDEX, 1, 0x0E); // cursor location high
        v.port_write(VGA_CRTC_DATA, 1, 0x12);
        v.port_write(VGA_CRTC_INDEX, 1, 0x0F); // cursor location low
        v.port_write(VGA_CRTC_DATA, 1, 0x34);
        assert_eq!(v.crtc_regs[0x0E], 0x12);
        assert_eq!(v.crtc_regs[0x0F], 0x34);
        v.port_write(VGA_CRTC_INDEX, 1, 0x0E);
        assert_eq!(v.port_read(VGA_CRTC_INDEX, 1) as u8, 0x0E);
        assert_eq!(v.port_read(VGA_CRTC_DATA, 1) as u8, 0x12);
        v.port_write(VGA_CRTC_INDEX, 1, 0x0F);
        assert_eq!(v.port_read(VGA_CRTC_DATA, 1) as u8, 0x34);
    }

    #[test]
    fn crtc_word_write_index_and_data() {
        // Spec: OSDev VGA Hardware — 16-bit write to 0x3D4 (lo=index, hi=data).
        let mut v = VgaText::new();
        v.port_write(VGA_CRTC_INDEX, 2, 0xAB_0C);
        assert_eq!(v.crtc_index, 0x0C);
        assert_eq!(v.crtc_regs[0x0C], 0xAB);
        assert_eq!(v.port_read(VGA_CRTC_DATA, 1) as u8, 0xAB);
    }

    #[test]
    fn crtc_out_of_range_index_ignored_on_data() {
        let mut v = VgaText::new();
        v.port_write(VGA_CRTC_INDEX, 1, 0x20);
        v.port_write(VGA_CRTC_DATA, 1, 0x55);
        assert_eq!(v.crtc_regs, [0; VGA_CRTC_REG_COUNT]);
        assert_eq!(v.port_read(VGA_CRTC_DATA, 1) as u8, 0);
    }

    #[test]
    fn reset_clears_crtc_regs() {
        let mut v = VgaText::new();
        v.port_write(VGA_CRTC_INDEX, 1, 0x07);
        v.port_write(VGA_CRTC_DATA, 1, 0x99);
        v.reset();
        assert_eq!(v.crtc_index, 0);
        assert_eq!(v.crtc_regs[0x07], 0);
    }

    #[test]
    fn misc_output_owns_ports_with_crtc_not_mono() {
        // Spec: FreeVGA / OSDev — Misc Output write `0x3C2`, read `0x3CC`;
        // color CRTC remains `0x3D4`/`0x3D5`; mono `0x3B4`/`0x3B5` not owned.
        let v = VgaText::new();
        assert!(v.owns_port(VGA_MISC_OUTPUT_WRITE));
        assert!(v.owns_port(VGA_MISC_OUTPUT_READ));
        assert!(v.owns_port(VGA_CRTC_INDEX));
        assert!(v.owns_port(VGA_CRTC_DATA));
        assert!(!v.owns_port(0x3B4));
        assert!(!v.owns_port(0x3B5));
    }

    #[test]
    fn misc_output_write_3c2_readback_3cc() {
        // Spec: FreeVGA / OSDev VGA Hardware — write MOR at 0x3C2, read at 0x3CC.
        let mut v = VgaText::new();
        assert_eq!(v.misc_output, VGA_MISC_OUTPUT_DEFAULT);
        assert_eq!(
            v.port_read(VGA_MISC_OUTPUT_READ, 1) as u8,
            VGA_MISC_OUTPUT_DEFAULT
        );
        v.port_write(VGA_MISC_OUTPUT_WRITE, 1, 0xA5);
        assert_eq!(v.misc_output, 0xA5);
        assert_eq!(v.port_read(VGA_MISC_OUTPUT_READ, 1) as u8, 0xA5);
    }

    #[test]
    fn misc_output_3c2_read_is_open_bus() {
        // Spec: FreeVGA / OSDev — 0x3C2 is write-only; read undefined.
        // Choice: return 0xFF (open-bus style); guests must use 0x3CC.
        let mut v = VgaText::new();
        v.port_write(VGA_MISC_OUTPUT_WRITE, 1, 0x67);
        assert_eq!(v.port_read(VGA_MISC_OUTPUT_WRITE, 1) as u8, 0xFF);
        assert_eq!(v.port_read(VGA_MISC_OUTPUT_READ, 1) as u8, 0x67);
    }

    #[test]
    fn reset_restores_misc_output_default() {
        let mut v = VgaText::new();
        v.port_write(VGA_MISC_OUTPUT_WRITE, 1, 0x12);
        assert_eq!(v.misc_output, 0x12);
        v.reset();
        assert_eq!(v.misc_output, VGA_MISC_OUTPUT_DEFAULT);
        assert_eq!(
            v.port_read(VGA_MISC_OUTPUT_READ, 1) as u8,
            VGA_MISC_OUTPUT_DEFAULT
        );
    }

    #[test]
    fn sequencer_owns_ports() {
        // Spec: FreeVGA / OSDev VGA Hardware / IBM VGA — Sequencer Address
        // `0x3C4`, Data `0x3C5`.
        let v = VgaText::new();
        assert!(v.owns_port(VGA_SEQ_INDEX));
        assert!(v.owns_port(VGA_SEQ_DATA));
    }

    #[test]
    fn graphics_controller_owns_ports() {
        // Spec: FreeVGA / OSDev VGA Hardware / IBM VGA — GC Address `0x3CE`,
        // Data `0x3CF`.
        let v = VgaText::new();
        assert!(v.owns_port(VGA_GC_INDEX));
        assert!(v.owns_port(VGA_GC_DATA));
    }

    #[test]
    fn graphics_controller_index_data_round_trip() {
        // Spec: FreeVGA Graphics Registers — write index 0x3CE, data 0x3CF;
        // indexes 0x00–0x08 (Set/Reset … Bit Mask).
        let mut v = VgaText::new();
        v.port_write(VGA_GC_INDEX, 1, 0x05); // Graphics Mode
        v.port_write(VGA_GC_DATA, 1, 0x00);
        v.port_write(VGA_GC_INDEX, 1, 0x08); // Bit Mask
        v.port_write(VGA_GC_DATA, 1, 0xAA);
        assert_eq!(v.gc_regs[0x05], 0x00);
        assert_eq!(v.gc_regs[0x08], 0xAA);
        assert_eq!(v.port_read(VGA_GC_INDEX, 1) as u8, 0x08);
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, 0xAA);
        v.port_write(VGA_GC_INDEX, 1, 0x05);
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, 0x00);
    }

    #[test]
    fn graphics_controller_word_write_index_and_data() {
        // Mirror CRTC/Sequencer: 16-bit write to address port (lo=index, hi=data).
        let mut v = VgaText::new();
        v.port_write(VGA_GC_INDEX, 2, 0x0F_06);
        assert_eq!(v.gc_index, 0x06);
        assert_eq!(v.gc_regs[0x06], 0x0F);
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, 0x0F);
    }

    #[test]
    fn graphics_controller_out_of_range_index_ignored_on_data() {
        let mut v = VgaText::new();
        v.port_write(VGA_GC_INDEX, 1, 0x09);
        v.port_write(VGA_GC_DATA, 1, 0x55);
        // Index 0x09 is beyond the 9 standard registers; data write ignored,
        // read returns 0 (same CRTC/Sequencer out-of-range policy).
        assert_eq!(v.gc_regs, VGA_GC_DEFAULTS);
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, 0);
        v.port_write(VGA_GC_INDEX, 1, 0x05);
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, 0x10);
        v.port_write(VGA_GC_INDEX, 1, 0x08);
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, 0xFF);
    }

    #[test]
    fn graphics_controller_reset_defaults_mode03h() {
        // Spec: FreeVGA / OSDev / IBM VGA mode-03h-class GC programming SeaBIOS
        // probes — Mode `0x10`, Misc `0x0E`, Bit Mask `0xFF` (store/readback only).
        let v = VgaText::new();
        assert_eq!(v.gc_index, 0);
        assert_eq!(v.gc_regs, VGA_GC_DEFAULTS);
        assert_eq!(
            v.gc_regs,
            [0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x0E, 0x00, 0xFF]
        );
    }

    #[test]
    fn reset_restores_graphics_controller_defaults() {
        let mut v = VgaText::new();
        v.port_write(VGA_GC_INDEX, 1, 0x05);
        v.port_write(VGA_GC_DATA, 1, 0x40);
        v.port_write(VGA_GC_INDEX, 1, 0x08);
        v.port_write(VGA_GC_DATA, 1, 0x00);
        v.reset();
        assert_eq!(v.gc_index, 0);
        assert_eq!(v.gc_regs, VGA_GC_DEFAULTS);
        v.port_write(VGA_GC_INDEX, 1, 0x05);
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, 0x10);
        v.port_write(VGA_GC_INDEX, 1, 0x08);
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, 0xFF);
    }

    #[test]
    fn sequencer_index_data_round_trip() {
        // Spec: FreeVGA Sequencer Registers — write index 0x3C4, data 0x3C5;
        // indexes 0x00–0x04 (Reset, Clocking Mode, Map Mask, Character Map,
        // Memory Mode).
        let mut v = VgaText::new();
        v.port_write(VGA_SEQ_INDEX, 1, 0x02); // Map Mask
        v.port_write(VGA_SEQ_DATA, 1, 0x0F);
        v.port_write(VGA_SEQ_INDEX, 1, 0x04); // Memory Mode
        v.port_write(VGA_SEQ_DATA, 1, 0x06);
        assert_eq!(v.seq_regs[0x02], 0x0F);
        assert_eq!(v.seq_regs[0x04], 0x06);
        assert_eq!(v.port_read(VGA_SEQ_INDEX, 1) as u8, 0x04);
        assert_eq!(v.port_read(VGA_SEQ_DATA, 1) as u8, 0x06);
        v.port_write(VGA_SEQ_INDEX, 1, 0x02);
        assert_eq!(v.port_read(VGA_SEQ_DATA, 1) as u8, 0x0F);
    }

    #[test]
    fn sequencer_word_write_index_and_data() {
        // Mirror CRTC: 16-bit write to address port (lo=index, hi=data).
        let mut v = VgaText::new();
        v.port_write(VGA_SEQ_INDEX, 2, 0x0A_01);
        assert_eq!(v.seq_index, 0x01);
        assert_eq!(v.seq_regs[0x01], 0x0A);
        assert_eq!(v.port_read(VGA_SEQ_DATA, 1) as u8, 0x0A);
    }

    #[test]
    fn sequencer_out_of_range_index_ignored_on_data() {
        let mut v = VgaText::new();
        v.port_write(VGA_SEQ_INDEX, 1, 0x05);
        v.port_write(VGA_SEQ_DATA, 1, 0x55);
        // Index 0x05 is beyond the 5 standard registers; data write ignored,
        // read returns 0 (same CRTC out-of-range policy).
        assert_eq!(v.seq_regs, VGA_SEQ_DEFAULTS);
        assert_eq!(v.port_read(VGA_SEQ_DATA, 1) as u8, 0);
        // In-range defaults unchanged by the ignored write.
        v.port_write(VGA_SEQ_INDEX, 1, 0x00);
        assert_eq!(v.port_read(VGA_SEQ_DATA, 1) as u8, 0x03);
        v.port_write(VGA_SEQ_INDEX, 1, 0x02);
        assert_eq!(v.port_read(VGA_SEQ_DATA, 1) as u8, 0x03);
    }

    #[test]
    fn sequencer_reset_defaults_mode03h() {
        // Spec: FreeVGA / IBM VGA mode-03h-class Sequencer programming SeaBIOS
        // probes — Reset `0x03`, Clocking Mode `0x00`, Map Mask `0x03`,
        // Character Map Select `0x00`, Memory Mode `0x02` (store/readback only).
        let v = VgaText::new();
        assert_eq!(v.seq_index, 0);
        assert_eq!(v.seq_regs, VGA_SEQ_DEFAULTS);
        assert_eq!(v.seq_regs, [0x03, 0x00, 0x03, 0x00, 0x02]);
    }

    #[test]
    fn reset_restores_sequencer_defaults() {
        let mut v = VgaText::new();
        v.port_write(VGA_SEQ_INDEX, 1, 0x02);
        v.port_write(VGA_SEQ_DATA, 1, 0x0F);
        v.port_write(VGA_SEQ_INDEX, 1, 0x01);
        v.port_write(VGA_SEQ_DATA, 1, 0x01);
        v.reset();
        assert_eq!(v.seq_index, 0);
        assert_eq!(v.seq_regs, VGA_SEQ_DEFAULTS);
        v.port_write(VGA_SEQ_INDEX, 1, 0x02);
        assert_eq!(v.port_read(VGA_SEQ_DATA, 1) as u8, 0x03);
    }

    #[test]
    fn attribute_controller_owns_ports() {
        // Spec: FreeVGA Attribute Controller — Address/Data `0x3C0`, Data Read
        // `0x3C1`; Input Status #1 `0x3DA` resets the flip-flop (color map).
        let v = VgaText::new();
        assert!(v.misc_ioas_color());
        assert!(v.owns_port(VGA_ATC_ADDRESS_DATA));
        assert!(v.owns_port(VGA_ATC_DATA_READ));
        assert!(v.owns_port(VGA_INPUT_STATUS_1));
        assert!(!v.owns_port(VGA_INPUT_STATUS_1_MONO));
    }

    #[test]
    fn attribute_controller_flip_flop_index_data_round_trip() {
        // Spec: FreeVGA Accessing the Attribute Registers — read 0x3DA to reset
        // flip-flop; write index then data to 0x3C0; read data from 0x3C1.
        let mut v = VgaText::new();
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        assert!(!v.atc_flip_flop_data);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x10); // Mode Control index, PAS=0
        assert!(v.atc_flip_flop_data);
        assert_eq!(v.atc_index, 0x10);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x0C);
        assert!(!v.atc_flip_flop_data);
        assert_eq!(v.atc_regs[0x10], 0x0C);
        // Read path: reset → write index → read 0x3C1 (does not toggle flip-flop).
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x10);
        assert_eq!(v.port_read(VGA_ATC_DATA_READ, 1) as u8, 0x0C);
        assert!(v.atc_flip_flop_data); // still awaiting data after address write
        assert_eq!(v.port_read(VGA_ATC_ADDRESS_DATA, 1) as u8, 0x10);
    }

    #[test]
    fn attribute_controller_palette_and_mode_control_round_trip() {
        // Spec: FreeVGA Attribute Controller — palette indexes 0x00–0x0F and
        // Mode Control 0x10 are SeaBIOS-touchable ATC registers.
        let mut v = VgaText::new();
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x06); // palette brown entry
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x14);
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x12); // Color Plane Enable
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x0F);
        assert_eq!(v.atc_regs[0x06], 0x14);
        assert_eq!(v.atc_regs[0x12], 0x0F);
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x06);
        assert_eq!(v.port_read(VGA_ATC_DATA_READ, 1) as u8, 0x14);
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x12);
        assert_eq!(v.port_read(VGA_ATC_DATA_READ, 1) as u8, 0x0F);
    }

    #[test]
    fn attribute_controller_word_write_index_and_data() {
        // Consecutive lo/hi bytes through the flip-flop (index then data).
        let mut v = VgaText::new();
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 2, 0x0F_12);
        assert_eq!(v.atc_index, 0x12);
        assert_eq!(v.atc_regs[0x12], 0x0F);
        assert!(!v.atc_flip_flop_data);
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x12);
        assert_eq!(v.port_read(VGA_ATC_DATA_READ, 1) as u8, 0x0F);
    }

    #[test]
    fn attribute_controller_input_status1_resets_flip_flop() {
        // Spec: FreeVGA — mid-sequence read of 0x3DA forces address state so the
        // next 0x3C0 write is treated as index (not data).
        let mut v = VgaText::new();
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x10);
        assert!(v.atc_flip_flop_data);
        // Flip-flop reset is independent of the status-bit model; bit values vary.
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        assert!(!v.atc_flip_flop_data);
        // Without reset this would have been a data write to index 0x10.
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x11); // Overscan index
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x55);
        assert_eq!(v.atc_regs[0x11], 0x55);
        assert_eq!(v.atc_regs[0x10], 0x0C); // Mode Control default untouched
    }

    #[test]
    fn input_status1_vertical_retrace_and_display_disable_bits() {
        // Spec: FreeVGA External Registers / IBM VGA / OSDev VGA Hardware —
        // Input Status #1 (color `0x3DA`): bit0 = Display Disabled (inverted
        // display-enable; set during H or V retrace), bit3 = Vertical Retrace
        // (set during vertical retrace). Reading also resets the ATC flip-flop.
        let mut v = VgaText::new();
        assert_eq!(v.status1_phase, 0);

        let a = v.port_read(VGA_INPUT_STATUS_1, 1) as u8;
        assert_eq!(a & VGA_STATUS1_VR, 0, "phase0: not in vertical retrace");
        assert_eq!(a & VGA_STATUS1_DD, 0, "phase0: display enabled");
        assert!(!v.atc_flip_flop_data);

        let b = v.port_read(VGA_INPUT_STATUS_1, 1) as u8;
        assert_ne!(b & VGA_STATUS1_VR, 0, "phase1: vertical retrace");
        assert_ne!(b & VGA_STATUS1_DD, 0, "phase1: display disabled during VR");
        assert_eq!(
            b & (VGA_STATUS1_DD | VGA_STATUS1_VR),
            VGA_STATUS1_DD | VGA_STATUS1_VR
        );

        let c = v.port_read(VGA_INPUT_STATUS_1, 1) as u8;
        assert_eq!(c & VGA_STATUS1_VR, 0, "phase2: leaves vertical retrace");
        assert_eq!(c & VGA_STATUS1_DD, 0, "phase2: display enabled again");
    }

    #[test]
    fn input_status1_seabios_style_wait_for_retrace_terminates() {
        // Firmware commonly polls: wait until VR set, then wait until VR clear.
        // Deterministic read-phase model must make both waits terminate.
        let mut v = VgaText::new();
        let mut saw_vr = false;
        for _ in 0..8 {
            let s = v.port_read(VGA_INPUT_STATUS_1, 1) as u8;
            if s & VGA_STATUS1_VR != 0 {
                saw_vr = true;
                break;
            }
        }
        assert!(saw_vr, "must eventually observe Vertical Retrace");

        let mut left_vr = false;
        for _ in 0..8 {
            let s = v.port_read(VGA_INPUT_STATUS_1, 1) as u8;
            if s & VGA_STATUS1_VR == 0 {
                left_vr = true;
                break;
            }
        }
        assert!(left_vr, "must eventually leave Vertical Retrace");
    }

    #[test]
    fn input_status1_phase_advances_only_on_status_read() {
        let mut v = VgaText::new();
        let first = v.port_read(VGA_INPUT_STATUS_1, 1) as u8;
        // ATC / Misc traffic must not advance the status phase.
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x10);
        let _ = v.port_read(VGA_ATC_DATA_READ, 1);
        let _ = v.port_read(VGA_MISC_OUTPUT_READ, 1);
        assert_eq!(v.status1_phase, 1);
        let second = v.port_read(VGA_INPUT_STATUS_1, 1) as u8;
        assert_ne!(first & VGA_STATUS1_VR, second & VGA_STATUS1_VR);
    }

    #[test]
    fn input_status1_reset_clears_phase_and_keeps_flip_flop_reset() {
        let mut v = VgaText::new();
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1); // phase 0 → 1
        assert_eq!(v.status1_phase, 1);
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1); // phase 1 → 0 (wraps)
        assert_eq!(v.status1_phase, 0);
        // Advance again so reset has a non-zero phase to clear.
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        assert_eq!(v.status1_phase, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x10);
        assert!(v.atc_flip_flop_data);
        v.reset();
        assert_eq!(v.status1_phase, 0);
        assert!(!v.atc_flip_flop_data);
        let s = v.port_read(VGA_INPUT_STATUS_1, 1) as u8;
        assert_eq!(s & (VGA_STATUS1_DD | VGA_STATUS1_VR), 0);
        assert!(!v.atc_flip_flop_data);
    }

    #[test]
    fn input_status1_write_ignored() {
        let mut v = VgaText::new();
        v.port_write(VGA_INPUT_STATUS_1, 1, 0xFF);
        assert_eq!(v.status1_phase, 0);
        let s = v.port_read(VGA_INPUT_STATUS_1, 1) as u8;
        assert_eq!(s & (VGA_STATUS1_DD | VGA_STATUS1_VR), 0);
    }

    #[test]
    fn input_status1_mono_alias_when_misc_ioas_cleared() {
        // Spec: FreeVGA / IBM VGA Misc Output bit0 (IOAS) — clear selects mono
        // I/O map; Input Status #1 moves to `0x3BA` with the same DD/VR model
        // and ATC flip-flop reset. Color `0x3DA` is not owned.
        let mut v = VgaText::new();
        assert!(v.misc_ioas_color());
        assert!(v.owns_port(VGA_INPUT_STATUS_1));
        assert!(!v.owns_port(VGA_INPUT_STATUS_1_MONO));

        // Clear IOAS (keep other default-ish bits); mono status map.
        v.port_write(
            VGA_MISC_OUTPUT_WRITE,
            1,
            u32::from(VGA_MISC_OUTPUT_DEFAULT & !VGA_MISC_IOAS),
        );
        assert!(!v.misc_ioas_color());
        assert!(!v.owns_port(VGA_INPUT_STATUS_1));
        assert!(v.owns_port(VGA_INPUT_STATUS_1_MONO));

        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x10);
        assert!(v.atc_flip_flop_data);

        let a = v.port_read(VGA_INPUT_STATUS_1_MONO, 1) as u8;
        assert_eq!(a & VGA_STATUS1_VR, 0, "phase0: not in vertical retrace");
        assert_eq!(a & VGA_STATUS1_DD, 0, "phase0: display enabled");
        assert!(!v.atc_flip_flop_data);

        let b = v.port_read(VGA_INPUT_STATUS_1_MONO, 1) as u8;
        assert_eq!(
            b & (VGA_STATUS1_DD | VGA_STATUS1_VR),
            VGA_STATUS1_DD | VGA_STATUS1_VR
        );

        // Inactive color port must not advance phase or reset the flip-flop.
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x10);
        assert!(v.atc_flip_flop_data);
        let phase_before = v.status1_phase;
        assert_eq!(v.port_read(VGA_INPUT_STATUS_1, 1), 0xFFFF_FFFF);
        assert_eq!(v.status1_phase, phase_before);
        assert!(v.atc_flip_flop_data);
    }

    #[test]
    fn input_status1_color_port_when_misc_ioas_set() {
        // Spec: FreeVGA / IBM — IOAS=1 keeps Input Status #1 at `0x3DA`; mono
        // `0x3BA` is ignored / not owned.
        let mut v = VgaText::new();
        assert_eq!(v.misc_output & VGA_MISC_IOAS, VGA_MISC_IOAS);
        assert!(v.owns_port(VGA_INPUT_STATUS_1));
        assert!(!v.owns_port(VGA_INPUT_STATUS_1_MONO));

        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x10);
        assert!(v.atc_flip_flop_data);
        let phase_before = v.status1_phase;
        assert_eq!(v.port_read(VGA_INPUT_STATUS_1_MONO, 1), 0xFFFF_FFFF);
        assert_eq!(v.status1_phase, phase_before);
        assert!(v.atc_flip_flop_data);

        let s = v.port_read(VGA_INPUT_STATUS_1, 1) as u8;
        assert_eq!(s & (VGA_STATUS1_DD | VGA_STATUS1_VR), 0);
        assert!(!v.atc_flip_flop_data);
        assert_eq!(v.status1_phase, 1);
    }

    #[test]
    fn input_status1_ioas_switch_shares_phase_counter() {
        // Spec: FreeVGA External Registers — color/mono are aliases of one
        // Input Status #1; the phase model is shared across IOAS switches.
        let mut v = VgaText::new();
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1); // phase 0 → 1
        assert_eq!(v.status1_phase, 1);

        v.port_write(
            VGA_MISC_OUTPUT_WRITE,
            1,
            u32::from(VGA_MISC_OUTPUT_DEFAULT & !VGA_MISC_IOAS),
        );
        let s = v.port_read(VGA_INPUT_STATUS_1_MONO, 1) as u8;
        assert_eq!(
            s & (VGA_STATUS1_DD | VGA_STATUS1_VR),
            VGA_STATUS1_DD | VGA_STATUS1_VR
        );
        assert_eq!(v.status1_phase, 0);

        // Restore color map; next read continues the shared counter.
        v.port_write(VGA_MISC_OUTPUT_WRITE, 1, u32::from(VGA_MISC_OUTPUT_DEFAULT));
        let s = v.port_read(VGA_INPUT_STATUS_1, 1) as u8;
        assert_eq!(s & (VGA_STATUS1_DD | VGA_STATUS1_VR), 0);
        assert_eq!(v.status1_phase, 1);
    }

    #[test]
    fn input_status1_mono_write_ignored() {
        let mut v = VgaText::new();
        v.port_write(
            VGA_MISC_OUTPUT_WRITE,
            1,
            u32::from(VGA_MISC_OUTPUT_DEFAULT & !VGA_MISC_IOAS),
        );
        v.port_write(VGA_INPUT_STATUS_1_MONO, 1, 0xFF);
        assert_eq!(v.status1_phase, 0);
        let s = v.port_read(VGA_INPUT_STATUS_1_MONO, 1) as u8;
        assert_eq!(s & (VGA_STATUS1_DD | VGA_STATUS1_VR), 0);
    }

    #[test]
    fn attribute_controller_pas_bit_stored_in_address() {
        // Spec: FreeVGA Attribute Address — bit5 PAS; bits4:0 select register.
        let mut v = VgaText::new();
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, u32::from(VGA_ATC_PAS | 0x10));
        assert_eq!(v.atc_index, VGA_ATC_PAS | 0x10);
        assert_eq!(
            v.port_read(VGA_ATC_ADDRESS_DATA, 1) as u8,
            VGA_ATC_PAS | 0x10
        );
        assert_eq!(v.port_read(VGA_ATC_DATA_READ, 1) as u8, 0x0C);
        // Finishing the data write leaves PAS in the address register.
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x08);
        assert_eq!(v.atc_regs[0x10], 0x08);
        assert_eq!(
            v.port_read(VGA_ATC_ADDRESS_DATA, 1) as u8,
            VGA_ATC_PAS | 0x10
        );
    }

    #[test]
    fn attribute_controller_out_of_range_index_ignored_on_data() {
        let mut v = VgaText::new();
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x15); // beyond 0x00–0x14
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x55);
        assert_eq!(v.atc_regs, VGA_ATC_DEFAULTS);
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x15);
        assert_eq!(v.port_read(VGA_ATC_DATA_READ, 1) as u8, 0);
    }

    #[test]
    fn attribute_controller_reset_defaults_mode03h() {
        // Spec: FreeVGA / IBM VGA / Abrash mode-03h-class ATC — palette
        // 00..05/14/07/38..3F, Mode Control 0x0C, plane enable 0x0F, pan 0x08.
        let v = VgaText::new();
        assert_eq!(v.atc_index, VGA_ATC_INDEX_DEFAULT);
        assert!(!v.atc_flip_flop_data);
        assert_eq!(v.atc_regs, VGA_ATC_DEFAULTS);
        assert_eq!(v.atc_regs[0x06], 0x14);
        assert_eq!(v.atc_regs[0x10], 0x0C);
        assert_eq!(v.atc_regs[0x12], 0x0F);
        assert_eq!(v.atc_regs[0x13], 0x08);
    }

    #[test]
    fn reset_restores_attribute_controller_defaults() {
        let mut v = VgaText::new();
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x10);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x00);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x06);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x06);
        assert_eq!(v.atc_regs[0x10], 0x00);
        assert_eq!(v.atc_regs[0x06], 0x06);
        v.reset();
        assert_eq!(v.atc_index, VGA_ATC_INDEX_DEFAULT);
        assert!(!v.atc_flip_flop_data);
        assert_eq!(v.atc_regs, VGA_ATC_DEFAULTS);
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x10);
        assert_eq!(v.port_read(VGA_ATC_DATA_READ, 1) as u8, 0x0C);
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x06);
        assert_eq!(v.port_read(VGA_ATC_DATA_READ, 1) as u8, 0x14);
    }
}
