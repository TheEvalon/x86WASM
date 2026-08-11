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
//! words); Ralf Brown's Interrupt List memory map `0040h` (including shift
//! flags at `0040:0017`/`0018` and enhanced-keyboard status at `0040:0096`).

use crate::{Machine, MachineError};

/// BDA segment physical base (`0040:0000`).
pub const BDA_PHYS_BASE: u64 = 0x400;

/// Keyboard shift-status flags (`0040:0017`) — INT 16h AH=02h / AH=12h AL.
///
/// Spec: RBIL MEM 0040h:0017h / INT 16h Table 00582.
pub const BDA_KBD_FLAG1: u64 = BDA_PHYS_BASE + 0x17;
/// Extended keyboard status (`0040:0018`) — pressed-state companion to flag1.
///
/// Spec: RBIL MEM 0040h:0018h. Note: INT 16h AH=12h AH is **synthesized** from
/// this byte plus `0040:0096` right-Alt/Ctrl bits (Table 00588), not a raw copy.
pub const BDA_KBD_FLAG2: u64 = BDA_PHYS_BASE + 0x18;
/// Keyboard mode / type / E0 prefix / right Ctrl-Alt (`0040:0096`).
///
/// Spec: RBIL MEM 0040h:0096h — bit4 = enhanced keyboard present.
pub const BDA_KBD_MODE: u64 = BDA_PHYS_BASE + 0x96;
/// Keyboard LED flags (`0040:0097`) — Scroll/Num/Caps mirror.
///
/// Spec: RBIL MEM 0040h:0097h bits 0–2.
pub const BDA_KBD_LEDS: u64 = BDA_PHYS_BASE + 0x97;

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

// --- `0040:0017` shift flags (RBIL Table 00582 / INT 16h AH=02h AL) ---
pub const KBD_FLAG1_RIGHT_SHIFT: u8 = 1 << 0;
pub const KBD_FLAG1_LEFT_SHIFT: u8 = 1 << 1;
pub const KBD_FLAG1_CTRL: u8 = 1 << 2;
pub const KBD_FLAG1_ALT: u8 = 1 << 3;
pub const KBD_FLAG1_SCROLL_LOCK: u8 = 1 << 4;
pub const KBD_FLAG1_NUM_LOCK: u8 = 1 << 5;
pub const KBD_FLAG1_CAPS_LOCK: u8 = 1 << 6;
pub const KBD_FLAG1_INSERT: u8 = 1 << 7;

// --- `0040:0018` pressed-state flags (RBIL MEM 0040h:0018h) ---
pub const KBD_FLAG2_LEFT_CTRL: u8 = 1 << 0;
pub const KBD_FLAG2_LEFT_ALT: u8 = 1 << 1;
pub const KBD_FLAG2_SYSREQ: u8 = 1 << 2;
pub const KBD_FLAG2_PAUSE: u8 = 1 << 3;
pub const KBD_FLAG2_SCROLL_DOWN: u8 = 1 << 4;
pub const KBD_FLAG2_NUM_DOWN: u8 = 1 << 5;
pub const KBD_FLAG2_CAPS_DOWN: u8 = 1 << 6;
pub const KBD_FLAG2_INSERT_DOWN: u8 = 1 << 7;

// --- `0040:0096` enhanced keyboard / E0 prefix ---
pub const KBD_MODE_LAST_E0: u8 = 1 << 0;
pub const KBD_MODE_RIGHT_CTRL: u8 = 1 << 2;
pub const KBD_MODE_RIGHT_ALT: u8 = 1 << 3;
/// Enhanced (101/102-key) keyboard present — FreeDOS / INT 16h AH=12 probes.
pub const KBD_MODE_ENHANCED: u8 = 1 << 4;

// --- `0040:0097` LED mirror (RBIL) ---
pub const KBD_LED_SCROLL: u8 = 1 << 0;
pub const KBD_LED_NUM: u8 = 1 << 1;
pub const KBD_LED_CAPS: u8 = 1 << 2;

