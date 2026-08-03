//! VGA color text-mode frame buffer MMIO stub (physical `0xB8000`) plus CRTC
//! index/data port stub (`0x3D4`/`0x3D5`).
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
//! - `docs/machine-model-pc-v1.md`, `plan.md` §15.6 / §21 VGA text mode.
//!
//! # Scope (this slice)
//!
//! - 32 KiB text plane buffer at `VGA_TEXT_BASE`…`VGA_TEXT_END`
//! - Byte R/W; reset fills first 80×25 with space + attribute `0x07`
//! - Helpers for tests (`char_at` / `attr_at` / `put_char`)
//! - CRTC index/data noop: latch index on `0x3D4`, store/read register file on
//!   `0x3D5` (no timing, cursor render, or protect-bit enforcement)
//!
//! # Unsupported (explicit)
//!
//! - Sequencer / graphics / attribute controller port programming
//! - CRTC protect bit (index `0x11` bit7), mono map at `0x3B4`/`0x3B5`
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

/// Color text-mode frame buffer + CRTC index/data stub.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VgaText {
    /// Raw plane bytes (char/attr interleaved).
    pub mem: Vec<u8>,
    /// Latched CRTC index (written via `0x3D4`).
    pub crtc_index: u8,
    /// CRTC register file (noop store/readback).
    pub crtc_regs: [u8; VGA_CRTC_REG_COUNT],
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
        };
        v.reset();
        v
    }

    /// Reset text plane: 80×25 → space/`0x07`; remainder cleared; CRTC cleared.
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
    }

    /// True if `addr` (after A20) falls in the text plane.
    pub fn owns_addr(addr: u64) -> bool {
        (VGA_TEXT_BASE..VGA_TEXT_END).contains(&addr)
    }

    /// True if this device owns the I/O port (color CRTC pair only).
    pub fn owns_port(port: u16) -> bool {
        matches!(port, VGA_CRTC_INDEX | VGA_CRTC_DATA)
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
}

impl PortDevice for VgaText {
    fn port_read(&mut self, port: u16, _size: u8) -> u32 {
        match port {
            VGA_CRTC_INDEX => u32::from(self.read_crtc_index()),
            VGA_CRTC_DATA => u32::from(self.read_crtc_data()),
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
        assert!(VgaText::owns_port(VGA_CRTC_INDEX));
        assert!(VgaText::owns_port(VGA_CRTC_DATA));
        assert!(!VgaText::owns_port(0x3B4));
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
}
