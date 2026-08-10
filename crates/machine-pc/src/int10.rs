//! Host INT 10h video stub (AH=00h/01h/02h/03h/09h/0Ah/0Eh/0Fh).
//!
//! Bring-up path only: installs a real-mode IVT pointer and services selected
//! functions via [`Machine::service_int10`]. Not a VGA BIOS and not VBE
//! AX=4Fxxh.
//!
//! Spec: Ralf Brown's Interrupt List — INT 10h AH=00h "SET VIDEO MODE",
//! AH=01h "SET CURSOR TYPE", AH=02h "SET CURSOR POSITION", AH=03h "GET CURSOR
//! POSITION AND SIZE", AH=09h "WRITE CHARACTER AND ATTRIBUTE AT CURSOR
//! POSITION", AH=0Ah "WRITE CHARACTER ONLY AT CURSOR POSITION", AH=0Eh
//! "TELETYPE OUTPUT", AH=0Fh "GET CURRENT VIDEO MODE"; IBM PC BIOS Data Area
//! video fields at `0040:0049` / `0040:004A` / `0040:004C` / `0040:004E` /
//! `0040:0050` / `0040:0060` / `0040:0062`.

use crate::{Machine, MachineError};
use devices::{
    PortDevice, VgaRenderMode, VGA_CRTC_CURSOR_END, VGA_CRTC_CURSOR_START, VGA_CRTC_DATA,
    VGA_CRTC_INDEX, VGA_DEFAULT_ATTR, VGA_TEXT_COLS, VGA_TEXT_ROWS,
};
use x86_core::CpuState;

/// INT 10h vector number. Spec: IBM PC IVT / RBIL.
pub const INT10_VECTOR: u8 = 0x10;

/// AH=00h — SET VIDEO MODE. Spec: RBIL INT 10h AH=00h.
pub const INT10_AH_SET_MODE: u8 = 0x00;
/// AH=01h — SET CURSOR TYPE. Spec: RBIL INT 10h AH=01h.
pub const INT10_AH_SET_CURSOR_TYPE: u8 = 0x01;
/// AH=02h — SET CURSOR POSITION. Spec: RBIL INT 10h AH=02h.
pub const INT10_AH_SET_CURSOR: u8 = 0x02;
/// AH=03h — GET CURSOR POSITION AND SIZE. Spec: RBIL INT 10h AH=03h.
pub const INT10_AH_GET_CURSOR: u8 = 0x03;
/// AH=09h — WRITE CHARACTER AND ATTRIBUTE. Spec: RBIL INT 10h AH=09h.
pub const INT10_AH_WRITE_CHAR_ATTR: u8 = 0x09;
/// AH=0Ah — WRITE CHARACTER ONLY. Spec: RBIL INT 10h AH=0Ah.
pub const INT10_AH_WRITE_CHAR: u8 = 0x0A;
/// AH=0Eh — TELETYPE OUTPUT. Spec: RBIL INT 10h AH=0Eh.
pub const INT10_AH_TELETYPE: u8 = 0x0E;
/// AH=0Fh — GET CURRENT VIDEO MODE. Spec: RBIL INT 10h AH=0Fh.
pub const INT10_AH_GET_MODE: u8 = 0x0F;

/// BIOS mode 03h — 80×25 color text. Spec: IBM VGA / RBIL.
pub const INT10_MODE_03H_TEXT: u8 = 0x03;
/// BIOS mode 13h — 320×200×256. Spec: IBM VGA / RBIL.
pub const INT10_MODE_13H_GRAPHICS: u8 = 0x13;

