//! Device models. Milestone 1: COM1 data port + debug port 0x402.
//! Milestone 2 (partial): CMOS/RTC register bank at 0x70/0x71.

#![forbid(unsafe_code)]

mod cmos;
mod serial;

pub use cmos::{
    CmosRtc, CMOS_DATA, CMOS_INDEX, REG_STATUS_A, REG_STATUS_B, REG_STATUS_C, REG_STATUS_D,
};
pub use serial::{DebugConsole, Serial16550, SerialOutput};

/// Port I/O sink shared by CLI and browser.
pub trait PortDevice {
    fn port_read(&mut self, port: u16, size: u8) -> u32;
    fn port_write(&mut self, port: u16, size: u8, value: u32);
}
