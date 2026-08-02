//! Default port I/O for unimplemented ports.

use devices::PortDevice;

#[derive(Default)]
pub struct PortBus;

impl PortBus {
    pub fn new() -> Self {
        Self
    }
}

impl PortDevice for PortBus {
    fn port_read(&mut self, _port: u16, size: u8) -> u32 {
        match size {
            1 => 0xFF,
            2 => 0xFFFF,
            _ => 0xFFFF_FFFF,
        }
    }

    fn port_write(&mut self, _port: u16, _size: u8, _value: u32) {}
}
