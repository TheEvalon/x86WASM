//! Device models. Milestone 1: COM1 data port + debug port 0x402.

#![forbid(unsafe_code)]

mod serial;

pub use serial::{DebugConsole, Serial16550, SerialOutput};

/// Port I/O sink shared by CLI and browser.
pub trait PortDevice {
    fn port_read(&mut self, port: u16, size: u8) -> u32;
    fn port_write(&mut self, port: u16, size: u8, value: u32);
}
