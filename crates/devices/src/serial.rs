//! COM1 (`0x3F8`), COM2 (`0x2F8`), COM3 (`0x3E8`), COM4 (`0x2E8`), and Bochs/QEMU-style
//! debug port `0x402`.
//!
//! 16550 programming model (M1/M2 debug UART + bounded RX):
//! - THR (DLAB=0) appends to a per-port TX sink.
//! - Host [`Serial16550::push_rx`] feeds RBR; LSR.DR and IER.ERBFI / IIR RDA
//!   (`100b`) drive [`Serial16550::irq_line`] with priority over THRE.
//! - Machine routes COM1 → IRQ4 and COM2 → IRQ3. COM3/COM4 expose the same
//!   register file for POST probes but do **not** share ISA IRQ4/IRQ3 in this
//!   slice (shared-IRQ honesty deferred).
//!
//! Spec: NS16550A / classic PC COM1–COM4 I/O map (THR/RBR/IER/IIR/LSR subset).

use crate::PortDevice;

/// Classic COM1 base (IRQ4 on a real PC/AT).
pub const COM1_BASE: u16 = 0x3F8;
/// Classic COM2 base (IRQ3).
pub const COM2_BASE: u16 = 0x2F8;
/// Classic COM3 base (historically IRQ4 shared; IRQ not wired here).
pub const COM3_BASE: u16 = 0x3E8;
/// Classic COM4 base (historically IRQ3 shared; IRQ not wired here).
pub const COM4_BASE: u16 = 0x2E8;

/// Width of a classic 16550 I/O window (THR…SCR).
pub const COM_IO_SPAN: u16 = 8;

const IER_RDA: u8 = 1 << 0;
const IER_THRE: u8 = 1 << 1;
const IIR_NO_INTERRUPT: u8 = 0x01;
const IIR_THRE: u8 = 0x02;
const IIR_RDA: u8 = 0x04;
const LSR_DR: u8 = 1 << 0;
const LSR_THRE_TEMT: u8 = 0x60;

/// Bytes emitted by guest serial/debug writes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SerialOutput {
    bytes: Vec<u8>,
}

impl SerialOutput {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, b: u8) {
        self.bytes.push(b);
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn as_str_lossy(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }

    pub fn clear(&mut self) {
        self.bytes.clear();
    }
}

/// Very small 16550 subset at a classic COM base (`0x3F8`/`0x2F8`/`0x3E8`/`0x2E8`).
#[derive(Clone, Debug)]
pub struct Serial16550 {
    pub base: u16,
    /// Line control (bit7 = DLAB).
    pub lcr: u8,
    pub ier: u8,
    pub mcr: u8,
    pub scratch: u8,
    /// Divisor latch (when DLAB=1).
    pub divisor: u16,
    output: SerialOutput,
    /// Single-byte receive holding register (no FIFO in this slice).
    rx_data: Option<u8>,
    /// THRE interrupt latch (NS16550A IER/IIR/THR behavior).
    thre_interrupt_pending: bool,
}

impl Default for Serial16550 {
    fn default() -> Self {
        Self::new(COM1_BASE)
    }
}

impl Serial16550 {
    pub fn new(base: u16) -> Self {
        Self {
            base,
            lcr: 0,
            ier: 0,
            mcr: 0,
            scratch: 0,
            divisor: 1,
            output: SerialOutput::new(),
            rx_data: None,
            // Reset leaves THR empty. IER reset keeps the external line low.
            thre_interrupt_pending: true,
        }
    }

    pub fn output(&self) -> &SerialOutput {
        &self.output
    }

    pub fn output_mut(&mut self) -> &mut SerialOutput {
        &mut self.output
    }

    /// Host injects one received byte into RBR (overwrites unread data).
    ///
    /// Spec: NS16550A receiver holding register — sets LSR.DR; with IER.ERBFI
    /// asserted this raises the device IRQ line (IIR RDA `100b`).
    pub fn push_rx(&mut self, byte: u8) {
        self.rx_data = Some(byte);
    }

    /// True when RBR holds unread data (LSR.DR).
    pub fn rx_pending(&self) -> bool {
        self.rx_data.is_some()
    }

    /// Current device-level interrupt signal (RDA and/or THRE).
    ///
    /// Spec: NS16550A IER bit0 (ERBFI) gates received-data-available; bit1
    /// (ETBEI) gates THRE. The host wires this to ISA IRQ4 (COM1) or IRQ3 (COM2).
    pub fn irq_line(&self) -> bool {
        self.rda_irq_active() || self.thre_irq_active()
    }

    fn rda_irq_active(&self) -> bool {
        self.ier & IER_RDA != 0 && self.rx_data.is_some()
    }