impl Machine {
    /// Initialize classic BDA keyboard head/tail to an empty ring at `40:1E`.
    ///
    /// Spec: IBM PC/AT BDA — empty buffer has head == tail == `001Eh`.
    pub fn init_bda_kbd_buffer(&mut self) -> Result<(), MachineError> {
        self.write_bda_kbd_ptr(BDA_KBD_HEAD, BDA_KBD_BUF_START_OFF)?;
        self.write_bda_kbd_ptr(BDA_KBD_TAIL, BDA_KBD_BUF_START_OFF)?;
        Ok(())
    }

    /// Seed BDA keyboard shift/LED/mode fields for FreeDOS-friendly probes.
    ///
    /// Spec: RBIL MEM `0040:0017`/`0018`/`0096`/`0097`; IBM equipment list at
    /// `0040:0010` (also [`crate::guest_boot::BDA_EQUIPMENT`]).
    ///
    /// Honesty:
    /// - Equipment **word bit 0** is **floppy installed** (IBM / INT 11h), **not**
    ///   "keyboard present". Keyboard enabled is equipment **bit 2**
    ///   ([`devices::EQUIP_KEYBOARD_ENABLED`]) — already set by
    ///   [`Self::equipment_byte`].
    /// - This helper writes that equipment low byte, clears shift flags, marks
    ///   enhanced keyboard (`40:96` bit4), and clears LED mirror (`40:97`).
    /// - Does **not** claim a guest INT 09h/16h body or full AT LED hardware.
    pub fn seed_bda_keyboard_flags(&mut self) -> Result<(), MachineError> {
        let equip = self.equipment_byte();
        self.mem
            .write_u8(crate::guest_boot::BDA_EQUIPMENT, equip)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        self.mem
            .write_u8(crate::guest_boot::BDA_EQUIPMENT + 1, 0)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        self.write_kbd_bda_u8(BDA_KBD_FLAG1, 0)?;
        self.write_kbd_bda_u8(BDA_KBD_FLAG2, 0)?;
        self.write_kbd_bda_u8(BDA_KBD_MODE, KBD_MODE_ENHANCED)?;
        self.write_kbd_bda_u8(BDA_KBD_LEDS, 0)?;
        self.ensure_bda_kbd_ring()?;
        Ok(())
    }

    /// Read BDA keyboard shift flags (`0040:0017`).
    pub fn bda_kbd_flag1(&self) -> Result<u8, MachineError> {
        self.read_kbd_bda_u8(BDA_KBD_FLAG1)
    }

    /// Write BDA keyboard shift flags (`0040:0017`).
    pub fn set_bda_kbd_flag1(&mut self, flags: u8) -> Result<(), MachineError> {
        self.write_kbd_bda_u8(BDA_KBD_FLAG1, flags)
    }

    /// Read BDA extended keyboard status (`0040:0018`).
    pub fn bda_kbd_flag2(&self) -> Result<u8, MachineError> {
        self.read_kbd_bda_u8(BDA_KBD_FLAG2)
    }

    /// Write BDA extended keyboard status (`0040:0018`).
    pub fn set_bda_kbd_flag2(&mut self, flags: u8) -> Result<(), MachineError> {
        self.write_kbd_bda_u8(BDA_KBD_FLAG2, flags)
    }

    /// Read BDA keyboard mode/type (`0040:0096`).
    pub fn bda_kbd_mode(&self) -> Result<u8, MachineError> {
        self.read_kbd_bda_u8(BDA_KBD_MODE)
    }

