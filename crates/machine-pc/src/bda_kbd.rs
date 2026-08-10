//! Host-side IBM PC/AT BDA keyboard circular buffer (IRQ1 → `40:1E`).
//!
//! Classic BIOS INT 09h reads the 8042 data port on IRQ1 and enqueues a
//! `(ASCII, Set-1 scancode)` word into the BIOS Data Area ring at
//! `0040:001E` with head/tail pointers at `0040:001A` / `0040:001C`. This
//! module is a **host helper** that performs that enqueue path; it does **not**
//! install a guest INT 09h IVT body.
//!
//! Spec: IBM PC/AT Technical Reference — BDA keyboard buffer layout
//! (`0040:001A` head, `0040:001C` tail, `0040:001E`–`0040:003D` ring of 16
//! words); Ralf Brown's Interrupt List memory map `0040h`.

use crate::{Machine, MachineError};

/// BDA segment physical base (`0040:0000`).
pub const BDA_PHYS_BASE: u64 = 0x400;

/// Keyboard buffer head pointer (offset within segment `40h`).
pub const BDA_KBD_HEAD: u64 = BDA_PHYS_BASE + 0x1A;
/// Keyboard buffer tail pointer (offset within segment `40h`).
pub const BDA_KBD_TAIL: u64 = BDA_PHYS_BASE + 0x1C;
/// Start of the 16-word circular buffer (`0040:001E`).
pub const BDA_KBD_BUF_START_OFF: u16 = 0x1E;
/// End sentinel for the ring (`0040:003E` — one past the last word).
pub const BDA_KBD_BUF_END_OFF: u16 = 0x3E;
/// Physical address of the first buffer word.
pub const BDA_KBD_BUF_START: u64 = BDA_PHYS_BASE + BDA_KBD_BUF_START_OFF as u64;

/// Capacity of the classic BDA keyboard ring (15 usable words; one slot
/// distinguishes empty from full when head==tail).
pub const BDA_KBD_CAPACITY: usize = 15;

impl Machine {
    /// Initialize classic BDA keyboard head/tail to an empty ring at `40:1E`.
    ///
    /// Spec: IBM PC/AT BDA — empty buffer has head == tail == `001Eh`.
    pub fn init_bda_kbd_buffer(&mut self) -> Result<(), MachineError> {
        self.write_bda_kbd_ptr(BDA_KBD_HEAD, BDA_KBD_BUF_START_OFF)?;
        self.write_bda_kbd_ptr(BDA_KBD_TAIL, BDA_KBD_BUF_START_OFF)?;
        Ok(())
    }

    /// Enqueue one BIOS keystroke word into the BDA circular buffer.
    ///
    /// Spec: IBM PC/AT — store `AL=ASCII`, `AH=Set-1 scancode` at the current
    /// tail, then advance the tail by two bytes, wrapping at `003Eh`. Returns
    /// `false` when the ring is full (next tail would equal head).
    pub fn bda_kbd_enqueue(&mut self, ascii: u8, scancode: u8) -> Result<bool, MachineError> {
        let head = self.read_bda_kbd_ptr(BDA_KBD_HEAD)?;
        let tail = self.read_bda_kbd_ptr(BDA_KBD_TAIL)?;
        let next = advance_bda_kbd_off(tail);
        if next == head {
            return Ok(false);
        }
        let phys = BDA_PHYS_BASE + u64::from(tail);
        self.mem
            .write_u8(phys, ascii)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        self.mem
            .write_u8(phys + 1, scancode)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        self.write_bda_kbd_ptr(BDA_KBD_TAIL, next)?;
        Ok(true)
    }

    /// Host inject: enqueue `(ascii, scancode)` without touching the 8042.
    ///
    /// Ensures head/tail look like a classic empty ring when both are zero
    /// (uninitialized BDA), then enqueues.
    pub fn bda_kbd_inject_key(&mut self, ascii: u8, scancode: u8) -> Result<bool, MachineError> {
        self.ensure_bda_kbd_ring()?;
        self.bda_kbd_enqueue(ascii, scancode)
    }

