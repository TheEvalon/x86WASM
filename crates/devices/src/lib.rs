//! Device models. Milestone 1: COM1 data port + debug port 0x402.
//! Milestone 2 (partial): 8259 PIC ICW+OCW/IRQ; 8254 PIT channel-0; CMOS/RTC 0x70/0x71.

#![forbid(unsafe_code)]

mod cmos;
mod pic;
mod pit;
mod serial;

pub use cmos::{
    CmosRtc, CMOS_DATA, CMOS_INDEX, REG_STATUS_A, REG_STATUS_B, REG_STATUS_C, REG_STATUS_D,
};
pub use pic::{DualPic, Pic8259, PIC_MASTER_CMD, PIC_MASTER_DATA, PIC_SLAVE_CMD, PIC_SLAVE_DATA};
pub use pit::{Pit8254, PitChannel, PIT_CH0_DATA, PIT_CH1_DATA, PIT_CH2_DATA, PIT_CONTROL};
pub use serial::{DebugConsole, Serial16550, SerialOutput};

/// Port I/O sink shared by CLI and browser.
pub trait PortDevice {
    fn port_read(&mut self, port: u16, size: u8) -> u32;
    fn port_write(&mut self, port: u16, size: u8, value: u32);
}
