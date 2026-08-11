//! Host-side IBM BIOS INT 16h keyboard subset (AH=00h / AH=01h / AH=02h / AH=12h).
//!
//! Closest approach in-tree to SeaBIOS/FreeDOS keyboard services for the
//! measure path: a **host** typeahead buffer of `(ASCII, Set-1 scancode)` pairs
//! plus a dispatcher that applies classic INT 16h register conventions.
//! When the host buffer is empty, AH=00h/01h fall back to the classic BDA
//! keyboard ring (`0040:001A`/`001C`/`001E`) for path honesty with IRQ1→BDA
//! helpers. AH=02h/12h read BDA shift flags (`0040:0017` / synthesized AH).
//! This is **not** a guest IVT BIOS body.
//!
//! Spec: Ralf Brown's Interrupt List — INT 16h AH=00h (get keystroke) /
//! AH=01h (check for keystroke) / AH=02h (shift flags) / AH=12h (extended
//! shift states); IBM PC BIOS keyboard services.

use crate::{Machine, MachineError};

/// AH=00h — read (wait for) next keystroke; remove from buffer.
pub const INT16_AH_GET_KEYSTROKE: u8 = 0x00;
/// AH=01h — check whether a keystroke is ready; do not remove.
pub const INT16_AH_CHECK_KEYSTROKE: u8 = 0x01;
/// AH=02h — get shift flags (`AL` = BDA `0040:0017`).
pub const INT16_AH_SHIFT_STATUS: u8 = 0x02;
/// AH=12h — get extended shift states (enhanced keyboard).
pub const INT16_AH_EXTENDED_SHIFT_STATUS: u8 = 0x12;

/// Capacity of the host INT 16h typeahead buffer (classic BDA ring is 16 words).
pub const INT16_BUFFER_CAP: usize = 16;

/// One BIOS keystroke: ASCII in `AL`, Set-1 make scancode in `AH`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Int16Key {
    pub ascii: u8,
    pub scancode: u8,
}

impl Int16Key {
    pub const fn new(ascii: u8, scancode: u8) -> Self {
        Self { ascii, scancode }
    }

    pub fn ax(self) -> u16 {
        u16::from(self.scancode) << 8 | u16::from(self.ascii)
    }
}

/// RFLAGS bit 6 — Zero Flag (AH=01h empty → ZF=1).
const RFLAGS_ZF: u64 = 1 << 6;

impl Machine {
    /// Push one keystroke into the host INT 16h typeahead buffer.
    ///
    /// Spec: IBM BIOS keyboard buffer — `(AL=ASCII, AH=Set-1 scancode)`.
    /// Returns false when the bounded buffer is full ([`INT16_BUFFER_CAP`]).
    pub fn int16_push_key(&mut self, ascii: u8, scancode: u8) -> bool {
        if self.int16_buf.len() >= INT16_BUFFER_CAP {
            return false;
        }
        self.int16_buf.push(Int16Key::new(ascii, scancode));
        true
    }

    /// How many keystrokes wait in the host INT 16h buffer.
    pub fn int16_buffer_len(&self) -> usize {
        self.int16_buf.len()
    }

    /// Clear the host INT 16h typeahead buffer.
    pub fn int16_clear(&mut self) {
        self.int16_buf.clear();
    }

    /// Host-side INT 16h dispatch using current CPU `AH`.
    ///
    /// Spec: RBIL INT 16h —
    /// - AH=00h: if a key is available, load `AX` and remove it; if empty, set
    ///   ZF (host stub does **not** busy-wait — callers must push/enqueue first).
    /// - AH=01h: if empty set ZF and leave `AX`; if ready clear ZF, load `AX`,
    ///   leave the key in the buffer.
    /// - AH=02h: `AL` = BDA `0040:0017` shift flags (Table 00582); ZF cleared.
    /// - AH=12h: `AL` = same as AH=02h; `AH` = synthesized extended flags
    ///   (Table 00588 from `40:18` + `40:96`); ZF cleared.
    ///
    /// Source order for AH=00/01: host `int16_buf` first, then the classic BDA
    /// ring (`Machine::bda_kbd_peek` / `bda_kbd_dequeue`) so IRQ1→BDA injects
    /// are visible to the same INT 16h helpers.
    ///
    /// Other AH values leave registers unchanged and set ZF.
    ///
    /// Guest `INT 16h` still needs a real IVT handler (SeaBIOS) or an explicit
    /// call into this API.
    pub fn service_int16(&mut self) {
        match self.cpu.ah() {
            INT16_AH_GET_KEYSTROKE => self.int16_get_keystroke(),
            INT16_AH_CHECK_KEYSTROKE => self.int16_check_keystroke(),
            INT16_AH_SHIFT_STATUS => self.int16_shift_status(),
            INT16_AH_EXTENDED_SHIFT_STATUS => self.int16_ext_shift_status(),
            _ => self.int16_set_zf(true),
        }
    }

