//! COM1 (`0x3F8`), COM2 (`0x2F8`), and Bochs/QEMU-style debug port `0x402`.
//!
//! 16550 programming model is intentionally minimal (M1/M2 debug UART): writes
//! to THR (when DLAB=0) and writes to `0x402` append bytes to a per-port sink.
//! IER/IIR expose the transmitter-holding-register-empty interrupt; routing that
//! signal to the PIC remains a machine concern.
//! Spec: NS16550A / classic PC COM1–COM2 I/O map (THR/RBR/IER/IIR/LSR subset).

use crate::PortDevice;

const IER_THRE: u8 = 1 << 1;
const IIR_NO_INTERRUPT: u8 = 0x01;
const IIR_THRE: u8 = 0x02;

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

/// Very small 16550 subset at a classic COM base (`0x3F8` COM1 / `0x2F8` COM2).
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
    /// THRE interrupt latch (NS16550A IER/IIR/THR behavior).
    thre_interrupt_pending: bool,
}

impl Default for Serial16550 {
    fn default() -> Self {
        Self::new(0x3F8)
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

    /// Current device-level interrupt signal; external COM IRQ routing is out of scope.
    pub fn irq_line(&self) -> bool {
        self.ier & IER_THRE != 0 && self.thre_interrupt_pending
    }

    fn dlab(&self) -> bool {
        self.lcr & 0x80 != 0
    }

    fn owns(&self, port: u16) -> bool {
        (self.base..self.base.saturating_add(8)).contains(&port)
    }

    fn read_iir(&mut self) -> u8 {
        if self.irq_line() {
            self.thre_interrupt_pending = false;
            IIR_THRE
        } else {
            IIR_NO_INTERRUPT
        }
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
            0 => 0, // RHR empty
            1 => self.ier,
            2 => self.read_iir(),
            3 => self.lcr,
            4 => self.mcr,
            5 => 0x60, // LSR: THR empty + transmitter empty
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
    }

    /// Spec: NS16550A LSR offset +5 — THR empty (bit5) + transmitter empty (bit6).
    #[test]
    fn com2_lsr_reports_thr_empty() {
        let mut s = Serial16550::new(0x2F8);
        assert_eq!(s.port_read(0x2FD, 1) & 0x60, 0x60);
        // RBR empty (offset 0) — enough for polling OUT loops.
        assert_eq!(s.port_read(0x2F8, 1), 0);
    }

    /// Spec: NS16550A "Interrupt Enable Register" bit 1 and
    /// "Interrupt Identification Register" bit 0.
    #[test]
    fn thre_interrupt_reset_and_ier_gating() {
        let mut s = Serial16550::new(0x3F8);

        assert_eq!(s.port_read(0x3F9, 1), 0);
        assert_eq!(s.port_read(0x3FA, 1), 0x01);
        assert!(!s.irq_line());

        // Other IER sources remain inert in this TX-only subset.
        s.port_write(0x3F9, 1, 0x0D);
        assert_eq!(s.port_read(0x3F9, 1), 0x0D);
        assert!(!s.irq_line());
        assert_eq!(s.port_read(0x3FA, 1), 0x01);

        s.port_write(0x3F9, 1, 0x0F);
        assert!(s.irq_line());

        s.port_write(0x3F9, 1, 0x0D);
        assert!(!s.irq_line());
        assert_eq!(s.port_read(0x3FA, 1), 0x01);

        // Re-enabling THRE while LSR.THRE is set requests it again.
        s.port_write(0x3F9, 1, 0x0F);
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
    /// only in base address (and external IRQ routing, which is out of scope).
    #[test]
    fn com1_and_com2_thre_interrupt_behavior_matches() {
        assert_eq!(thre_interrupt_trace(0x3F8), thre_interrupt_trace(0x2F8));
    }
}