/// BDA current video mode (`0040:0049`). Spec: RBIL memory map.
pub const BDA_VIDEO_MODE: u64 = 0x449;
/// BDA screen columns (`0040:004A`). Spec: RBIL memory map.
pub const BDA_VIDEO_COLS: u64 = 0x44A;
/// BDA video page / regen buffer size in bytes (`0040:004C`). Spec: RBIL.
pub const BDA_VIDEO_PAGE_SIZE: u64 = 0x44C;
/// BDA current page start offset (`0040:004E`). Spec: RBIL — stub keeps `0000h`.
pub const BDA_VIDEO_PAGE_START: u64 = 0x44E;
/// BDA cursor position for display page 0 (`0040:0050`): low=col, high=row.
pub const BDA_CURSOR_PAGE0: u64 = 0x450;
/// BDA cursor type (`0040:0060`): low=end scanline, high=start scanline (CX form).
/// Spec: RBIL memory map / INT 10h AH=03h CH=start CL=end.
pub const BDA_CURSOR_TYPE: u64 = 0x460;
/// BDA active display page (`0040:0062`). Spec: RBIL memory map.
pub const BDA_ACTIVE_PAGE: u64 = 0x462;

/// Mode-03h default cursor start scanline (IBM VGA underline-ish). Spec: RBIL /
/// classic BIOS mode 03h cursor type `0607h`.
pub const INT10_MODE03_CURSOR_START: u8 = 0x06;
/// Mode-03h default cursor end scanline.
pub const INT10_MODE03_CURSOR_END: u8 = 0x07;
/// Mode-03h BDA page size (4 KiB regen). Spec: classic IBM BIOS BDA `0040:004C`.
pub const INT10_MODE03_PAGE_SIZE: u16 = 0x1000;
/// Mode-13h BDA page size. Spec: classic BIOS mode-13h BDA `0040:004C` (`FA00h`).
pub const INT10_MODE13_PAGE_SIZE: u16 = 0xFA00;

/// Bound CX repeat count for AH=09h/0Ah so a pathological count cannot walk
/// forever. One full 80×25 page is enough for host bring-up.
pub const INT10_WRITE_CHAR_MAX_COUNT: u16 = (VGA_TEXT_COLS * VGA_TEXT_ROWS) as u16;

/// Default teletype attribute in text mode (light grey on black).
const INT10_TTY_ATTR: u8 = VGA_DEFAULT_ATTR;