    /// Consume one keyboard-sourced 8042 OBF byte and enqueue a make code.
    ///
    /// Spec: IBM PC/AT INT 09h path — `IN 60h` clears OBF/IRQ1; make codes
    /// (Set-1 bit7 clear) become buffer words; break codes (bit7 set) are
    /// dropped for this bounded host stub. Returns `Ok(None)` when there is no
    /// keyboard OBF, or when the byte was a break / aux-sourced byte.
    ///
    /// Does **not** EOI the PIC and does **not** run a guest INT 09h body.
    pub fn service_kbd_irq1_bda(&mut self) -> Result<Option<(u8, u8)>, MachineError> {
        use devices::{PortDevice, STATUS_OBF};

        // Keyboard-sourced OBF only (aux bytes raise IRQ12, not this path).
        if self.kbd.status() & STATUS_OBF == 0 || self.kbd.aux_obf() {
            return Ok(None);
        }
        // Spec: IBM PC/AT INT 09h — `IN 60h` clears OBF / IRQ1.
        let scancode = self.kbd.port_read(I8042_DATA, 1) as u8;
        self.sync_kbd_irq1();
        if scancode & 0x80 != 0 {
            // Set-1 break: ignore for the typeahead ring (no shift-state track).
            return Ok(None);
        }
        self.ensure_bda_kbd_ring()?;
        let ascii = set1_make_to_ascii(scancode);
        if !self.bda_kbd_enqueue(ascii, scancode)? {
            return Ok(None);
        }
        Ok(Some((ascii, scancode)))
    }

    /// Inject a Set-1 make scancode through the 8042, then drain IRQ1 → BDA.
    ///
    /// Combines [`Machine::kbd_inject_scancode`] with
    /// [`Machine::service_kbd_irq1_bda`]. Returns whether a BDA word was
    /// enqueued.
    pub fn kbd_inject_scancode_to_bda(&mut self, make_code: u8) -> Result<bool, MachineError> {
        let _ = self.kbd_inject_scancode(make_code);
        Ok(self.service_kbd_irq1_bda()?.is_some())
    }

    /// How many keystrokes currently wait in the BDA ring.
    pub fn bda_kbd_len(&self) -> Result<usize, MachineError> {
        let head = self.read_bda_kbd_ptr(BDA_KBD_HEAD)?;
        let tail = self.read_bda_kbd_ptr(BDA_KBD_TAIL)?;
        Ok(bda_kbd_count(head, tail))
    }

    fn ensure_bda_kbd_ring(&mut self) -> Result<(), MachineError> {
        let head = self.read_bda_kbd_ptr(BDA_KBD_HEAD)?;
        let tail = self.read_bda_kbd_ptr(BDA_KBD_TAIL)?;
        if head == 0 && tail == 0 {
            self.init_bda_kbd_buffer()?;
        }
        Ok(())
    }

    fn read_bda_kbd_ptr(&self, phys: u64) -> Result<u16, MachineError> {
        let lo = self
            .mem
            .read_u8(phys)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        let hi = self
            .mem
            .read_u8(phys + 1)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        Ok(u16::from(lo) | (u16::from(hi) << 8))
    }

    fn write_bda_kbd_ptr(&mut self, phys: u64, off: u16) -> Result<(), MachineError> {
        self.mem
            .write_u8(phys, (off & 0xFF) as u8)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        self.mem
            .write_u8(phys + 1, (off >> 8) as u8)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        Ok(())
    }
}

use devices::I8042_DATA;

fn advance_bda_kbd_off(off: u16) -> u16 {
    let next = off.wrapping_add(2);
    if next >= BDA_KBD_BUF_END_OFF {
        BDA_KBD_BUF_START_OFF
    } else {
        next
    }
}

fn bda_kbd_count(head: u16, tail: u16) -> usize {
    if head == tail {
        return 0;
    }
    let mut n = 0usize;
    let mut cur = head;
    while cur != tail {
        n += 1;
        cur = advance_bda_kbd_off(cur);
        if n > BDA_KBD_CAPACITY {
            break;
        }
    }
    n
}

