//! Physical RAM + ROM window.

#[derive(Clone, Debug)]
pub struct RomWindow {
    pub base: u64,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct PhysMem {
    ram: Vec<u8>,
    rom: Option<RomWindow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemError {
    OutOfRange,
    RomWrite,
}

impl PhysMem {
    pub fn new(ram_size: usize) -> Self {
        Self {
            ram: vec![0; ram_size],
            rom: None,
        }
    }

    pub fn ram_len(&self) -> usize {
        self.ram.len()
    }

    pub fn map_rom(&mut self, base: u64, data: Vec<u8>) {
        self.rom = Some(RomWindow { base, data });
    }

    fn rom_read(&self, addr: u64) -> Option<u8> {
        let rom = self.rom.as_ref()?;
        if addr < rom.base {
            return None;
        }
        let off = (addr - rom.base) as usize;
        if off < rom.data.len() {
            Some(rom.data[off])
        } else {
            None
        }
    }

    pub fn read_u8(&self, addr: u64) -> Result<u8, MemError> {
        if let Some(b) = self.rom_read(addr) {
            return Ok(b);
        }
        let i = addr as usize;
        if i < self.ram.len() {
            Ok(self.ram[i])
        } else {
            // Open bus for unmapped high addresses outside ROM: return 0xFF
            Ok(0xFF)
        }
    }

    pub fn write_u8(&mut self, addr: u64, val: u8) -> Result<(), MemError> {
        if self.rom_read(addr).is_some() {
            return Err(MemError::RomWrite);
        }
        let i = addr as usize;
        if i < self.ram.len() {
            self.ram[i] = val;
            Ok(())
        } else {
            // Ignore writes to unmapped space (MMIO stub).
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rom_overrides_ram() {
        let mut m = PhysMem::new(1024);
        m.ram[0] = 0x11;
        m.map_rom(0, vec![0xF4]);
        assert_eq!(m.read_u8(0).unwrap(), 0xF4);
        assert_eq!(m.write_u8(0, 0x00), Err(MemError::RomWrite));
    }
}
