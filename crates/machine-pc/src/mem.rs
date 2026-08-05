//! Physical RAM + ROM window (+ A20 gate mask).

/// Physical address bit 20 — cleared when the A20 gate is disabled (IBM PC AT).
const A20_ADDR_BIT: u64 = 1 << 20;

#[derive(Clone, Debug)]
pub struct RomWindow {
    pub base: u64,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct PhysMem {
    ram: Vec<u8>,
    /// One or more ROM windows (e.g. BIOS high map + below-1 MiB alias).
    roms: Vec<RomWindow>,
    /// A20 gate: when false, physical bit 20 is forced clear (IBM PC AT).
    /// Power-on / reset default is enabled (open gate).
    a20_enabled: bool,
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
            roms: Vec::new(),
            a20_enabled: true,
        }
    }

    pub fn ram_len(&self) -> usize {
        self.ram.len()
    }

    pub fn a20_enabled(&self) -> bool {
        self.a20_enabled
    }

    /// Set A20 gate. Spec: IBM PC AT — gate disabled masks physical A20.
    pub fn set_a20_enabled(&mut self, enabled: bool) {
        self.a20_enabled = enabled;
    }

    /// Apply A20 mask to a physical address before RAM/ROM decode.
    fn apply_a20(&self, addr: u64) -> u64 {
        if self.a20_enabled {
            addr
        } else {
            addr & !A20_ADDR_BIT
        }
    }

    /// Replace all ROM windows with a single mapping (HELLO / lab ROMs).
    pub fn map_rom(&mut self, base: u64, data: Vec<u8>) {
        self.roms.clear();
        self.roms.push(RomWindow { base, data });
    }

    /// Clear every ROM window.
    pub fn clear_roms(&mut self) {
        self.roms.clear();
    }

    /// Append a ROM window without clearing existing ones (BIOS dual-map).
    pub fn add_rom(&mut self, base: u64, data: Vec<u8>) {
        self.roms.push(RomWindow { base, data });
    }

    fn rom_read(&self, addr: u64) -> Option<u8> {
        for rom in &self.roms {
            if addr < rom.base {
                continue;
            }
            let off = (addr - rom.base) as usize;
            if off < rom.data.len() {
                return Some(rom.data[off]);
            }
        }
        None
    }

    pub fn read_u8(&self, addr: u64) -> Result<u8, MemError> {
        let addr = self.apply_a20(addr);
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
        let addr = self.apply_a20(addr);
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

    /// Spec: machine-model — BIOS may appear at high map and `0xF0000` alias.
    #[test]
    fn dual_rom_windows_high_and_low_alias() {
        let mut m = PhysMem::new(1024 * 1024);
        m.add_rom(0xFFFF_0000, vec![0xAA; 4]);
        m.add_rom(0x000F_0000, vec![0xBB; 4]);
        assert_eq!(m.read_u8(0xFFFF_0000).unwrap(), 0xAA);
        assert_eq!(m.read_u8(0x000F_0000).unwrap(), 0xBB);
        assert_eq!(m.write_u8(0x000F_0001, 0x00), Err(MemError::RomWrite));
    }

    /// Spec: IBM PC AT A20 gate — when disabled, phys bit 20 is forced clear.
    #[test]
    fn a20_disabled_aliases_bit20() {
        let mut m = PhysMem::new(2 * 1024 * 1024);
        m.write_u8(0, 0x11).unwrap();
        m.write_u8(A20_ADDR_BIT, 0x22).unwrap();
        assert_eq!(m.read_u8(0).unwrap(), 0x11);
        assert_eq!(m.read_u8(A20_ADDR_BIT).unwrap(), 0x22);

        m.set_a20_enabled(false);
        assert!(!m.a20_enabled());
        // Access at 1 MiB aliases to address 0.
        assert_eq!(m.read_u8(A20_ADDR_BIT).unwrap(), 0x11);
        m.write_u8(A20_ADDR_BIT, 0x33).unwrap();
        assert_eq!(m.read_u8(0).unwrap(), 0x33);

        m.set_a20_enabled(true);
        assert_eq!(m.read_u8(A20_ADDR_BIT).unwrap(), 0x22);
    }
}
