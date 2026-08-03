//! Classic PC machine: CPU lab, serial HELLO ROM, and M2 PIC/PIT/CMOS port wiring.

#![forbid(unsafe_code)]

mod hello_rom;
mod mem;
mod ports;

pub use hello_rom::{build_hello_rom, EXPECTED_HELLO};
pub use mem::PhysMem;

use devices::{
    CmosRtc, DebugConsole, DualPic, Pit8254, PortDevice, Serial16550, CMOS_DATA, CMOS_INDEX,
    PIC_MASTER_CMD, PIC_MASTER_DATA, PIC_SLAVE_CMD, PIC_SLAVE_DATA, PIT_CH0_DATA, PIT_CH1_DATA,
    PIT_CH2_DATA, PIT_CONTROL,
};
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
    /// Dual 8259A — ICW1–ICW4 only (ports 0x20/0x21/0xA0/0xA1).
    pub pic: DualPic,
    /// 8254 PIT — channel-0 programming (ports 0x40–0x43).
    pub pit: Pit8254,
    /// MC146818 CMOS/RTC register bank (ports 0x70/0x71).
    pub cmos: CmosRtc,
    ports: PortBus,
}

impl Machine {
    pub fn new(ram_size: usize) -> Self {
        Self {
            cpu: CpuState::reset(),
            mem: PhysMem::new(ram_size),
            com1: Serial16550::new(0x3F8),
            debug: DebugConsole::new(),
            pic: DualPic::new(),
            pit: Pit8254::new(),
            cmos: CmosRtc::new(),
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
        self.pic.reset();
        self.pit.reset();
        self.cmos.reset();
    }

    pub fn step(&mut self) -> Result<(), MachineError> {
        let mut view = MachineBus {
            mem: &mut self.mem,
            com1: &mut self.com1,
            debug: &mut self.debug,
            pic: &mut self.pic,
            pit: &mut self.pit,
            cmos: &mut self.cmos,
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
            pic: &mut self.pic,
            pit: &mut self.pit,
            cmos: &mut self.cmos,
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
    pic: &'a mut DualPic,
    pit: &'a mut Pit8254,
    cmos: &'a mut CmosRtc,
    ports: &'a mut PortBus,
}

impl MachineBus<'_> {
    /// Decode classic PC port ownership. Spec: `docs/machine-model-pc-v1.md`.
    fn port_read(&mut self, port: u16, size: u8) -> u32 {
        match port {
            PIC_MASTER_CMD | PIC_MASTER_DATA | PIC_SLAVE_CMD | PIC_SLAVE_DATA => {
                self.pic.port_read(port, size)
            }
            PIT_CH0_DATA | PIT_CH1_DATA | PIT_CH2_DATA | PIT_CONTROL => {
                self.pit.port_read(port, size)
            }
            CMOS_INDEX | CMOS_DATA => self.cmos.port_read(port, size),
            0x3F8..0x400 => self.com1.port_read(port, size),
            0x402 => self.debug.port_read(port, size),
            _ => self.ports.port_read(port, size),
        }
    }

    fn port_write(&mut self, port: u16, size: u8, value: u32) {
        match port {
            PIC_MASTER_CMD | PIC_MASTER_DATA | PIC_SLAVE_CMD | PIC_SLAVE_DATA => {
                self.pic.port_write(port, size, value);
            }
            PIT_CH0_DATA | PIT_CH1_DATA | PIT_CH2_DATA | PIT_CONTROL => {
                self.pit.port_write(port, size, value);
            }
            CMOS_INDEX | CMOS_DATA => self.cmos.port_write(port, size, value),
            0x3F8..0x400 => self.com1.port_write(port, size, value),
            0x402 => self.debug.port_write(port, size, value),
            _ => self.ports.port_write(port, size, value),
        }
    }
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
        Ok(self.port_read(port, 1) as u8)
    }

    fn port_out_u8(&mut self, port: u16, val: u8) -> Result<(), ExecError> {
        self.port_write(port, 1, u32::from(val));
        Ok(())
    }

    fn port_in_u16(&mut self, port: u16) -> Result<u16, ExecError> {
        // Spec: Intel SDM Vol. 2 INS/OUTS/IN/OUT — I/O address in DX, size = operand size.
        Ok(self.port_read(port, 2) as u16)
    }

    fn port_out_u16(&mut self, port: u16, val: u16) -> Result<(), ExecError> {
        self.port_write(port, 2, u32::from(val));
        Ok(())
    }

    fn port_in_u32(&mut self, port: u16) -> Result<u32, ExecError> {
        Ok(self.port_read(port, 4))
    }

    fn port_out_u32(&mut self, port: u16, val: u32) -> Result<(), ExecError> {
        self.port_write(port, 4, val);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use devices::{
        CmosRtc, DualPic, Pit8254, CMOS_DATA, CMOS_INDEX, PIC_MASTER_CMD, PIC_MASTER_DATA,
        PIC_SLAVE_CMD, PIC_SLAVE_DATA, PIT_CH0_DATA, PIT_CONTROL, REG_STATUS_A,
    };

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

    /// Spec: classic PC PIC ports on MachineBus (Intel 8259A ICW1–ICW4; docs/machine-model-pc-v1.md).
    #[test]
    fn machine_bus_programs_dual_pic_icw() {
        let mut m = Machine::new(64 * 1024);
        {
            let mut bus = MachineBus {
                mem: &mut m.mem,
                com1: &mut m.com1,
                debug: &mut m.debug,
                pic: &mut m.pic,
                pit: &mut m.pit,
                cmos: &mut m.cmos,
                ports: &mut m.ports,
            };
            // Cascaded AT init: master 0x11/0x08/0x04/0x01, slave 0x11/0x70/0x02/0x01.
            bus.port_out_u8(PIC_MASTER_CMD, 0x11).unwrap();
            bus.port_out_u8(PIC_MASTER_DATA, 0x08).unwrap();
            bus.port_out_u8(PIC_MASTER_DATA, 0x04).unwrap();
            bus.port_out_u8(PIC_MASTER_DATA, 0x01).unwrap();
            bus.port_out_u8(PIC_SLAVE_CMD, 0x11).unwrap();
            bus.port_out_u8(PIC_SLAVE_DATA, 0x70).unwrap();
            bus.port_out_u8(PIC_SLAVE_DATA, 0x02).unwrap();
            bus.port_out_u8(PIC_SLAVE_DATA, 0x01).unwrap();
            // Reads still open-bus style until OCW (device unit model).
            assert_eq!(bus.port_in_u8(PIC_MASTER_CMD).unwrap(), 0xFF);
            assert_eq!(bus.port_in_u8(PIC_MASTER_DATA).unwrap(), 0xFF);
        }
        assert!(m.pic.master.initialized);
        assert!(m.pic.slave.initialized);
        assert_eq!(m.pic.master.vector_base, 0x08);
        assert_eq!(m.pic.master.slave_ir_mask(), 0x04);
        assert_eq!(m.pic.slave.vector_base, 0x70);
        assert_eq!(m.pic.slave.slave_id(), 2);
        assert!(m.pic.master.mode_8086);
        assert!(m.pic.slave.mode_8086);
    }

    /// Spec: 8254 channel-0 programming via MachineBus ports 0x40/0x43.
    #[test]
    fn machine_bus_programs_pit_channel0() {
        let mut m = Machine::new(64 * 1024);
        {
            let mut bus = MachineBus {
                mem: &mut m.mem,
                com1: &mut m.com1,
                debug: &mut m.debug,
                pic: &mut m.pic,
                pit: &mut m.pit,
                cmos: &mut m.cmos,
                ports: &mut m.ports,
            };
            // Mode 3 square wave, lo/hi access: control 0x36, count 0x1000.
            bus.port_out_u8(PIT_CONTROL, 0x36).unwrap();
            bus.port_out_u8(PIT_CH0_DATA, 0x00).unwrap();
            bus.port_out_u8(PIT_CH0_DATA, 0x10).unwrap();
            assert_eq!(bus.port_in_u8(PIT_CH0_DATA).unwrap(), 0x00);
            assert_eq!(bus.port_in_u8(PIT_CH0_DATA).unwrap(), 0x10);
            assert_eq!(bus.port_in_u8(PIT_CONTROL).unwrap(), 0xFF);
        }
        assert_eq!(m.pit.channel0().mode, 3);
        assert!(m.pit.channel0().count_loaded);
        assert_eq!(m.pit.channel0().count, 0x1000);
    }

    /// Spec: MC146818 CMOS index/data via MachineBus 0x70/0x71.
    #[test]
    fn machine_bus_cmos_index_data() {
        let mut m = Machine::new(64 * 1024);
        {
            let mut bus = MachineBus {
                mem: &mut m.mem,
                com1: &mut m.com1,
                debug: &mut m.debug,
                pic: &mut m.pic,
                pit: &mut m.pit,
                cmos: &mut m.cmos,
                ports: &mut m.ports,
            };
            assert_eq!(bus.port_in_u8(CMOS_INDEX).unwrap() & 0x7F, 0);
            bus.port_out_u8(CMOS_INDEX, 0x80 | 0x10).unwrap(); // NMI disable + index 0x10
            bus.port_out_u8(CMOS_DATA, 0x5A).unwrap();
            bus.port_out_u8(CMOS_INDEX, 0x10).unwrap();
            assert_eq!(bus.port_in_u8(CMOS_DATA).unwrap(), 0x5A);
            bus.port_out_u8(CMOS_INDEX, REG_STATUS_A).unwrap();
            assert_eq!(bus.port_in_u8(CMOS_DATA).unwrap(), 0x26);
        }
        assert!(!m.cmos.nmi_disabled);
        assert_eq!(m.cmos.read_reg(0x10), 0x5A);
    }

    /// Guest OUT/IN through interpreter → MachineBus programs PIC, PIT, CMOS.
    #[test]
    fn guest_out_in_programs_pic_pit_cmos() {
        let mut m = Machine::new(64 * 1024);
        // Real-mode program at 0000:0000 — Spec: SDM Vol. 2 OUT/IN imm8 forms.
        let prog: &[u8] = &[
            // PIC master cascade ICW: 0x11, 0x20, 0x04, 0x01
            0xB0, 0x11, // mov al, 0x11
            0xE6, 0x20, // out 0x20, al
            0xB0, 0x20, // mov al, 0x20
            0xE6, 0x21, // out 0x21, al
            0xB0, 0x04, // mov al, 0x04
            0xE6, 0x21, // out 0x21, al
            0xB0, 0x01, // mov al, 0x01
            0xE6, 0x21, // out 0x21, al
            // PIT ch0 mode3 count 0x0040
            0xB0, 0x36, // mov al, 0x36
            0xE6, 0x43, // out 0x43, al
            0xB0, 0x40, // mov al, 0x40
            0xE6, 0x40, // out 0x40, al
            0xB0, 0x00, // mov al, 0x00
            0xE6, 0x40, // out 0x40, al
            // CMOS write reg 0x14 = 0xA5
            0xB0, 0x14, // mov al, 0x14
            0xE6, 0x70, // out 0x70, al
            0xB0, 0xA5, // mov al, 0xA5
            0xE6, 0x71, // out 0x71, al
            // CMOS read back into AL
            0xB0, 0x14, // mov al, 0x14
            0xE6, 0x70, // out 0x70, al
            0xE4, 0x71, // in al, 0x71
            0xF4, // hlt
        ];
        for (i, b) in prog.iter().enumerate() {
            m.mem.write_u8(i as u64, *b).unwrap();
        }
        m.cpu = CpuState::reset();
        m.cpu.cs = x86_core::SegmentReg::real_mode_code(0x0000);
        m.cpu.set_ip16(0);
        m.cpu.halted = false;
        let steps = m.run(100).unwrap();
        assert!(steps > 0);
        assert!(m.cpu.halted);
        assert!(m.pic.master.initialized);
        assert_eq!(m.pic.master.vector_base, 0x20);
        assert_eq!(m.pic.master.slave_ir_mask(), 0x04);
        assert_eq!(m.pit.channel0().mode, 3);
        assert_eq!(m.pit.channel0().count, 0x0040);
        assert_eq!(m.cmos.read_reg(0x14), 0xA5);
        assert_eq!(m.cpu.al(), 0xA5);
    }

    /// Unrelated ports stay open-bus; COM1 / debug port 0x402 unchanged.
    #[test]
    fn unrelated_ports_open_bus_serial_unchanged() {
        let mut m = Machine::new(64 * 1024);
        {
            let mut bus = MachineBus {
                mem: &mut m.mem,
                com1: &mut m.com1,
                debug: &mut m.debug,
                pic: &mut m.pic,
                pit: &mut m.pit,
                cmos: &mut m.cmos,
                ports: &mut m.ports,
            };
            assert_eq!(bus.port_in_u8(0x60).unwrap(), 0xFF); // keyboard — unimplemented
            assert_eq!(bus.port_in_u8(0x80).unwrap(), 0xFF); // POST — unimplemented
            bus.port_out_u8(0x80, 0xAA).unwrap(); // ignored
            bus.port_out_u8(0x3F8, b'Z').unwrap();
            bus.port_out_u8(0x402, b'!').unwrap();
            // LSR THR empty bit still present on COM1
            assert_ne!(bus.port_in_u8(0x3FD).unwrap() & 0x20, 0);
        }
        assert_eq!(m.com1_text(), "Z");
        assert_eq!(m.debug_text(), "!");
        assert!(!m.pic.master.initialized);
        assert!(!m.pit.channel0().count_loaded);
    }

    /// Reset clears PIC/PIT/CMOS device state like serial recreation.
    #[test]
    fn reset_clears_pic_pit_cmos() {
        let mut m = Machine::new(64 * 1024);
        m.pic.port_write(PIC_MASTER_CMD, 1, 0x13);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x08);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x01);
        m.pit.port_write(PIT_CONTROL, 1, 0x36);
        m.pit.port_write(PIT_CH0_DATA, 1, 0x00);
        m.pit.port_write(PIT_CH0_DATA, 1, 0x10);
        m.cmos.port_write(CMOS_INDEX, 1, 0x10);
        m.cmos.port_write(CMOS_DATA, 1, 0xAB);
        m.com1.port_write(0x3F8, 1, u32::from(b'X'));

        m.reset();
        assert_eq!(m.pic, DualPic::new());
        assert_eq!(m.pit, Pit8254::new());
        assert_eq!(m.cmos, CmosRtc::new());
        assert_eq!(m.com1_text(), "");
        assert_eq!(m.debug_text(), "");
    }
}