    /// Install a real-mode IVT entry for vector `0x16` that points at `handler`.
    ///
    /// Does **not** install a BIOS body — only the far pointer. Host harnesses
    /// that want keyboard services must call [`Self::service_int16`] explicitly.
    pub fn install_int16_ivt_pointer(
        &mut self,
        handler_seg: u16,
        handler_off: u16,
    ) -> Result<(), MachineError> {
        let base = 0x16u64 * 4;
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

    fn int16_get_keystroke(&mut self) {
        if !self.int16_buf.is_empty() {
            let key = self.int16_buf.remove(0);
            self.cpu.set_ax(key.ax());
            self.int16_set_zf(false);
            return;
        }
        // Fall back to classic BDA ring (IRQ1→BDA / inject path).
        match self.bda_kbd_dequeue() {
            Ok(Some((ascii, scancode))) => {
                self.cpu.set_ax(Int16Key::new(ascii, scancode).ax());
                self.int16_set_zf(false);
            }
            _ => {
                // Real BIOS blocks; host stub reports empty via ZF without spinning.
                self.int16_set_zf(true);
            }
        }
    }

    fn int16_check_keystroke(&mut self) {
        if let Some(key) = self.int16_buf.first().copied() {
            self.cpu.set_ax(key.ax());
            self.int16_set_zf(false);
            return;
        }
        match self.bda_kbd_peek() {
            Ok(Some((ascii, scancode))) => {
                self.cpu.set_ax(Int16Key::new(ascii, scancode).ax());
                self.int16_set_zf(false);
            }
            _ => self.int16_set_zf(true),
        }
    }

    /// Spec: RBIL INT 16h AH=02h — `AL` = shift flags at `0040:0017` (Table 00582).
    fn int16_shift_status(&mut self) {
        let flags = self.bda_kbd_flag1().unwrap_or(0);
        self.cpu.set_al(flags);
        self.int16_set_zf(false);
    }

    /// Spec: RBIL INT 16h AH=12h — `AL` = Table 00587 (= AH=02h); `AH` = Table 00588.
    fn int16_ext_shift_status(&mut self) {
        let al = self.bda_kbd_flag1().unwrap_or(0);
        let ah = self.int16_extended_shift_ah().unwrap_or(0);
        self.cpu.set_ax(u16::from(ah) << 8 | u16::from(al));
        self.int16_set_zf(false);
    }

    fn int16_set_zf(&mut self, set: bool) {
        if set {
            self.cpu.rflags |= RFLAGS_ZF;
        } else {
            self.cpu.rflags &= !RFLAGS_ZF;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        KBD_FLAG1_ALT, KBD_FLAG1_CAPS_LOCK, KBD_FLAG1_CTRL, KBD_FLAG1_LEFT_SHIFT,
        KBD_FLAG1_NUM_LOCK, KBD_FLAG1_RIGHT_SHIFT, KBD_FLAG1_SCROLL_LOCK, KBD_FLAG2_LEFT_ALT,
        KBD_FLAG2_LEFT_CTRL, KBD_MODE_RIGHT_ALT, KBD_MODE_RIGHT_CTRL,
    };
    use devices::{PortDevice, CFG_INT1, CFG_TRANSLATE, I8042_DATA, I8042_STATUS_CMD};
    use x86_core::CpuState;

    fn zf(cpu: &CpuState) -> bool {
        cpu.rflags & RFLAGS_ZF != 0
    }

    fn enable_kbd_irq1(m: &mut Machine) {
        m.kbd.port_write(I8042_STATUS_CMD, 1, 0x60);
        m.kbd
            .port_write(I8042_DATA, 1, u32::from(CFG_INT1 | CFG_TRANSLATE));
        m.sync_kbd_irq1();
    }

    /// Spec: RBIL INT 16h AH=01h — empty buffer sets ZF; ready key clears ZF and
    /// loads AX without removing the key.
    #[test]
    fn int16_ah01_check_empty_and_ready() {
        let mut m = Machine::new(64 * 1024);
        m.cpu.set_ah(INT16_AH_CHECK_KEYSTROKE);
        m.service_int16();
        assert!(zf(&m.cpu));

        assert!(m.int16_push_key(b'A', 0x1E));
        m.cpu.set_ah(INT16_AH_CHECK_KEYSTROKE);
        m.service_int16();
        assert!(!zf(&m.cpu));
        assert_eq!(m.cpu.ax(), 0x1E41);
        assert_eq!(m.int16_buffer_len(), 1);
    }

    /// Spec: RBIL INT 16h AH=00h — returns AX and removes the keystroke.
    #[test]
    fn int16_ah00_get_removes_key() {
        let mut m = Machine::new(64 * 1024);
        assert!(m.int16_push_key(b'A', 0x1E));
        assert!(m.int16_push_key(b'B', 0x30));
        m.cpu.set_ah(INT16_AH_GET_KEYSTROKE);
        m.service_int16();
        assert!(!zf(&m.cpu));
        assert_eq!(m.cpu.ax(), 0x1E41);
        assert_eq!(m.int16_buffer_len(), 1);

        m.cpu.set_ah(INT16_AH_GET_KEYSTROKE);
        m.service_int16();
        assert_eq!(m.cpu.ax(), 0x3042);
        assert_eq!(m.int16_buffer_len(), 0);
    }

    /// Spec: host stub does not busy-wait; empty AH=00 sets ZF.
    #[test]
    fn int16_ah00_empty_sets_zf() {
        let mut m = Machine::new(64 * 1024);
        m.cpu.set_ax(0x1234);
        m.cpu.set_ah(INT16_AH_GET_KEYSTROKE);
        m.service_int16();
        assert!(zf(&m.cpu));
        assert_eq!(m.cpu.ax() & 0xFF, 0x34, "AL preserved on empty get");
    }

    #[test]
    fn int16_buffer_cap_and_reset() {
        let mut m = Machine::new(64 * 1024);
        for i in 0..INT16_BUFFER_CAP {
            assert!(m.int16_push_key(i as u8, 0x10));
        }
        assert!(!m.int16_push_key(0xFF, 0x10));
        m.reset();
        assert_eq!(m.int16_buffer_len(), 0);
    }

    #[test]
    fn int16_ivt_pointer_install() {
        let mut m = Machine::new(64 * 1024);
        m.install_int16_ivt_pointer(0xF000, 0xE100).unwrap();
        assert_eq!(m.mem.read_u8(0x16 * 4).unwrap(), 0x00);
        assert_eq!(m.mem.read_u8(0x16 * 4 + 1).unwrap(), 0xE1);
        assert_eq!(m.mem.read_u8(0x16 * 4 + 2).unwrap(), 0x00);
        assert_eq!(m.mem.read_u8(0x16 * 4 + 3).unwrap(), 0xF0);
    }

    /// Spec honesty: empty host buffer falls back to BDA `40:1E` ring.
    #[test]
    fn int16_falls_back_to_bda_ring() {
        let mut m = Machine::new(64 * 1024);
        assert!(m.bda_kbd_inject_key(b'A', 0x1E).unwrap());

        m.cpu.set_ah(INT16_AH_CHECK_KEYSTROKE);
        m.service_int16();
        assert!(!zf(&m.cpu));
        assert_eq!(m.cpu.ax(), 0x1E41);
        assert_eq!(m.bda_kbd_len().unwrap(), 1);

        m.cpu.set_ah(INT16_AH_GET_KEYSTROKE);
        m.service_int16();
        assert!(!zf(&m.cpu));
        assert_eq!(m.cpu.ax(), 0x1E41);
        assert_eq!(m.bda_kbd_len().unwrap(), 0);
    }

    /// Spec: RBIL INT 16h AH=02h — AL = BDA `0040:0017` (Table 00582).
    #[test]
    fn int16_ah02_returns_bda_shift_flags() {
        let mut m = Machine::new(64 * 1024);
        m.seed_bda_keyboard_flags().unwrap();
        let flags = KBD_FLAG1_LEFT_SHIFT
            | KBD_FLAG1_RIGHT_SHIFT
            | KBD_FLAG1_CTRL
            | KBD_FLAG1_ALT
            | KBD_FLAG1_SCROLL_LOCK
            | KBD_FLAG1_NUM_LOCK
            | KBD_FLAG1_CAPS_LOCK;
        m.set_bda_kbd_flag1(flags).unwrap();

        m.cpu.set_ax(0xFFFF);
        m.cpu.set_ah(INT16_AH_SHIFT_STATUS);
        m.service_int16();
        assert!(!zf(&m.cpu));
        assert_eq!(m.cpu.al(), flags);
    }

    /// Spec: inject Left Shift make via IRQ1 path → AH=02 reflects bit1.
    #[test]
    fn int16_ah02_after_modifier_inject() {
        let mut m = Machine::new(64 * 1024);
        enable_kbd_irq1(&mut m);
        m.seed_bda_keyboard_flags().unwrap();

        // Raw Set-1 Left Shift make (0x2A) on OBF.
        assert!(m.kbd.place_output(0x2A));
        m.sync_kbd_irq1();
        assert!(m.service_kbd_irq1_bda().unwrap().is_none());
        assert_eq!(
            m.bda_kbd_flag1().unwrap() & KBD_FLAG1_LEFT_SHIFT,
            KBD_FLAG1_LEFT_SHIFT
        );
        assert_eq!(m.bda_kbd_len().unwrap(), 0);

        m.cpu.set_ah(INT16_AH_SHIFT_STATUS);
        m.service_int16();
        assert_eq!(m.cpu.al() & KBD_FLAG1_LEFT_SHIFT, KBD_FLAG1_LEFT_SHIFT);
    }

    /// Spec: RBIL INT 16h AH=12h — AL = 40:17; AH synthesized (Table 00588).
    #[test]
    fn int16_ah12_extended_shift_status() {
        let mut m = Machine::new(64 * 1024);
        m.seed_bda_keyboard_flags().unwrap();
        m.set_bda_kbd_flag1(KBD_FLAG1_CTRL | KBD_FLAG1_ALT | KBD_FLAG1_LEFT_SHIFT)
            .unwrap();
        m.set_bda_kbd_flag2(KBD_FLAG2_LEFT_CTRL | KBD_FLAG2_LEFT_ALT)
            .unwrap();
        // Right Ctrl/Alt live in 40:96 bits 2–3.
        let mode = m.bda_kbd_mode().unwrap() | KBD_MODE_RIGHT_CTRL | KBD_MODE_RIGHT_ALT;
        m.mem.write_u8(crate::BDA_KBD_MODE, mode).unwrap();

        m.cpu.set_ah(INT16_AH_EXTENDED_SHIFT_STATUS);
        m.service_int16();
        assert!(!zf(&m.cpu));
        assert_eq!(
            m.cpu.al(),
            KBD_FLAG1_CTRL | KBD_FLAG1_ALT | KBD_FLAG1_LEFT_SHIFT
        );
        // Table 00588: bit0 left Ctrl, bit1 left Alt, bit2 right Ctrl, bit3 right Alt.
        assert_eq!(m.cpu.ah(), 0x0F);
    }

    /// Spec: right Ctrl via E0+1D updates AH=12 extended byte bit2.
    #[test]
    fn int16_ah12_right_ctrl_via_e0() {
        let mut m = Machine::new(64 * 1024);
        enable_kbd_irq1(&mut m);
        m.seed_bda_keyboard_flags().unwrap();

        assert!(m.kbd.place_output(0xE0));
        m.sync_kbd_irq1();
        assert!(m.service_kbd_irq1_bda().unwrap().is_none());
        assert!(m.kbd.place_output(0x1D)); // Right Ctrl make
        m.sync_kbd_irq1();
        assert!(m.service_kbd_irq1_bda().unwrap().is_none());

        m.cpu.set_ah(INT16_AH_EXTENDED_SHIFT_STATUS);
        m.service_int16();
        assert_eq!(m.cpu.al() & KBD_FLAG1_CTRL, KBD_FLAG1_CTRL);
        assert_eq!(m.cpu.ah() & (1 << 2), 1 << 2);
        assert_eq!(m.cpu.ah() & (1 << 0), 0, "left Ctrl not set");
    }
}
