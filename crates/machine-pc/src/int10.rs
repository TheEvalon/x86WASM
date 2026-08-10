//! Host INT 10h video stub (AH=00h set mode / AH=0Eh teletype).
//!
//! Bring-up path only: installs a real-mode IVT pointer and services selected
//! functions via [`Machine::service_int10`]. Not a VGA BIOS and not VBE
//! AX=4Fxxh.
//!
//! Spec: Ralf Brown's Interrupt List — INT 10h AH=00h "SET VIDEO MODE",
//! AH=0Eh "TELETYPE OUTPUT"; IBM PC BIOS Data Area video fields at
//! `0040:0049` / `0040:004A` / `0040:0050` / `0040:0062`.

use crate::{Machine, MachineError};
use devices::{VgaRenderMode, VGA_DEFAULT_ATTR, VGA_TEXT_COLS, VGA_TEXT_ROWS};
use x86_core::CpuState;

/// INT 10h vector number. Spec: IBM PC IVT / RBIL.
pub const INT10_VECTOR: u8 = 0x10;

/// AH=00h — SET VIDEO MODE. Spec: RBIL INT 10h AH=00h.
pub const INT10_AH_SET_MODE: u8 = 0x00;
/// AH=0Eh — TELETYPE OUTPUT. Spec: RBIL INT 10h AH=0Eh.
pub const INT10_AH_TELETYPE: u8 = 0x0E;

/// BIOS mode 03h — 80×25 color text. Spec: IBM VGA / RBIL.
pub const INT10_MODE_03H_TEXT: u8 = 0x03;
/// BIOS mode 13h — 320×200×256. Spec: IBM VGA / RBIL.
pub const INT10_MODE_13H_GRAPHICS: u8 = 0x13;

/// BDA current video mode (`0040:0049`). Spec: RBIL memory map.
pub const BDA_VIDEO_MODE: u64 = 0x449;
/// BDA screen columns (`0040:004A`). Spec: RBIL memory map.
pub const BDA_VIDEO_COLS: u64 = 0x44A;
/// BDA cursor position for display page 0 (`0040:0050`): low=col, high=row.
pub const BDA_CURSOR_PAGE0: u64 = 0x450;
/// BDA active display page (`0040:0062`). Spec: RBIL memory map.
pub const BDA_ACTIVE_PAGE: u64 = 0x462;

/// Default teletype attribute in text mode (light grey on black).
const INT10_TTY_ATTR: u8 = VGA_DEFAULT_ATTR;

impl Machine {
    /// Dispatch host INT 10h for the AH currently in `CPU.AH`.
    ///
    /// Supported:
    /// - AH=00h AL=03h / AL=13h — set video mode (text reset / mode 13h program)
    /// - AH=0Eh — teletype output in text mode (CR/LF/BS + printable)
    ///
    /// Unsupported AH values leave CPU/VGA unchanged. Spec: RBIL INT 10h subset.
    pub fn service_int10(&mut self) {
        match self.cpu.ah() {
            INT10_AH_SET_MODE => self.int10_set_mode(self.cpu.al()),
            INT10_AH_TELETYPE => self.int10_teletype(self.cpu.al()),
            _ => {}
        }
    }

    /// Install a real-mode IVT entry for vector `0x10`.
    ///
    /// Does **not** install a BIOS body — only the far pointer. Host harnesses
    /// call [`Self::service_int10`] explicitly (or use SeaVGABIOS later).
    /// Spec: IBM PC IVT — `0x10 * 4` holds `offset:segment`.
    pub fn install_int10_ivt_pointer(
        &mut self,
        handler_seg: u16,
        handler_off: u16,
    ) -> Result<(), MachineError> {
        let base = u64::from(INT10_VECTOR) * 4;
        self.mem
            .write_u8(base, (handler_off & 0xFF) as u8)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        self.mem
            .write_u8(base + 1, (handler_off >> 8) as u8)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        self.mem
            .write_u8(base + 2, (handler_seg & 0xFF) as u8)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        self.mem
            .write_u8(base + 3, (handler_seg >> 8) as u8)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        Ok(())
    }

