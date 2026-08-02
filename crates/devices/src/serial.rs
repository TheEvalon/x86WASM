//! COM1 (0x3F8) and Bochs/QEMU-style debug port 0x402.
//!
//! 16550 programming model is intentionally minimal for M1: writes to the
//! COM1 THR (when DLAB=0) and writes to 0x402 append bytes to a shared sink.

use crate::PortDevice;

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

/// Very small 16550 subset at base `0x3F8`.
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
        }
    }

    pub fn output(&self) -> &SerialOutput {
        &self.output
    }

    pub fn output_mut(&mut self) -> &mut SerialOutput {
        &mut self.output
    }

    fn dlab(&self) -> bool {
        self.lcr & 0x80 != 0
    }

    fn owns(&self, port: u16) -> bool {
        (self.base..self.base.saturating_add(8)).contains(&port)
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
            2 => 0x01, // IIR: no interrupt pending
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
            0 => self.output.push(v),
            1 => self.ier = v,
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
}
