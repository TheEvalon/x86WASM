//! VGA color text-mode frame buffer MMIO stub (physical `0xB8000`).
//!
//! # Spec refs
//!
//! - IBM VGA / classic PC: color text frame buffer at physical `0xB8000`,
//!   80×25 cells, 2 bytes per cell (ASCII character, attribute).
//! - OSDev Text UI — memory layout for mode 03h text (char at even offset,
//!   attribute at odd); window commonly treated as 32 KiB (`0xB8000`–`0xBFFFF`).
//! - `docs/machine-model-pc-v1.md`, `plan.md` §15.6 / §21 VGA text mode.
//!
//! # Scope (this slice)
//!
//! - 32 KiB text plane buffer at `VGA_TEXT_BASE`…`VGA_TEXT_END`
//! - Byte R/W; reset fills first 80×25 with space + attribute `0x07`
//! - Helpers for tests (`char_at` / `attr_at` / `put_char`)
//!
//! # Unsupported (explicit)
//!
//! - CRTC / sequencer / graphics / attribute controller port programming
//!   (`0x3C0`–`0x3DF`, etc.)
//! - Planar graphics, VBE, host canvas rendering, dirty tracking
//! - Font ROM, hardware cursor position registers

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

/// Color text-mode frame buffer stub.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VgaText {
    /// Raw plane bytes (char/attr interleaved).
    pub mem: Vec<u8>,
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
        };
        v.reset();
        v
    }

    /// Reset text plane: 80×25 → space/`0x07`; remainder cleared.
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
    }

    /// True if `addr` (after A20) falls in the text plane.
    pub fn owns_addr(addr: u64) -> bool {
        (VGA_TEXT_BASE..VGA_TEXT_END).contains(&addr)
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
}