    fn thre_irq_active(&self) -> bool {
        self.ier & IER_THRE != 0 && self.thre_interrupt_pending
    }

    fn dlab(&self) -> bool {
        self.lcr & 0x80 != 0
    }

    fn owns(&self, port: u16) -> bool {
        (self.base..self.base.saturating_add(COM_IO_SPAN)).contains(&port)
    }

    /// Whether `port` falls in any classic COM1–COM4 window.
    pub fn owns_classic_com(port: u16) -> bool {
        (COM1_BASE..COM1_BASE + COM_IO_SPAN).contains(&port)
            || (COM2_BASE..COM2_BASE + COM_IO_SPAN).contains(&port)
            || (COM3_BASE..COM3_BASE + COM_IO_SPAN).contains(&port)
            || (COM4_BASE..COM4_BASE + COM_IO_SPAN).contains(&port)
    }

    fn lsr(&self) -> u8 {
        let mut v = LSR_THRE_TEMT;
        if self.rx_data.is_some() {
            v |= LSR_DR;
        }
        v
    }

    /// Spec: NS16550A IIR priority — RDA (`100b`) above THRE (`010b`).
    /// Reading IIR clears only the THRE condition when THRE is reported.
    fn read_iir(&mut self) -> u8 {
        if self.rda_irq_active() {
            IIR_RDA
        } else if self.thre_irq_active() {
            self.thre_interrupt_pending = false;
            IIR_THRE
        } else {
            IIR_NO_INTERRUPT
        }
    }

    fn read_rbr(&mut self) -> u8 {
        self.rx_data.take().unwrap_or(0)
    }

    fn write_ier(&mut self, value: u8) {
        let thre_was_enabled = self.ier & IER_THRE != 0;
        self.ier = value;

        // Enabling THRE while LSR.THRE is set requests an interrupt.
        if !thre_was_enabled && value & IER_THRE != 0 {
            self.thre_interrupt_pending = true;
        }
    }

    fn write_thr(&mut self, value: u8) {
        // A THR write clears THRE. This debug sink consumes the byte
        // synchronously, so THR becomes empty and requests THRE again.
        self.thre_interrupt_pending = false;
        self.output.push(value);
        self.thre_interrupt_pending = true;
    }
}

impl PortDevice for Serial16550 {
    fn port_read(&mut self, port: u16, _size: u8) -> u32 {
        if !self.owns(port) {
            return 0xFFFFFFFF;
        }
        let off = port - self.base;
        let v = match off {
            0 if self.dlab() => (self.divisor & 0xFF) as u8,
            1 if self.dlab() => (self.divisor >> 8) as u8,
            0 => self.read_rbr(),
            1 => self.ier,
            2 => self.read_iir(),
            3 => self.lcr,
            4 => self.mcr,
            5 => self.lsr(),
            6 => 0x10, // MSR: DSR
            7 => self.scratch,
            _ => 0xFF,
        };
        u32::from(v)
    }

    fn port_write(&mut self, port: u16, _size: u8, value: u32) {
        if !self.owns(port) {
            return;
        }
        let off = port - self.base;
        let v = value as u8;
        match off {
            0 if self.dlab() => {
                self.divisor = (self.divisor & 0xFF00) | u16::from(v);
            }
            1 if self.dlab() => {
                self.divisor = (self.divisor & 0x00FF) | (u16::from(v) << 8);
            }
            0 => self.write_thr(v),
            1 => self.write_ier(v),
            3 => self.lcr = v,
            4 => self.mcr = v,
            7 => self.scratch = v,
            _ => {}
        }
    }
}

/// Write-only debug console at port `0x402`.
#[derive(Clone, Debug, Default)]
pub struct DebugConsole {
    output: SerialOutput,
}

impl DebugConsole {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn output(&self) -> &SerialOutput {
        &self.output
    }

    pub fn output_mut(&mut self) -> &mut SerialOutput {
        &mut self.output
    }
}

impl PortDevice for DebugConsole {
    fn port_read(&mut self, _port: u16, _size: u8) -> u32 {
        0xFFFFFFFF
    }