    /// Synthesize INT 16h AH=12h high byte from BDA `40:18` + `40:96`.
    ///
    /// Spec: RBIL INT 16h Table 00588 — not a raw copy of `40:18`.
    pub fn int16_extended_shift_ah(&self) -> Result<u8, MachineError> {
        let f2 = self.bda_kbd_flag2()?;
        let mode = self.bda_kbd_mode().unwrap_or(0);
        let mut ah = 0u8;
        if f2 & KBD_FLAG2_LEFT_CTRL != 0 {
            ah |= 1 << 0;
        }
        if f2 & KBD_FLAG2_LEFT_ALT != 0 {
            ah |= 1 << 1;
        }
        if mode & KBD_MODE_RIGHT_CTRL != 0 {
            ah |= 1 << 2;
        }
        if mode & KBD_MODE_RIGHT_ALT != 0 {
            ah |= 1 << 3;
        }
        if f2 & KBD_FLAG2_SCROLL_DOWN != 0 {
            ah |= 1 << 4;
        }
        if f2 & KBD_FLAG2_NUM_DOWN != 0 {
            ah |= 1 << 5;
        }
        if f2 & KBD_FLAG2_CAPS_DOWN != 0 {
            ah |= 1 << 6;
        }
        if f2 & KBD_FLAG2_SYSREQ != 0 {
            ah |= 1 << 7;
        }
        Ok(ah)
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

    /// Consume one keyboard-sourced 8042 OBF byte and update BDA / typeahead.
    ///
    /// Spec: IBM PC/AT INT 09h path — `IN 60h` clears OBF/IRQ1. This R14 deepen:
    /// - Set-1 **modifier** make/break update `40:17`/`40:18`/`40:96` (and LED
    ///   mirror `40:97` for Caps/Num/Scroll lock toggles) and are **not**
    ///   enqueued as typeahead words.
    /// - Ordinary make codes enqueue `(ASCII, scancode)`; Set-1 breaks for
    ///   non-modifiers are dropped.
    /// - **Ring-full edge:** OBF is still consumed and modifier flags still
    ///   update; a non-modifier make that cannot enqueue is dropped (classic
    ///   BIOS would beep — beep unsupported here).
    /// - `E0` prefix is recorded in `40:96` bit0 for right Ctrl/Alt pairing.
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

        if scancode == 0xE0 {
            let mode = self.bda_kbd_mode().unwrap_or(0) | KBD_MODE_LAST_E0;
            self.write_kbd_bda_u8(BDA_KBD_MODE, mode)?;
            return Ok(None);
        }

        let e0 = self.bda_kbd_mode().unwrap_or(0) & KBD_MODE_LAST_E0 != 0;
        if e0 {
            let mode = self.bda_kbd_mode().unwrap_or(0) & !KBD_MODE_LAST_E0;
            self.write_kbd_bda_u8(BDA_KBD_MODE, mode)?;
        }

        if self.apply_set1_modifier(scancode, e0)? {
            return Ok(None);
        }

        if scancode & 0x80 != 0 {
            // Non-modifier Set-1 break: ignore for the typeahead ring.
            return Ok(None);
        }
        self.ensure_bda_kbd_ring()?;
        let ascii = set1_make_to_ascii(scancode);
        if !self.bda_kbd_enqueue(ascii, scancode)? {
            // Ring full: key dropped after OBF drain (no beep in this stub).
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

    /// Peek the next BDA keystroke without removing it.
    ///
    /// Spec: IBM PC/AT — INT 16h AH=01h reads the word at the current head.
    pub fn bda_kbd_peek(&self) -> Result<Option<(u8, u8)>, MachineError> {
        let head = self.read_bda_kbd_ptr(BDA_KBD_HEAD)?;
        let tail = self.read_bda_kbd_ptr(BDA_KBD_TAIL)?;
        if head == tail {
            return Ok(None);
        }
        let phys = BDA_PHYS_BASE + u64::from(head);
        let ascii = self
            .mem
            .read_u8(phys)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        let scancode = self
            .mem
            .read_u8(phys + 1)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        Ok(Some((ascii, scancode)))
    }

    /// Dequeue one keystroke from the BDA ring (advance head).
    ///
    /// Spec: IBM PC/AT — INT 16h AH=00h removes the word at head and advances
    /// the head pointer, wrapping at `003Eh`.
    pub fn bda_kbd_dequeue(&mut self) -> Result<Option<(u8, u8)>, MachineError> {
        let Some((ascii, scancode)) = self.bda_kbd_peek()? else {
            return Ok(None);
        };
        let head = self.read_bda_kbd_ptr(BDA_KBD_HEAD)?;
        self.write_bda_kbd_ptr(BDA_KBD_HEAD, advance_bda_kbd_off(head))?;
        Ok(Some((ascii, scancode)))
    }

    fn ensure_bda_kbd_ring(&mut self) -> Result<(), MachineError> {
        let head = self.read_bda_kbd_ptr(BDA_KBD_HEAD)?;
        let tail = self.read_bda_kbd_ptr(BDA_KBD_TAIL)?;
        if head == 0 && tail == 0 {
            self.init_bda_kbd_buffer()?;
        }
        Ok(())
    }

    /// Private BDA byte read for keyboard fields (avoids clash with INT10 helper).
    fn read_kbd_bda_u8(&self, phys: u64) -> Result<u8, MachineError> {
        self.mem
            .read_u8(phys)
            .map_err(|_| MachineError::MbrRamTooSmall)
    }

    /// Private BDA byte write for keyboard fields (avoids clash with INT10 helper).
    fn write_kbd_bda_u8(&mut self, phys: u64, value: u8) -> Result<(), MachineError> {
        self.mem
            .write_u8(phys, value)
            .map_err(|_| MachineError::MbrRamTooSmall)
    }

    /// Apply Set-1 modifier / lock toggles to BDA flags. Returns `true` if the
    /// scancode was consumed as a modifier (not a typeahead word).
    ///
    /// Spec: IBM PC/AT INT 09h shift-state updates; RBIL `40:17`/`40:18`/`40:96`.
    fn apply_set1_modifier(&mut self, scancode: u8, e0: bool) -> Result<bool, MachineError> {
        let make = scancode & 0x7F;
        let is_break = scancode & 0x80 != 0;
        let mut f1 = self.bda_kbd_flag1().unwrap_or(0);
        let mut f2 = self.bda_kbd_flag2().unwrap_or(0);
        let mut mode = self.bda_kbd_mode().unwrap_or(0);
        // Preserve enhanced bit if never seeded.
        if mode & KBD_MODE_ENHANCED == 0 && mode == 0 {
            mode = KBD_MODE_ENHANCED;
        }

        let handled = match make {
            0x2A => {
                // Left Shift
                if is_break {
                    f1 &= !KBD_FLAG1_LEFT_SHIFT;
                } else {
                    f1 |= KBD_FLAG1_LEFT_SHIFT;
                }
                true
            }
            0x36 if !e0 => {
                // Right Shift
                if is_break {
                    f1 &= !KBD_FLAG1_RIGHT_SHIFT;
                } else {
                    f1 |= KBD_FLAG1_RIGHT_SHIFT;
                }
                true
            }
            0x1D => {
                // Ctrl (left unless E0 → right)
                if e0 {
                    if is_break {
                        mode &= !KBD_MODE_RIGHT_CTRL;
                    } else {
                        mode |= KBD_MODE_RIGHT_CTRL;
                    }
                } else if is_break {
                    f2 &= !KBD_FLAG2_LEFT_CTRL;
                } else {
                    f2 |= KBD_FLAG2_LEFT_CTRL;
                }
                Self::recompute_ctrl_alt(&mut f1, f2, mode);
                true
            }
            0x38 => {
                // Alt (left unless E0 → right)
                if e0 {
                    if is_break {
                        mode &= !KBD_MODE_RIGHT_ALT;
                    } else {
                        mode |= KBD_MODE_RIGHT_ALT;
                    }
                } else if is_break {
                    f2 &= !KBD_FLAG2_LEFT_ALT;
                } else {
                    f2 |= KBD_FLAG2_LEFT_ALT;
                }
                Self::recompute_ctrl_alt(&mut f1, f2, mode);
                true
            }
            0x3A if !e0 => {
                // Caps Lock — toggle lock on make; track down in flag2
                if is_break {
                    f2 &= !KBD_FLAG2_CAPS_DOWN;
                } else {
                    f2 |= KBD_FLAG2_CAPS_DOWN;
                    f1 ^= KBD_FLAG1_CAPS_LOCK;
                    self.sync_bda_led_mirror(f1)?;
                }
                true
            }
            0x45 if !e0 => {
                // Num Lock
                if is_break {
                    f2 &= !KBD_FLAG2_NUM_DOWN;
                } else {
                    f2 |= KBD_FLAG2_NUM_DOWN;
                    f1 ^= KBD_FLAG1_NUM_LOCK;
                    self.sync_bda_led_mirror(f1)?;
                }
                true
            }
            0x46 if !e0 => {
                // Scroll Lock
                if is_break {
                    f2 &= !KBD_FLAG2_SCROLL_DOWN;
                } else {
                    f2 |= KBD_FLAG2_SCROLL_DOWN;
                    f1 ^= KBD_FLAG1_SCROLL_LOCK;
                    self.sync_bda_led_mirror(f1)?;
                }
                true
            }
            // Insert (0x52) intentionally not handled here: non-E0 0x52 is also
            // keypad '0' under NumLock; full Insert lock+enqueue is out of scope.
            _ => false,
        };

        if handled {
            self.write_kbd_bda_u8(BDA_KBD_FLAG1, f1)?;
            self.write_kbd_bda_u8(BDA_KBD_FLAG2, f2)?;
            self.write_kbd_bda_u8(BDA_KBD_MODE, mode)?;
        }
        Ok(handled)
    }

    fn recompute_ctrl_alt(f1: &mut u8, f2: u8, mode: u8) {
        if f2 & KBD_FLAG2_LEFT_CTRL != 0 || mode & KBD_MODE_RIGHT_CTRL != 0 {
            *f1 |= KBD_FLAG1_CTRL;
        } else {
            *f1 &= !KBD_FLAG1_CTRL;
        }
        if f2 & KBD_FLAG2_LEFT_ALT != 0 || mode & KBD_MODE_RIGHT_ALT != 0 {
            *f1 |= KBD_FLAG1_ALT;
        } else {
            *f1 &= !KBD_FLAG1_ALT;
        }
    }

    /// Mirror Caps/Num/Scroll lock bits into BDA `40:97` and the 8042 LED store.
    ///
    /// Spec: RBIL `0040:0097`; OSDev PS/2 Keyboard Set LEDs mask bits. Host-only
    /// store update — no real LED hardware.
    fn sync_bda_led_mirror(&mut self, f1: u8) -> Result<(), MachineError> {
        let mut leds = 0u8;
        if f1 & KBD_FLAG1_SCROLL_LOCK != 0 {
            leds |= KBD_LED_SCROLL;
        }
        if f1 & KBD_FLAG1_NUM_LOCK != 0 {
            leds |= KBD_LED_NUM;
        }
        if f1 & KBD_FLAG1_CAPS_LOCK != 0 {
            leds |= KBD_LED_CAPS;
        }
        self.write_kbd_bda_u8(BDA_KBD_LEDS, leds)?;
        // Reflect into the keyboard stub LED mask (same bit layout as 0xED).
        self.kbd_set_leds_host(leds);
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

    /// Spec: IBM PC/AT INT 16h path — peek then dequeue advances head.
    #[test]
    fn bda_kbd_peek_and_dequeue() {
        let mut m = Machine::new(64 * 1024);
        assert!(m.bda_kbd_inject_key(b'A', 0x1E).unwrap());
        assert_eq!(m.bda_kbd_peek().unwrap(), Some((b'A', 0x1E)));
        assert_eq!(m.bda_kbd_len().unwrap(), 1);
        assert_eq!(m.bda_kbd_dequeue().unwrap(), Some((b'A', 0x1E)));
        assert_eq!(m.bda_kbd_len().unwrap(), 0);
        assert_eq!(m.bda_kbd_dequeue().unwrap(), None);
    }

    /// Spec: FreeDOS / RBIL — seed equipment + shift/mode/LED BDA keyboard fields.
    /// Honesty: equipment bit0 = floppy; keyboard enabled = bit2.
    #[test]
    fn seed_bda_keyboard_flags_for_freedos() {
        use devices::EQUIP_KEYBOARD_ENABLED;

        let mut m = Machine::new(64 * 1024);
        m.seed_bda_keyboard_flags().unwrap();
        let equip = m.mem.read_u8(crate::guest_boot::BDA_EQUIPMENT).unwrap();
        assert_eq!(equip, m.equipment_byte());
        assert_ne!(equip & EQUIP_KEYBOARD_ENABLED, 0, "keyboard enabled bit2");
        // Bit0 is floppy-installed, not "keyboard present".
        assert_eq!(
            equip & 0x01,
            m.equipment_byte() & 0x01,
            "bit0 tracks floppy media only"
        );
        assert_eq!(m.bda_kbd_flag1().unwrap(), 0);
        assert_eq!(m.bda_kbd_flag2().unwrap(), 0);
        assert_eq!(
            m.bda_kbd_mode().unwrap() & KBD_MODE_ENHANCED,
            KBD_MODE_ENHANCED
        );
        assert_eq!(m.mem.read_u8(BDA_KBD_LEDS).unwrap(), 0);
        assert_eq!(read_word(&m, BDA_KBD_HEAD), BDA_KBD_BUF_START_OFF);
    }

    /// Spec: IBM PC/AT INT 09h — Left Shift make/break updates `40:17`.
    #[test]
    fn irq1_shift_updates_bda_flag1() {
        let mut m = Machine::new(64 * 1024);
        enable_kbd_irq1(&mut m);
        m.seed_bda_keyboard_flags().unwrap();

        // Place raw Set-1 Left Shift make (0x2A) without translation.
        assert!(m.kbd.place_output(0x2A));
        m.sync_kbd_irq1();
        assert!(m.service_kbd_irq1_bda().unwrap().is_none());
        assert_eq!(
            m.bda_kbd_flag1().unwrap() & KBD_FLAG1_LEFT_SHIFT,
            KBD_FLAG1_LEFT_SHIFT
        );
        assert_eq!(m.bda_kbd_len().unwrap(), 0);

        assert!(m.kbd.place_output(0xAA)); // break
        m.sync_kbd_irq1();
        assert!(m.service_kbd_irq1_bda().unwrap().is_none());
        assert_eq!(m.bda_kbd_flag1().unwrap() & KBD_FLAG1_LEFT_SHIFT, 0);
    }

    /// Spec: Caps Lock make toggles lock bit and mirrors LEDs (`40:97` + 8042 store).
    #[test]
    fn irq1_caps_lock_toggles_and_mirrors_leds() {
        let mut m = Machine::new(64 * 1024);
        enable_kbd_irq1(&mut m);
        m.seed_bda_keyboard_flags().unwrap();

        assert!(m.kbd.place_output(0x3A)); // Caps make
        m.sync_kbd_irq1();
        assert!(m.service_kbd_irq1_bda().unwrap().is_none());
        assert_eq!(
            m.bda_kbd_flag1().unwrap() & KBD_FLAG1_CAPS_LOCK,
            KBD_FLAG1_CAPS_LOCK
        );
        assert_eq!(
            m.mem.read_u8(BDA_KBD_LEDS).unwrap() & KBD_LED_CAPS,
            KBD_LED_CAPS
        );
        assert_eq!(m.kbd.kbd_leds() & KBD_LED_CAPS, KBD_LED_CAPS);
    }

    /// Spec: ring-full edge — OBF still drains; modifier flags still update.
    #[test]
    fn irq1_ring_full_still_updates_shift_flags() {
        let mut m = Machine::new(64 * 1024);
        enable_kbd_irq1(&mut m);
        m.init_bda_kbd_buffer().unwrap();
        for i in 0..BDA_KBD_CAPACITY {
            assert!(m.bda_kbd_enqueue(i as u8, 0x10).unwrap());
        }
        assert_eq!(m.bda_kbd_len().unwrap(), BDA_KBD_CAPACITY);

        // Non-modifier make while full: OBF consumed, nothing enqueued.
        assert!(m.kbd.place_output(0x1E)); // 'A' make
        m.sync_kbd_irq1();
        assert!(m.service_kbd_irq1_bda().unwrap().is_none());
        assert_eq!(m.bda_kbd_len().unwrap(), BDA_KBD_CAPACITY);

        // Modifier still updates flags while full.
        assert!(m.kbd.place_output(0x2A));
        m.sync_kbd_irq1();
        assert!(m.service_kbd_irq1_bda().unwrap().is_none());
        assert_eq!(
            m.bda_kbd_flag1().unwrap() & KBD_FLAG1_LEFT_SHIFT,
            KBD_FLAG1_LEFT_SHIFT
        );
    }

    /// Spec: E0 + Right Ctrl make sets `40:96` right-Ctrl and `40:17` Ctrl.
    #[test]
    fn irq1_e0_right_ctrl_sets_mode_and_flag1() {
        let mut m = Machine::new(64 * 1024);
        enable_kbd_irq1(&mut m);
        m.seed_bda_keyboard_flags().unwrap();

        assert!(m.kbd.place_output(0xE0));
        m.sync_kbd_irq1();
        assert!(m.service_kbd_irq1_bda().unwrap().is_none());

        assert!(m.kbd.place_output(0x1D)); // Right Ctrl make after E0
        m.sync_kbd_irq1();
        assert!(m.service_kbd_irq1_bda().unwrap().is_none());
        assert_eq!(
            m.bda_kbd_mode().unwrap() & KBD_MODE_RIGHT_CTRL,
            KBD_MODE_RIGHT_CTRL
        );
        assert_eq!(m.bda_kbd_flag1().unwrap() & KBD_FLAG1_CTRL, KBD_FLAG1_CTRL);
        assert_eq!(m.int16_extended_shift_ah().unwrap() & (1 << 2), 1 << 2);
    }

    /// Spec: after Shift make, ordinary key still reaches INT16 AH=00/01 via BDA.
    #[test]
    fn irq1_modifiers_coherent_with_int16_get() {
        let mut m = Machine::new(64 * 1024);
        enable_kbd_irq1(&mut m);
        m.seed_bda_keyboard_flags().unwrap();

        assert!(m.kbd.place_output(0x2A)); // Shift make
        m.sync_kbd_irq1();
        assert!(m.service_kbd_irq1_bda().unwrap().is_none());
        assert!(m.kbd.place_output(0x1E)); // 'A'
        m.sync_kbd_irq1();
        assert_eq!(m.service_kbd_irq1_bda().unwrap(), Some((b'a', 0x1E)));

        m.cpu.set_ah(crate::INT16_AH_CHECK_KEYSTROKE);
        m.service_int16();
        assert_eq!(m.cpu.ax(), 0x1E61);
        m.cpu.set_ah(crate::INT16_AH_GET_KEYSTROKE);
        m.service_int16();
        assert_eq!(m.cpu.ax(), 0x1E61);
        assert_eq!(m.bda_kbd_len().unwrap(), 0);
        assert_eq!(
            m.bda_kbd_flag1().unwrap() & KBD_FLAG1_LEFT_SHIFT,
            KBD_FLAG1_LEFT_SHIFT
        );
    }
}