impl Machine {
    /// Dispatch host INT 10h for the AH currently in `CPU.AH`.
    ///
    /// Supported:
    /// - AH=00h AL=03h / AL=13h — set video mode (text reset / mode 13h program)
    /// - AH=01h — set cursor type (BDA `0040:0060` + CRTC Cursor Start/End)
    /// - AH=02h — set cursor position (page 0; BDA `0040:0050`)
    /// - AH=03h — get cursor position and size (page 0; BDA + CRTC scanlines)
    /// - AH=09h — write character+attribute at cursor (page 0 text; no advance)
    /// - AH=0Ah — write character only at cursor (page 0 text; no advance)
    /// - AH=0Eh — teletype output in text mode (CR/LF/BS + printable)
    /// - AH=0Fh — get current video mode / columns / page from BDA
    ///
    /// Unsupported AH values leave CPU/VGA unchanged. Spec: RBIL INT 10h subset.
    pub fn service_int10(&mut self) {
        match self.cpu.ah() {
            INT10_AH_SET_MODE => self.int10_set_mode(self.cpu.al()),
            INT10_AH_SET_CURSOR_TYPE => self.int10_set_cursor_type(),
            INT10_AH_SET_CURSOR => self.int10_set_cursor(),
            INT10_AH_GET_CURSOR => self.int10_get_cursor(),
            INT10_AH_WRITE_CHAR_ATTR => self.int10_write_char(true),
            INT10_AH_WRITE_CHAR => self.int10_write_char(false),
            INT10_AH_TELETYPE => self.int10_teletype(self.cpu.al()),
            INT10_AH_GET_MODE => self.int10_get_mode(),
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
                self.write_bda_video(mode, VGA_TEXT_COLS as u16, INT10_MODE03_PAGE_SIZE, 0, 0);
                let _ =
                    self.write_bda_cursor_type(INT10_MODE03_CURSOR_START, INT10_MODE03_CURSOR_END);
            }
            INT10_MODE_13H_GRAPHICS => {
                self.vga.program_bios_mode13h();
                // Mode 13h is 40 columns in the classic BDA sense (320/8).
                self.write_bda_video(mode, 40, INT10_MODE13_PAGE_SIZE, 0, 0);
            }
            _ => {
                // Unsupported mode: leave hardware alone (honest subset).
            }
        }
    }

    /// AH=01h SET CURSOR TYPE. Spec: RBIL — CH=start scanline, CL=end scanline
    /// (CH bit5 = cursor disable on VGA).
    ///
    /// Writes BDA `0040:0060` (CX layout: low=end, high=start) and programs
    /// FreeVGA CRTC Cursor Start (`0x0A`) / Cursor End (`0x0B`).
    fn int10_set_cursor_type(&mut self) {
        let start = self.cpu.gpr_u8(4 + CpuState::RCX); // CH
        let end = self.cpu.gpr_u8_low(CpuState::RCX); // CL
        let _ = self.write_bda_cursor_type(start, end);
        self.program_crtc_cursor_type(start, end);
    }

    /// Program CRTC Cursor Start/End to match AH=01h / BDA cursor type.
    ///
    /// Spec: FreeVGA CRT Controller — indices `0x0A` / `0x0B` (scanline mask
    /// and disable bit preserved in the written byte).
    fn program_crtc_cursor_type(&mut self, start: u8, end: u8) {
        self.vga
            .port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_CURSOR_START));
        self.vga.port_write(VGA_CRTC_DATA, 1, u32::from(start));
        self.vga
            .port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_CURSOR_END));
        self.vga.port_write(VGA_CRTC_DATA, 1, u32::from(end));
    }

    /// AH=02h SET CURSOR POSITION. Spec: RBIL — BH=page, DH=row, DL=col.
    ///
    /// Page 0 only: writes BDA `0040:0050`. Full CRTC Location High/Low sync is
    /// out of scope (prefer BDA + existing VGA helpers for reads).
    fn int10_set_cursor(&mut self) {
        let page = self.cpu.gpr_u8(4 + CpuState::RBX); // BH
        if page != 0 {
            return;
        }
        let row = self.cpu.gpr_u8(4 + CpuState::RDX); // DH
        let col = self.cpu.gpr_u8_low(CpuState::RDX); // DL
        let cols = self.read_bda_cols().unwrap_or(VGA_TEXT_COLS as u16);
        let max_col = cols.saturating_sub(1).min(u16::from(u8::MAX)) as u8;
        let max_row = (VGA_TEXT_ROWS as u8).saturating_sub(1);
        let col = col.min(max_col);
        let row = row.min(max_row);
        let _ = self.write_bda_cursor(row, col);
    }

    /// AH=03h GET CURSOR POSITION AND SIZE. Spec: RBIL — BH=page in;
    /// DH=row, DL=col, CH=start, CL=end out.
    fn int10_get_cursor(&mut self) {
        let page = self.cpu.gpr_u8(4 + CpuState::RBX); // BH
        if page != 0 {
            return;
        }
        let (row, col) = self.read_bda_cursor().unwrap_or((0, 0));
        let (start, end) = self.read_bda_cursor_type().unwrap_or_else(|| {
            (
                self.vga.crtc_cursor_start_scanline(),
                self.vga.crtc_cursor_end_scanline(),
            )
        });
        self.cpu.set_gpr_u8(4 + CpuState::RDX, row); // DH
        self.cpu.set_gpr_u8_low(CpuState::RDX, col); // DL
        self.cpu.set_gpr_u8(4 + CpuState::RCX, start); // CH
        self.cpu.set_gpr_u8_low(CpuState::RCX, end); // CL
    }

    /// AH=0Fh GET CURRENT VIDEO MODE. Spec: RBIL — AL=mode, AH=columns, BH=page.
    ///
    /// Reads BDA `0040:0049` / `0040:004A` / `0040:0062` (host stub state).
    fn int10_get_mode(&mut self) {
        let mode = self
            .read_bda_u8(BDA_VIDEO_MODE)
            .unwrap_or(INT10_MODE_03H_TEXT);
        let cols = self
            .read_bda_cols()
            .unwrap_or(VGA_TEXT_COLS as u16)
            .min(u16::from(u8::MAX)) as u8;
        let page = self.read_bda_u8(BDA_ACTIVE_PAGE).unwrap_or(0);
        self.cpu.set_al(mode);
        self.cpu.set_ah(cols);
        self.cpu.set_gpr_u8(4 + CpuState::RBX, page); // BH
    }

    /// AH=09h / AH=0Ah write character at cursor. Spec: RBIL — AL=char, BH=page,
    /// CX=count; AH=09h also uses BL=attribute. Cursor position is unchanged.
    ///
    /// Page 0 + text mode only. Count is capped at [`INT10_WRITE_CHAR_MAX_COUNT`]
    /// and stops at the end of the 80×25 page. Graphics / multi-page are out.
    fn int10_write_char(&mut self, with_attr: bool) {
        if self.vga.render_mode() != VgaRenderMode::Text {
            return;
        }
        let page = self.cpu.gpr_u8(4 + CpuState::RBX); // BH
        if page != 0 {
            return;
        }
        let ch = self.cpu.al();
        let attr = if with_attr {
            self.cpu.gpr_u8_low(CpuState::RBX) // BL
        } else {
            0
        };
        let count = self.cpu.gpr_u16(CpuState::RCX).min(INT10_WRITE_CHAR_MAX_COUNT);
        if count == 0 {
            return;
        }
        let (row0, col0) = self.read_bda_cursor().unwrap_or((0, 0));
        let cols = self
            .read_bda_cols()
            .unwrap_or(VGA_TEXT_COLS as u16)
            .max(1) as usize;
        let rows = VGA_TEXT_ROWS;
        let mut row = usize::from(row0);
        let mut col = usize::from(col0);
        for _ in 0..count {
            if row >= rows {
                break;
            }
            if with_attr {
                let _ = self.vga.put_char(row, col, ch, attr);
            } else {
                let existing = self.vga.attr_at(row, col).unwrap_or(INT10_TTY_ATTR);
                let _ = self.vga.put_char(row, col, ch, existing);
            }
            col += 1;
            if col >= cols {
                col = 0;
                row += 1;
            }
        }
        // Spec: RBIL AH=09h/0Ah — do not advance the stored cursor.
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

    /// Write the core BDA video fields kept coherent with AH=00/02/03/0F.
    ///
    /// Spec: RBIL BIOS Data Area — mode `0040:0049`, columns `0040:004A`,
    /// page size `0040:004C`, page start `0040:004E`, cursor `0040:0050`,
    /// active page `0040:0062`.
    fn write_bda_video(&mut self, mode: u8, cols: u16, page_size: u16, row: u8, col: u8) {
        let _ = self.mem.write_u8(BDA_VIDEO_MODE, mode);
        let _ = self.mem.write_u8(BDA_VIDEO_COLS, (cols & 0xFF) as u8);
        let _ = self.mem.write_u8(BDA_VIDEO_COLS + 1, (cols >> 8) as u8);
        let _ = self.write_bda_u16(BDA_VIDEO_PAGE_SIZE, page_size);
        let _ = self.write_bda_u16(BDA_VIDEO_PAGE_START, 0);
        let _ = self.mem.write_u8(BDA_ACTIVE_PAGE, 0);
        let _ = self.write_bda_cursor(row, col);
    }

    fn write_bda_u16(&mut self, phys: u64, val: u16) -> Result<(), MachineError> {
        self.mem
            .write_u8(phys, (val & 0xFF) as u8)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        self.mem
            .write_u8(phys + 1, (val >> 8) as u8)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        Ok(())
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

    fn write_bda_cursor_type(&mut self, start: u8, end: u8) -> Result<(), MachineError> {
        // Spec: RBIL — cursor type word matches CX (CH=start, CL=end).
        self.mem
            .write_u8(BDA_CURSOR_TYPE, end)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        self.mem
            .write_u8(BDA_CURSOR_TYPE + 1, start)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        Ok(())
    }

    fn read_bda_cursor(&self) -> Option<(u8, u8)> {
        let col = self.read_bda_u8(BDA_CURSOR_PAGE0)?;
        let row = self.read_bda_u8(BDA_CURSOR_PAGE0 + 1)?;
        Some((row, col))
    }

    fn read_bda_cursor_type(&self) -> Option<(u8, u8)> {
        let end = self.read_bda_u8(BDA_CURSOR_TYPE)?;
        let start = self.read_bda_u8(BDA_CURSOR_TYPE + 1)?;
        Some((start, end))
    }

    fn read_bda_cols(&self) -> Option<u16> {
        let lo = self.read_bda_u8(BDA_VIDEO_COLS)?;
        let hi = self.read_bda_u8(BDA_VIDEO_COLS + 1)?;
        Some(u16::from(lo) | (u16::from(hi) << 8))
    }

    #[cfg(test)]
    fn read_bda_u16(&self, phys: u64) -> Option<u16> {
        let lo = self.read_bda_u8(phys)?;
        let hi = self.read_bda_u8(phys + 1)?;
        Some(u16::from(lo) | (u16::from(hi) << 8))
    }

    #[cfg(test)]
    fn read_bda_page_size(&self) -> Option<u16> {
        self.read_bda_u16(BDA_VIDEO_PAGE_SIZE)
    }

    #[cfg(test)]
    fn read_bda_page_start(&self) -> Option<u16> {
        self.read_bda_u16(BDA_VIDEO_PAGE_START)
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

/// Load AH/CH/CL for SET CURSOR TYPE.
pub fn setup_int10_set_cursor_type(cpu: &mut CpuState, start: u8, end: u8) {
    cpu.set_ah(INT10_AH_SET_CURSOR_TYPE);
    cpu.set_gpr_u8(4 + CpuState::RCX, start); // CH
    cpu.set_gpr_u8_low(CpuState::RCX, end); // CL
}

/// Load AH/BH/DH/DL for SET CURSOR POSITION.
pub fn setup_int10_set_cursor(cpu: &mut CpuState, page: u8, row: u8, col: u8) {
    cpu.set_ah(INT10_AH_SET_CURSOR);
    cpu.set_gpr_u8(4 + CpuState::RBX, page); // BH
    cpu.set_gpr_u8(4 + CpuState::RDX, row); // DH
    cpu.set_gpr_u8_low(CpuState::RDX, col); // DL
}

/// Load AH/BH for GET CURSOR POSITION AND SIZE.
pub fn setup_int10_get_cursor(cpu: &mut CpuState, page: u8) {
    cpu.set_ah(INT10_AH_GET_CURSOR);
    cpu.set_gpr_u8(4 + CpuState::RBX, page); // BH
}

/// Load AH for GET CURRENT VIDEO MODE.
pub fn setup_int10_get_mode(cpu: &mut CpuState) {
    cpu.set_ah(INT10_AH_GET_MODE);
}

/// Load AH/AL for teletype output.
pub fn setup_int10_teletype(cpu: &mut CpuState, ch: u8) {
    cpu.set_ah(INT10_AH_TELETYPE);
    cpu.set_al(ch);
}

/// Load AH=09h write character and attribute.
pub fn setup_int10_write_char_attr(cpu: &mut CpuState, ch: u8, page: u8, attr: u8, count: u16) {
    cpu.set_ah(INT10_AH_WRITE_CHAR_ATTR);
    cpu.set_al(ch);
    cpu.set_gpr_u8(4 + CpuState::RBX, page); // BH
    cpu.set_gpr_u8_low(CpuState::RBX, attr); // BL
    cpu.set_gpr_u16(CpuState::RCX, count);
}

/// Load AH=0Ah write character only.
pub fn setup_int10_write_char(cpu: &mut CpuState, ch: u8, page: u8, count: u16) {
    cpu.set_ah(INT10_AH_WRITE_CHAR);
    cpu.set_al(ch);
    cpu.set_gpr_u8(4 + CpuState::RBX, page); // BH
    cpu.set_gpr_u16(CpuState::RCX, count);
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
        assert_eq!(
            m.read_bda_u16(BDA_VIDEO_PAGE_SIZE),
            Some(INT10_MODE03_PAGE_SIZE)
        );
        assert_eq!(m.read_bda_u16(BDA_VIDEO_PAGE_START), Some(0));
        assert_eq!(m.mem.read_u8(BDA_ACTIVE_PAGE).unwrap(), 0);
        assert_eq!(
            m.read_bda_cursor_type(),
            Some((INT10_MODE03_CURSOR_START, INT10_MODE03_CURSOR_END))
        );
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
        assert_eq!(m.read_bda_page_size(), Some(INT10_MODE13_PAGE_SIZE));
        assert_eq!(m.read_bda_u16(BDA_VIDEO_PAGE_START), Some(0));
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
    fn int10_ah01_sets_bda_and_crtc_cursor_type() {
        // Spec: RBIL INT 10h AH=01h — CH=start, CL=end → BDA 0040:0060 + CRTC.
        let mut m = Machine::new(1024 * 1024);
        setup_int10_set_mode(&mut m.cpu, INT10_MODE_03H_TEXT);
        m.service_int10();

        setup_int10_set_cursor_type(&mut m.cpu, 0x0A, 0x0B);
        m.service_int10();

        assert_eq!(m.read_bda_cursor_type(), Some((0x0A, 0x0B)));
        assert_eq!(m.vga.crtc_cursor_start_scanline(), 0x0A);
        assert_eq!(m.vga.crtc_cursor_end_scanline(), 0x0B);
        assert!(!m.vga.crtc_cursor_disabled());

        // AH=03h must observe the new type. Spec: RBIL AH=03h CH/CL.
        setup_int10_get_cursor(&mut m.cpu, 0);
        m.service_int10();
        assert_eq!(m.cpu.gpr_u8(4 + CpuState::RCX), 0x0A); // CH
        assert_eq!(m.cpu.gpr_u8_low(CpuState::RCX), 0x0B); // CL
    }

    #[test]
    fn int10_ah01_cursor_disable_bit_reaches_crtc() {
        // Spec: FreeVGA / IBM VGA — Cursor Start bit5 disables the cursor.
        let mut m = Machine::new(1024 * 1024);
        setup_int10_set_mode(&mut m.cpu, INT10_MODE_03H_TEXT);
        m.service_int10();
        setup_int10_set_cursor_type(&mut m.cpu, 0x20 | 0x06, 0x07);
        m.service_int10();
        assert!(m.vga.crtc_cursor_disabled());
        assert_eq!(m.read_bda_cursor_type(), Some((0x26, 0x07)));
    }

    #[test]
    fn int10_ah02_sets_bda_cursor_page0() {
        // Spec: RBIL INT 10h AH=02h — BH=page, DH=row, DL=col → BDA 0040:0050.
        let mut m = Machine::new(1024 * 1024);
        setup_int10_set_mode(&mut m.cpu, INT10_MODE_03H_TEXT);
        m.service_int10();

        setup_int10_set_cursor(&mut m.cpu, 0, 12, 40);
        m.service_int10();

        assert_eq!(m.mem.read_u8(BDA_CURSOR_PAGE0).unwrap(), 40);
        assert_eq!(m.mem.read_u8(BDA_CURSOR_PAGE0 + 1).unwrap(), 12);
        assert_eq!(m.read_bda_cursor(), Some((12, 40)));
    }

    #[test]
    fn int10_ah03_returns_cursor_pos_and_type() {
        // Spec: RBIL INT 10h AH=03h — DH/DL position; CH/CL start/end scanlines.
        let mut m = Machine::new(1024 * 1024);
        setup_int10_set_mode(&mut m.cpu, INT10_MODE_03H_TEXT);
        m.service_int10();
        setup_int10_set_cursor(&mut m.cpu, 0, 5, 10);
        m.service_int10();

        setup_int10_get_cursor(&mut m.cpu, 0);
        m.service_int10();

        assert_eq!(m.cpu.gpr_u8(4 + CpuState::RDX), 5); // DH
        assert_eq!(m.cpu.gpr_u8_low(CpuState::RDX), 10); // DL
        assert_eq!(m.cpu.gpr_u8(4 + CpuState::RCX), INT10_MODE03_CURSOR_START); // CH
        assert_eq!(m.cpu.gpr_u8_low(CpuState::RCX), INT10_MODE03_CURSOR_END); // CL
    }

    #[test]
    fn int10_ah02_clamps_to_screen() {
        let mut m = Machine::new(1024 * 1024);
        setup_int10_set_mode(&mut m.cpu, INT10_MODE_03H_TEXT);
        m.service_int10();
        setup_int10_set_cursor(&mut m.cpu, 0, 200, 200);
        m.service_int10();
        assert_eq!(m.mem.read_u8(BDA_CURSOR_PAGE0).unwrap(), 79);
        assert_eq!(m.mem.read_u8(BDA_CURSOR_PAGE0 + 1).unwrap(), 24);
    }

    #[test]
    fn int10_ah02_ignores_nonzero_page() {
        let mut m = Machine::new(1024 * 1024);
        setup_int10_set_mode(&mut m.cpu, INT10_MODE_03H_TEXT);
        m.service_int10();
        setup_int10_set_cursor(&mut m.cpu, 1, 3, 4);
        m.service_int10();
        assert_eq!(m.mem.read_u8(BDA_CURSOR_PAGE0).unwrap(), 0);
        assert_eq!(m.mem.read_u8(BDA_CURSOR_PAGE0 + 1).unwrap(), 0);
    }

    #[test]
    fn int10_ah0f_returns_mode_cols_page() {
        // Spec: RBIL INT 10h AH=0Fh — AL=mode, AH=columns, BH=active page.
        let mut m = Machine::new(1024 * 1024);
        setup_int10_set_mode(&mut m.cpu, INT10_MODE_03H_TEXT);
        m.service_int10();

        setup_int10_get_mode(&mut m.cpu);
        m.service_int10();
        assert_eq!(m.cpu.al(), INT10_MODE_03H_TEXT);
        assert_eq!(m.cpu.ah(), VGA_TEXT_COLS as u8);
        assert_eq!(m.cpu.gpr_u8(4 + CpuState::RBX), 0); // BH

        setup_int10_set_mode(&mut m.cpu, INT10_MODE_13H_GRAPHICS);
        m.service_int10();
        setup_int10_get_mode(&mut m.cpu);
        m.service_int10();
        assert_eq!(m.cpu.al(), INT10_MODE_13H_GRAPHICS);
        assert_eq!(m.cpu.ah(), 40);
        assert_eq!(m.cpu.gpr_u8(4 + CpuState::RBX), 0);
    }

    #[test]
    fn int10_bda_video_fields_coherent_after_mode_cursor_get() {
        // Spec: RBIL BDA video map — mode/cols/page size/cursor type/page stay
        // coherent across AH=00h / AH=02h / AH=03h / AH=0Fh.
        let mut m = Machine::new(1024 * 1024);
        setup_int10_set_mode(&mut m.cpu, INT10_MODE_03H_TEXT);
        m.service_int10();

        assert_eq!(m.mem.read_u8(BDA_VIDEO_MODE).unwrap(), INT10_MODE_03H_TEXT);
        assert_eq!(m.read_bda_cols(), Some(VGA_TEXT_COLS as u16));
        assert_eq!(m.read_bda_page_size(), Some(INT10_MODE03_PAGE_SIZE));
        assert_eq!(m.read_bda_page_start(), Some(0));
        assert_eq!(m.mem.read_u8(BDA_ACTIVE_PAGE).unwrap(), 0);
        assert_eq!(
            m.read_bda_cursor_type(),
            Some((INT10_MODE03_CURSOR_START, INT10_MODE03_CURSOR_END))
        );

        setup_int10_set_cursor(&mut m.cpu, 0, 3, 7);
        m.service_int10();
        setup_int10_get_cursor(&mut m.cpu, 0);
        m.service_int10();
        assert_eq!(m.cpu.gpr_u8(4 + CpuState::RDX), 3);
        assert_eq!(m.cpu.gpr_u8_low(CpuState::RDX), 7);

        setup_int10_get_mode(&mut m.cpu);
        m.service_int10();
        assert_eq!(m.cpu.al(), INT10_MODE_03H_TEXT);
        assert_eq!(m.cpu.ah(), VGA_TEXT_COLS as u8);
        // Cursor position must survive AH=0Fh (does not rewrite BDA).
        assert_eq!(m.read_bda_cursor(), Some((3, 7)));
        assert_eq!(m.read_bda_page_size(), Some(INT10_MODE03_PAGE_SIZE));
        assert_eq!(m.read_bda_page_start(), Some(0));

        setup_int10_set_mode(&mut m.cpu, INT10_MODE_13H_GRAPHICS);
        m.service_int10();
        assert_eq!(m.read_bda_page_size(), Some(INT10_MODE13_PAGE_SIZE));
        assert_eq!(m.read_bda_page_start(), Some(0));
        assert_eq!(m.read_bda_cols(), Some(40));
        assert_eq!(m.read_bda_cursor(), Some((0, 0)));
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
    fn int10_ah09_writes_char_attr_without_advancing_cursor() {
        // Spec: RBIL INT 10h AH=09h — AL/BL/CX at cursor; cursor stays put.
        let mut m = Machine::new(1024 * 1024);
        setup_int10_set_mode(&mut m.cpu, INT10_MODE_03H_TEXT);
        m.service_int10();
        setup_int10_set_cursor(&mut m.cpu, 0, 2, 5);
        m.service_int10();

        setup_int10_write_char_attr(&mut m.cpu, b'*', 0, 0x1E, 3);
        m.service_int10();

        assert_eq!(m.vga.char_at(2, 5), Some(b'*'));
        assert_eq!(m.vga.attr_at(2, 5), Some(0x1E));
        assert_eq!(m.vga.char_at(2, 6), Some(b'*'));
        assert_eq!(m.vga.attr_at(2, 6), Some(0x1E));
        assert_eq!(m.vga.char_at(2, 7), Some(b'*'));
        assert_eq!(m.vga.attr_at(2, 7), Some(0x1E));
        assert_eq!(m.vga.char_at(2, 8), Some(b' ')); // untouched
        assert_eq!(m.read_bda_cursor(), Some((2, 5)));
    }

    #[test]
    fn int10_ah0a_writes_char_preserves_attribute() {
        // Spec: RBIL INT 10h AH=0Ah — character only; attribute unchanged.
        let mut m = Machine::new(1024 * 1024);
        setup_int10_set_mode(&mut m.cpu, INT10_MODE_03H_TEXT);
        m.service_int10();
        assert!(m.vga.put_char(0, 0, b' ', 0x4E));
        assert!(m.vga.put_char(0, 1, b' ', 0x4E));

        setup_int10_write_char(&mut m.cpu, b'Z', 0, 2);
        m.service_int10();

        assert_eq!(m.vga.char_at(0, 0), Some(b'Z'));
        assert_eq!(m.vga.attr_at(0, 0), Some(0x4E));
        assert_eq!(m.vga.char_at(0, 1), Some(b'Z'));
        assert_eq!(m.vga.attr_at(0, 1), Some(0x4E));
        assert_eq!(m.read_bda_cursor(), Some((0, 0)));
    }

    #[test]
    fn int10_ah09_wraps_within_page_and_ignores_nonzero_page() {
        let mut m = Machine::new(1024 * 1024);
        setup_int10_set_mode(&mut m.cpu, INT10_MODE_03H_TEXT);
        m.service_int10();
        setup_int10_set_cursor(&mut m.cpu, 0, 0, 78);
        m.service_int10();
        setup_int10_write_char_attr(&mut m.cpu, b'W', 0, 0x07, 3);
        m.service_int10();
        assert_eq!(m.vga.char_at(0, 78), Some(b'W'));
        assert_eq!(m.vga.char_at(0, 79), Some(b'W'));
        assert_eq!(m.vga.char_at(1, 0), Some(b'W'));
        assert_eq!(m.read_bda_cursor(), Some((0, 78)));

        // Non-zero page is a no-op in this stub.
        setup_int10_write_char_attr(&mut m.cpu, b'X', 1, 0x07, 1);
        m.service_int10();
        assert_eq!(m.vga.char_at(0, 78), Some(b'W'));
    }

    #[test]
    fn int10_unsupported_ah_is_noop() {
        let mut m = Machine::new(1024 * 1024);
        m.vga.put_char(0, 0, b'X', 0x07);
        m.cpu.set_ah(0x05); // SELECT ACTIVE DISPLAY PAGE — not in this stub
        m.cpu.set_al(0);
        m.service_int10();
        assert_eq!(m.vga.char_at(0, 0), Some(b'X'));
    }
}
