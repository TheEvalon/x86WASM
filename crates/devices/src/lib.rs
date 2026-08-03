//! Device models. Milestone 1: COM1 data port + debug port 0x402.
//! Milestone 2 (partial): 8259 PIC ICW1–ICW4 initialization.

#![forbid(unsafe_code)]

mod pic;
mod serial;

pub use pic::{DualPic, Pic8259, PIC_MASTER_CMD, PIC_MASTER_DATA, PIC_SLAVE_CMD, PIC_SLAVE_DATA};
pub use serial::{DebugConsole, Serial16550, SerialOutput};

/// Port I/O sink shared by CLI and browser.
pub trait PortDevice {
    fn port_read(&mut self, port: u16, size: u8) -> u32;
    fn port_write(&mut self, port: u16, size: u8, value: u32);
}