    fn int10_set_mode(&mut self, mode: u8) {
        match mode {
            INT10_MODE_03H_TEXT => {
                self.vga.reset();
                self.write_bda_video(mode, VGA_TEXT_COLS as u16, 0, 0);
            }
            INT10_MODE_13H_GRAPHICS => {
                self.vga.program_bios_mode13h();
                // Mode 13h is 40 columns in the classic BDA sense (320/8).
                self.write_bda_video(mode, 40, 0, 0);
            }
            _ => {
                // Unsupported mode: leave hardware alone (honest subset).
            }
        }
    }

    fn int10_teletype(&mut self, ch: u8) {
        // Graphics teletype and multi-page are out of scope; only text mode.
        if self.vga.render_mode() != VgaRenderMode::Text {
            return;
        }
        let page = self.read_bda_u8(BDA_ACTIVE_PAGE).unwrap_or(0);
        if page != 0 {
            // Only page 0 is modeled in the text helpers.
            return;
        }
        let (mut row, mut col) = self.read_bda_cursor().unwrap_or((0, 0));
        match ch {
            0x0D => {
                // CR — column 0. Spec: RBIL teletype / classic BIOS.
                col = 0;
            }
            0x0A => {
                // LF — next row, wrap at bottom without scroll (bounded stub).
                if row + 1 < VGA_TEXT_ROWS as u8 {
                    row += 1;
                }
            }
            0x08 => {
                // BS — move left; do not erase (minimal TTY).
                col = col.saturating_sub(1);
            }
            0x07 => {
                // Bell — no speaker path in this stub.
            }
            _ => {
                let attr = self
                    .vga
                    .attr_at(usize::from(row), usize::from(col))
                    .unwrap_or(INT10_TTY_ATTR);
                let _ = self
                    .vga
                    .put_char(usize::from(row), usize::from(col), ch, attr);
                col += 1;
                if col as usize >= VGA_TEXT_COLS {
                    col = 0;
                    if row + 1 < VGA_TEXT_ROWS as u8 {
                        row += 1;
                    }
                }
            }
        }
        let _ = self.write_bda_cursor(row, col);
    }

    fn write_bda_video(&mut self, mode: u8, cols: u16, row: u8, col: u8) {
        let _ = self.mem.write_u8(BDA_VIDEO_MODE, mode);
        let _ = self.mem.write_u8(BDA_VIDEO_COLS, (cols & 0xFF) as u8);
        let _ = self.mem.write_u8(BDA_VIDEO_COLS + 1, (cols >> 8) as u8);
        let _ = self.mem.write_u8(BDA_ACTIVE_PAGE, 0);
        let _ = self.write_bda_cursor(row, col);
    }

    fn write_bda_cursor(&mut self, row: u8, col: u8) -> Result<(), MachineError> {
        self.mem
            .write_u8(BDA_CURSOR_PAGE0, col)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        self.mem
            .write_u8(BDA_CURSOR_PAGE0 + 1, row)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        Ok(())
    }

    fn read_bda_cursor(&self) -> Option<(u8, u8)> {
        let col = self.read_bda_u8(BDA_CURSOR_PAGE0)?;
        let row = self.read_bda_u8(BDA_CURSOR_PAGE0 + 1)?;
        Some((row, col))
    }

    fn read_bda_u8(&self, phys: u64) -> Option<u8> {
        self.mem.read_u8(phys).ok()
    }
}

/// Load AH/AL for a host INT 10h call in tests/harnesses.
pub fn setup_int10_set_mode(cpu: &mut CpuState, mode: u8) {
    cpu.set_ah(INT10_AH_SET_MODE);
    cpu.set_al(mode);
}

/// Load AH/AL for teletype output.
pub fn setup_int10_teletype(cpu: &mut CpuState, ch: u8) {
    cpu.set_ah(INT10_AH_TELETYPE);
    cpu.set_al(ch);
}

#[cfg(test)]
mod tests {
    use super::*;
    use devices::VgaRenderMode;