    fn port_write(&mut self, port: u16, _size: u8, value: u32) {
        if port == 0x402 {
            self.output.push(value as u8);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn com1_thr_emits_byte() {
        let mut s = Serial16550::new(0x3F8);
        s.port_write(0x3F8, 1, u32::from(b'H'));
        assert_eq!(s.output().as_bytes(), b"H");
    }

    /// Spec: NS16550A THR at COM2 base `0x2F8` (DLAB=0) — same debug-UART model as COM1.
    #[test]
    fn com2_thr_emits_byte() {
        let mut s = Serial16550::new(0x2F8);
        s.port_write(0x2F8, 1, u32::from(b'C'));
        assert_eq!(s.output().as_bytes(), b"C");
        // COM1 range must not be owned by a COM2 instance.
        s.port_write(0x3F8, 1, u32::from(b'X'));
        assert_eq!(s.output().as_bytes(), b"C");
    }

    #[test]
    fn debug_port_emits_byte() {
        let mut d = DebugConsole::new();
        d.port_write(0x402, 1, u32::from(b'E'));
        assert_eq!(d.output().as_bytes(), b"E");
    }

    #[test]
    fn lsr_reports_thr_empty() {
        let mut s = Serial16550::new(0x3F8);
        assert_eq!(s.port_read(0x3FD, 1) & 0x60, 0x60);
        assert_eq!(s.port_read(0x3FD, 1) & u32::from(LSR_DR), 0);
    }

    /// Spec: NS16550A LSR offset +5 — THR empty (bit5) + transmitter empty (bit6).
    #[test]
    fn com2_lsr_reports_thr_empty() {
        let mut s = Serial16550::new(0x2F8);
        assert_eq!(s.port_read(0x2FD, 1) & 0x60, 0x60);
        // RBR empty (offset 0) — enough for polling OUT loops.
        assert_eq!(s.port_read(0x2F8, 1), 0);
    }

    /// Spec: NS16550A RBR / LSR.DR — host `push_rx` sets data ready; RBR read clears.
    #[test]
    fn push_rx_sets_lsr_dr_and_rbr() {
        let mut s = Serial16550::new(0x3F8);
        s.push_rx(b'R');
        assert!(s.rx_pending());
        assert_eq!(s.port_read(0x3FD, 1) & u32::from(LSR_DR), u32::from(LSR_DR));
        assert_eq!(s.port_read(0x3F8, 1), u32::from(b'R'));
        assert!(!s.rx_pending());
        assert_eq!(s.port_read(0x3FD, 1) & u32::from(LSR_DR), 0);
        assert_eq!(s.port_read(0x3F8, 1), 0);
    }

    /// Spec: NS16550A IER bit0 (ERBFI) + IIR RDA ID `100b`.
    #[test]
    fn rda_interrupt_ier_gating_and_iir() {
        let mut s = Serial16550::new(0x3F8);
        s.push_rx(b'A');
        assert!(!s.irq_line());
        assert_eq!(s.port_read(0x3FA, 1), u32::from(IIR_NO_INTERRUPT));

        s.port_write(0x3F9, 1, u32::from(IER_RDA));
        assert!(s.irq_line());
        assert_eq!(s.port_read(0x3FA, 1), u32::from(IIR_RDA));
        // Reading IIR does not clear RDA.
        assert!(s.irq_line());
        assert_eq!(s.port_read(0x3F8, 1), u32::from(b'A'));
        assert!(!s.irq_line());
        assert_eq!(s.port_read(0x3FA, 1), u32::from(IIR_NO_INTERRUPT));
    }

    /// Spec: NS16550A interrupt priority — RDA reported ahead of THRE.
    #[test]
    fn rda_has_priority_over_thre_in_iir() {
        let mut s = Serial16550::new(0x3F8);
        s.port_write(0x3F9, 1, u32::from(IER_RDA | IER_THRE));
        assert!(s.irq_line());
        assert_eq!(s.port_read(0x3FA, 1), u32::from(IIR_THRE)); // THRE only so far
        s.push_rx(b'P');
        assert_eq!(s.port_read(0x3FA, 1), u32::from(IIR_RDA));
        let _ = s.port_read(0x3F8, 1);
        // After RBR read, THRE may still be pending (cleared by prior IIR read).
        // Re-enable THRE edge by disabling/enabling ETBEI.
        s.port_write(0x3F9, 1, 0);
        s.port_write(0x3F9, 1, u32::from(IER_THRE));
        assert_eq!(s.port_read(0x3FA, 1), u32::from(IIR_THRE));
    }

    /// Spec: COM2 RX path is base-relative (same as COM1).
    #[test]
    fn com2_push_rx_rda_irq() {
        let mut s = Serial16550::new(0x2F8);
        s.push_rx(b'2');
        s.port_write(0x2F9, 1, u32::from(IER_RDA));
        assert!(s.irq_line());
        assert_eq!(s.port_read(0x2FA, 1), u32::from(IIR_RDA));
        assert_eq!(s.port_read(0x2F8, 1), u32::from(b'2'));
        assert!(!s.irq_line());
    }

    /// Spec: NS16550A "Interrupt Enable Register" bit 1 and
    /// "Interrupt Identification Register" bit 0.
    #[test]
    fn thre_interrupt_reset_and_ier_gating() {
        let mut s = Serial16550::new(0x3F8);

        assert_eq!(s.port_read(0x3F9, 1), 0);
        assert_eq!(s.port_read(0x3FA, 1), 0x01);
        assert!(!s.irq_line());

        // ELSR/EDSSI without ERBFI/THRE remain inert when RX empty.
        s.port_write(0x3F9, 1, 0x0C);
        assert_eq!(s.port_read(0x3F9, 1), 0x0C);
        assert!(!s.irq_line());
        assert_eq!(s.port_read(0x3FA, 1), 0x01);

        s.port_write(0x3F9, 1, 0x0E); // THRE + ELSR/EDSSI
        assert!(s.irq_line());

        s.port_write(0x3F9, 1, 0x0C);
        assert!(!s.irq_line());
        assert_eq!(s.port_read(0x3FA, 1), 0x01);

        // Re-enabling THRE while LSR.THRE is set requests it again.
        s.port_write(0x3F9, 1, 0x0E);
        assert!(s.irq_line());
    }

    /// Spec: NS16550A IIR interrupt ID `010b` is THRE; reading IIR while
    /// THRE is the reported source clears that interrupt.
    #[test]
    fn iir_read_reports_and_clears_thre_interrupt() {
        let mut s = Serial16550::new(0x3F8);
        s.port_write(0x3F9, 1, 0x02);

        assert!(s.irq_line());
        assert_eq!(s.port_read(0x3FA, 1), 0x02);
        assert!(!s.irq_line());
        assert_eq!(s.port_read(0x3FA, 1), 0x01);
    }

    /// Spec: NS16550A THRE interrupt is cleared by a THR write and set again
    /// when THR becomes empty. This sink drains THR synchronously.
    #[test]
    fn thr_write_reasserts_thre_after_synchronous_drain() {
        let mut s = Serial16550::new(0x3F8);
        s.port_write(0x3F9, 1, 0x02);
        assert_eq!(s.port_read(0x3FA, 1), 0x02);
        assert!(!s.irq_line());

        s.port_write(0x3F8, 1, u32::from(b'T'));

        assert_eq!(s.output().as_bytes(), b"T");
        assert!(s.irq_line());
        assert_eq!(s.port_read(0x3FA, 1), 0x02);
        assert!(!s.irq_line());
    }

    fn thre_interrupt_trace(base: u16) -> (u32, bool, u32, bool, bool, u32, Vec<u8>) {
        let mut s = Serial16550::new(base);
        let initial_iir = s.port_read(base + 2, 1);
        s.port_write(base + 1, 1, 0x02);
        let enabled_irq = s.irq_line();
        let first_iir = s.port_read(base + 2, 1);
        let cleared_irq = s.irq_line();
        s.port_write(base, 1, u32::from(b'P'));
        let reasserted_irq = s.irq_line();
        let reasserted_iir = s.port_read(base + 2, 1);
        (
            initial_iir,
            enabled_irq,
            first_iir,
            cleared_irq,
            reasserted_irq,
            reasserted_iir,
            s.output().as_bytes().to_vec(),
        )
    }

    /// Spec: NS16550A register behavior is base-relative; COM1 and COM2 differ
    /// only in base address (and external IRQ routing).
    #[test]
    fn com1_and_com2_thre_interrupt_behavior_matches() {
        assert_eq!(thre_interrupt_trace(0x3F8), thre_interrupt_trace(0x2F8));
    }

    /// Spec: classic PC COM3 `0x3E8` / COM4 `0x2E8` — same 16550 window as COM1/2.
    /// SeaBIOS POST historically probed IER at `0x3E9` / `0x2E9` (base+1).
    #[test]
    fn com3_com4_ier_and_lsr_probe_sites() {
        let mut c3 = Serial16550::new(COM3_BASE);
        let mut c4 = Serial16550::new(COM4_BASE);
        assert_eq!(c3.port_read(COM3_BASE + 1, 1), 0); // IER
        assert_eq!(c4.port_read(COM4_BASE + 1, 1), 0);
        assert_eq!(c3.port_read(COM3_BASE + 5, 1) & 0x60, 0x60); // LSR THRE|TEMT
        assert_eq!(c4.port_read(COM4_BASE + 5, 1) & 0x60, 0x60);
        c3.port_write(COM3_BASE, 1, u32::from(b'3'));
        c4.port_write(COM4_BASE, 1, u32::from(b'4'));
        assert_eq!(c3.output().as_bytes(), b"3");
        assert_eq!(c4.output().as_bytes(), b"4");
        assert!(Serial16550::owns_classic_com(0x3E9));
        assert!(Serial16550::owns_classic_com(0x2E9));
        assert!(!Serial16550::owns_classic_com(0x3E7));
        assert!(!Serial16550::owns_classic_com(0x2F0));
    }
}