/// Bounded Set-1 make → ASCII (unshifted US layout) for host BDA enqueue.
///
/// Spec: IBM PC/AT keyboard — BIOS typeahead stores ASCII in the low byte.
/// Letters, digits, space, and Enter only; everything else yields `0`.
fn set1_make_to_ascii(scancode: u8) -> u8 {
    match scancode {
        0x02 => b'1',
        0x03 => b'2',
        0x04 => b'3',
        0x05 => b'4',
        0x06 => b'5',
        0x07 => b'6',
        0x08 => b'7',
        0x09 => b'8',
        0x0A => b'9',
        0x0B => b'0',
        0x10 => b'q',
        0x11 => b'w',
        0x12 => b'e',
        0x13 => b'r',
        0x14 => b't',
        0x15 => b'y',
        0x16 => b'u',
        0x17 => b'i',
        0x18 => b'o',
        0x19 => b'p',
        0x1C => b'\r',
        0x1E => b'a',
        0x1F => b's',
        0x20 => b'd',
        0x21 => b'f',
        0x22 => b'g',
        0x23 => b'h',
        0x24 => b'j',
        0x25 => b'k',
        0x26 => b'l',
        0x2C => b'z',
        0x2D => b'x',
        0x2E => b'c',
        0x2F => b'v',
        0x30 => b'b',
        0x31 => b'n',
        0x32 => b'm',
        0x39 => b' ',
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use devices::{PortDevice, CFG_INT1, CFG_TRANSLATE, I8042_STATUS_CMD};

    fn read_word(m: &Machine, phys: u64) -> u16 {
        let lo = m.mem.read_u8(phys).unwrap();
        let hi = m.mem.read_u8(phys + 1).unwrap();
        u16::from(lo) | (u16::from(hi) << 8)
    }

    /// Enable first-port clock + IRQ1 + Set2→Set1 translation (POST-like).
    fn enable_kbd_irq1(m: &mut Machine) {
        m.kbd.port_write(I8042_STATUS_CMD, 1, 0x60); // write config
        m.kbd
            .port_write(I8042_DATA, 1, u32::from(CFG_INT1 | CFG_TRANSLATE));
        m.sync_kbd_irq1();
    }

    /// Spec: IBM PC/AT BDA — empty ring head==tail==`001Eh`.
    #[test]
    fn bda_kbd_init_empty_ring() {
        let mut m = Machine::new(64 * 1024);
        m.init_bda_kbd_buffer().unwrap();
        assert_eq!(read_word(&m, BDA_KBD_HEAD), BDA_KBD_BUF_START_OFF);
        assert_eq!(read_word(&m, BDA_KBD_TAIL), BDA_KBD_BUF_START_OFF);
        assert_eq!(m.bda_kbd_len().unwrap(), 0);
    }

    /// Spec: IBM PC/AT — enqueue stores ASCII/scancode and advances tail.
    #[test]
    fn bda_kbd_inject_enqueues_word() {
        let mut m = Machine::new(64 * 1024);
        assert!(m.bda_kbd_inject_key(b'A', 0x1E).unwrap());
        assert_eq!(m.bda_kbd_len().unwrap(), 1);
        assert_eq!(read_word(&m, BDA_KBD_BUF_START), 0x1E41);
        assert_eq!(read_word(&m, BDA_KBD_HEAD), BDA_KBD_BUF_START_OFF);
        assert_eq!(read_word(&m, BDA_KBD_TAIL), BDA_KBD_BUF_START_OFF + 2);
    }

    /// Spec: IBM PC/AT — ring holds 15 words (empty≠full via wasted slot); 16th fails.
    #[test]
    fn bda_kbd_ring_full() {
        let mut m = Machine::new(64 * 1024);
        m.init_bda_kbd_buffer().unwrap();
        for i in 0..BDA_KBD_CAPACITY {
            assert!(
                m.bda_kbd_enqueue(i as u8, 0x10).unwrap(),
                "slot {i} should enqueue"
            );
        }
        assert!(!m.bda_kbd_enqueue(0xFF, 0x10).unwrap());
        assert_eq!(m.bda_kbd_len().unwrap(), BDA_KBD_CAPACITY);
    }

    /// Spec: IBM PC/AT INT 09h-like path — 8042 make → `IN 60h` → BDA word.
    #[test]
    fn irq1_scancode_drains_to_bda() {
        let mut m = Machine::new(64 * 1024);
        enable_kbd_irq1(&mut m);
        // Set-2 'A' (0x1C) → translated Set-1 0x1E on OBF.
        assert!(m.kbd_inject_scancode(0x1C));
        assert!(m.kbd.irq1_line());
        let got = m.service_kbd_irq1_bda().unwrap();
        assert_eq!(got, Some((b'a', 0x1E)));
        assert!(!m.kbd.irq1_line());
        assert_eq!(m.bda_kbd_len().unwrap(), 1);
        assert_eq!(read_word(&m, BDA_KBD_BUF_START), 0x1E61);
    }

    /// Spec: Set-1 break codes are not placed in the typeahead ring.
    #[test]
    fn break_code_not_enqueued() {
        let mut m = Machine::new(64 * 1024);
        enable_kbd_irq1(&mut m);
        m.init_bda_kbd_buffer().unwrap();
        // Place a raw Set-1 break ('A' break = 0x9E) without translation path.
        assert!(m.kbd.place_output(0x9E));
        m.sync_kbd_irq1();
        assert!(m.service_kbd_irq1_bda().unwrap().is_none());
        assert_eq!(m.bda_kbd_len().unwrap(), 0);
    }

    /// Combined host helper: inject through 8042 then drain to BDA.
    #[test]
    fn kbd_inject_scancode_to_bda_helper() {
        let mut m = Machine::new(64 * 1024);
        enable_kbd_irq1(&mut m);
        assert!(m.kbd_inject_scancode_to_bda(0x1C).unwrap());
        assert_eq!(m.bda_kbd_len().unwrap(), 1);
    }
}
