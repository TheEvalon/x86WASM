//! Classic PC machine for Milestone 1 (CPU lab + serial HELLO ROM).

#![forbid(unsafe_code)]

mod hello_rom;
mod mem;
mod ports;

pub use hello_rom::{build_hello_rom, EXPECTED_HELLO};
pub use mem::PhysMem;

use devices::{DebugConsole, PortDevice, Serial16550};
use firmware_interface::RomImage;
use ports::PortBus;
use thiserror::Error;
use x86_core::CpuState;
use x86_interpreter::{run, step, Bus, ExecError};

#[derive(Debug, Error)]
pub enum MachineError {
    #[error(transparent)]
    Exec(#[from] ExecError),
    #[error("ROM too large for window")]
    RomTooLarge,
}

pub struct Machine {
    pub cpu: CpuState,
    pub mem: PhysMem,
    pub com1: Serial16550,
    pub debug: DebugConsole,
    ports: PortBus,
}

impl Machine {
    pub fn new(ram_size: usize) -> Self {
        Self {
            cpu: CpuState::reset(),
            mem: PhysMem::new(ram_size),
            com1: Serial16550::new(0x3F8),
            debug: DebugConsole::new(),
            ports: PortBus::new(),
        }
    }

    /// Load a 64 KiB (or smaller) ROM at `0xFFFF_0000` for the Intel reset vector.
    pub fn load_rom(&mut self, data: &[u8]) -> Result<(), MachineError> {
        if data.len() > 64 * 1024 {
            return Err(MachineError::RomTooLarge);
        }
        let mut rom = vec![0u8; 64 * 1024];
        if data.len() == 64 * 1024 {
            rom.copy_from_slice(data);
        } else {
            // Small images start at ROM offset 0; caller must include a reset
            // vector at 0xFFF0 when using a full 64 KiB buffer (HELLO does).
            rom[..data.len()].copy_from_slice(data);
        }
        self.mem.map_rom(0xFFFF_0000, rom);
        Ok(())
    }

    pub fn load_rom_image(&mut self, image: &RomImage) -> Result<(), MachineError> {
        self.mem.map_rom(image.phys_base, image.data.clone());
        Ok(())
    }

    pub fn reset(&mut self) {
        self.cpu = CpuState::reset();
        self.com1 = Serial16550::new(0x3F8);
        self.debug = DebugConsole::new();
    }

    pub fn step(&mut self) -> Result<(), MachineError> {
        let mut view = MachineBus {
            mem: &mut self.mem,
            com1: &mut self.com1,
            debug: &mut self.debug,
            ports: &mut self.ports,
        };
        step(&mut self.cpu, &mut view)?;
        Ok(())
    }

    pub fn run(&mut self, max_steps: u64) -> Result<u64, MachineError> {
        let mut view = MachineBus {
            mem: &mut self.mem,
            com1: &mut self.com1,
            debug: &mut self.debug,
            ports: &mut self.ports,
        };
        Ok(run(&mut self.cpu, &mut view, max_steps)?)
    }

    /// Combined guest console (COM1 then debug port bytes are tracked separately).
    pub fn com1_text(&self) -> String {
        self.com1.output().as_str_lossy()
    }

    pub fn debug_text(&self) -> String {
        self.debug.output().as_str_lossy()
    }

    pub fn load_hello_rom(&mut self) -> Result<(), MachineError> {
        let rom = build_hello_rom();
        self.load_rom(&rom)
    }
}

struct MachineBus<'a> {
    mem: &'a mut PhysMem,
    com1: &'a mut Serial16550,
    debug: &'a mut DebugConsole,
    ports: &'a mut PortBus,
}

impl Bus for MachineBus<'_> {
    fn read_u8(&mut self, addr: u64) -> Result<u8, ExecError> {
        self.mem
            .read_u8(addr)
            .map_err(|_| ExecError::MemoryFault(addr))
    }

    fn write_u8(&mut self, addr: u64, val: u8) -> Result<(), ExecError> {
        self.mem
            .write_u8(addr, val)
            .map_err(|_| ExecError::MemoryFault(addr))
    }

    fn port_in_u8(&mut self, port: u16) -> Result<u8, ExecError> {
        if (0x3F8..0x400).contains(&port) {
            return Ok(self.com1.port_read(port, 1) as u8);
        }
        if port == 0x402 {
            return Ok(self.debug.port_read(port, 1) as u8);
        }
        Ok(self.ports.port_read(port, 1) as u8)
    }

    fn port_out_u8(&mut self, port: u16, val: u8) -> Result<(), ExecError> {
        if (0x3F8..0x400).contains(&port) {
            self.com1.port_write(port, 1, u32::from(val));
            return Ok(());
        }
        if port == 0x402 {
            self.debug.port_write(port, 1, u32::from(val));
            return Ok(());
        }
        self.ports.port_write(port, 1, u32::from(val));
        Ok(())
    }

    fn port_in_u16(&mut self, port: u16) -> Result<u16, ExecError> {
        // Spec: Intel SDM Vol. 2 INS/OUTS/IN/OUT — I/O address in DX, size = operand size.
        if (0x3F8..0x400).contains(&port) {
            return Ok(self.com1.port_read(port, 2) as u16);
        }
        if port == 0x402 {
            return Ok(self.debug.port_read(port, 2) as u16);
        }
        Ok(self.ports.port_read(port, 2) as u16)
    }

    fn port_out_u16(&mut self, port: u16, val: u16) -> Result<(), ExecError> {
        if (0x3F8..0x400).contains(&port) {
            self.com1.port_write(port, 2, u32::from(val));
            return Ok(());
        }
        if port == 0x402 {
            self.debug.port_write(port, 2, u32::from(val));
            return Ok(());
        }
        self.ports.port_write(port, 2, u32::from(val));
        Ok(())
    }

    fn port_in_u32(&mut self, port: u16) -> Result<u32, ExecError> {
        if (0x3F8..0x400).contains(&port) {
            return Ok(self.com1.port_read(port, 4));
        }
        if port == 0x402 {
            return Ok(self.debug.port_read(port, 4));
        }
        Ok(self.ports.port_read(port, 4))
    }

    fn port_out_u32(&mut self, port: u16, val: u32) -> Result<(), ExecError> {
        if (0x3F8..0x400).contains(&port) {
            self.com1.port_write(port, 4, val);
            return Ok(());
        }
        if port == 0x402 {
            self.debug.port_write(port, 4, val);
            return Ok(());
        }
        self.ports.port_write(port, 4, val);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_rom_prints_on_com1_and_debug() {
        let mut m = Machine::new(16 * 1024 * 1024);
        m.load_hello_rom().unwrap();
        m.reset();
        let steps = m.run(10_000).unwrap();
        assert!(steps > 0);
        assert!(m.cpu.halted, "ROM should HLT");
        assert_eq!(m.com1_text(), EXPECTED_HELLO);
        assert_eq!(m.debug_text(), EXPECTED_HELLO);
    }

    #[test]
    fn reset_fetch_is_rom() {
        let mut m = Machine::new(1024 * 1024);
        m.load_hello_rom().unwrap();
        m.reset();
        let b = m.mem.read_u8(0xFFFF_FFF0).unwrap();
        assert_eq!(b, 0xE9, "near JMP at reset vector");
    }
}