    #[test]
    fn int10_ah00_mode03_resets_text() {
        let mut m = Machine::new(1024 * 1024);
        m.vga.program_bios_mode13h();
        assert!(m.vga.is_mode13h_programming());

        setup_int10_set_mode(&mut m.cpu, INT10_MODE_03H_TEXT);
        m.service_int10();

        assert_eq!(m.vga.render_mode(), VgaRenderMode::Text);
        assert!(!m.vga.is_mode13h_programming());
        assert_eq!(m.mem.read_u8(BDA_VIDEO_MODE).unwrap(), INT10_MODE_03H_TEXT);
        assert_eq!(m.mem.read_u8(BDA_VIDEO_COLS).unwrap(), VGA_TEXT_COLS as u8);
        assert_eq!(m.mem.read_u8(BDA_CURSOR_PAGE0).unwrap(), 0);
        assert_eq!(m.mem.read_u8(BDA_CURSOR_PAGE0 + 1).unwrap(), 0);
    }

    #[test]
    fn int10_ah00_mode13_programs_chain4() {
        let mut m = Machine::new(1024 * 1024);
        setup_int10_set_mode(&mut m.cpu, INT10_MODE_13H_GRAPHICS);
        m.service_int10();
        assert!(m.vga.is_mode13h_programming());
        assert_eq!(m.vga.render_mode(), VgaRenderMode::Graphics256Chain4);
        assert_eq!(
            m.mem.read_u8(BDA_VIDEO_MODE).unwrap(),
            INT10_MODE_13H_GRAPHICS
        );
    }

    #[test]
    fn int10_ah0e_teletype_writes_and_advances_cursor() {
        let mut m = Machine::new(1024 * 1024);
        setup_int10_set_mode(&mut m.cpu, INT10_MODE_03H_TEXT);
        m.service_int10();

        setup_int10_teletype(&mut m.cpu, b'H');
        m.service_int10();
        setup_int10_teletype(&mut m.cpu, b'i');
        m.service_int10();

        assert_eq!(m.vga.char_at(0, 0), Some(b'H'));
        assert_eq!(m.vga.char_at(0, 1), Some(b'i'));
        assert_eq!(m.mem.read_u8(BDA_CURSOR_PAGE0).unwrap(), 2);
        assert_eq!(m.mem.read_u8(BDA_CURSOR_PAGE0 + 1).unwrap(), 0);
    }

    #[test]
    fn int10_ah0e_handles_cr_lf() {
        let mut m = Machine::new(1024 * 1024);
        setup_int10_set_mode(&mut m.cpu, INT10_MODE_03H_TEXT);
        m.service_int10();
        setup_int10_teletype(&mut m.cpu, b'A');
        m.service_int10();
        setup_int10_teletype(&mut m.cpu, 0x0D);
        m.service_int10();
        setup_int10_teletype(&mut m.cpu, 0x0A);
        m.service_int10();
        setup_int10_teletype(&mut m.cpu, b'B');
        m.service_int10();

        assert_eq!(m.vga.char_at(0, 0), Some(b'A'));
        assert_eq!(m.vga.char_at(1, 0), Some(b'B'));
        assert_eq!(m.mem.read_u8(BDA_CURSOR_PAGE0).unwrap(), 1);
        assert_eq!(m.mem.read_u8(BDA_CURSOR_PAGE0 + 1).unwrap(), 1);
    }

    #[test]
    fn int10_ivt_pointer_install() {
        let mut m = Machine::new(64 * 1024);
        m.install_int10_ivt_pointer(0xF000, 0xF065).unwrap();
        assert_eq!(m.mem.read_u8(0x40).unwrap(), 0x65);
        assert_eq!(m.mem.read_u8(0x41).unwrap(), 0xF0);
        assert_eq!(m.mem.read_u8(0x42).unwrap(), 0x00);
        assert_eq!(m.mem.read_u8(0x43).unwrap(), 0xF0);
    }

    #[test]
    fn int10_unsupported_ah_is_noop() {
        let mut m = Machine::new(1024 * 1024);
        m.vga.put_char(0, 0, b'X', 0x07);
        m.cpu.set_ah(0x0F); // GET CURRENT VIDEO MODE — not implemented
        m.cpu.set_al(0);
        m.service_int10();
        assert_eq!(m.vga.char_at(0, 0), Some(b'X'));
    }
}
