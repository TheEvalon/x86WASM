//! I/O APIC MMIO stub (classic base `0xFEC0_0000`).
//!
//! Spec: Intel 82093AA I/O Advanced Programmable Interrupt Controller:
//! - `IOREGSEL` at offset `00h` (index)
//! - `IOWIN` at offset `10h` (data window)
//! - Indirect: `IOAPICID` (`00h`), `IOAPICVER` (`01h`), `IOAPICARB` (`02h`),
//!   redirection table starting at index `10h` (two dwords per entry)
//!
//! Presence + redirection-table store/readback stub (24 entries). No IRQ
//! delivery onto the Local APIC.

/// Classic I/O APIC physical base (PC AT / ACPI MADT convention).
pub const IOAPIC_DEFAULT_BASE: u64 = 0xFEC0_0000;

/// I/O APIC MMIO window size (4 KiB page).
pub const IOAPIC_WINDOW_SIZE: u64 = 0x1000;

/// IOREGSEL — select register offset.
pub const IOAPIC_IOREGSEL: u32 = 0x00;

/// IOWIN — window register offset.
pub const IOAPIC_IOWIN: u32 = 0x10;

/// Indirect: IOAPICID.
pub const IOAPIC_IND_ID: u8 = 0x00;

/// Indirect: IOAPICVER.
pub const IOAPIC_IND_VER: u8 = 0x01;

/// Indirect: IOAPICARB.
pub const IOAPIC_IND_ARB: u8 = 0x02;

/// First redirection-table index (low dword of entry 0).
pub const IOAPIC_IND_REDTBL0: u8 = 0x10;

/// Version field (82093AA-class = `0x11`).
pub const IOAPIC_VERSION_ID: u8 = 0x11;

/// Max Redirection Entry (bits 23:16). `0x17` → 24 entries (0..=23).
pub const IOAPIC_MAX_REDIRECTION_ENTRY: u8 = 0x17;

/// Number of redirection entries (`MaxREDTBL + 1`).
pub const IOAPIC_REDIRECTION_COUNT: usize = (IOAPIC_MAX_REDIRECTION_ENTRY as usize) + 1;

/// Composed IOAPICVER reset value.
pub const IOAPIC_VER_VALUE: u32 =
    (IOAPIC_VERSION_ID as u32) | ((IOAPIC_MAX_REDIRECTION_ENTRY as u32) << 16);

/// I/O APIC presence MMIO with redirection-table store/readback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IoApicMmio {
    base: u64,
    /// IOREGSEL index (bits 7:0).
    index: u8,
    /// APIC ID in bits 27:24 of IOAPICID (82093AA).
    apic_id: u8,
    /// Redirection table: 24 entries × 2 dwords (low, high).
    redtbl: [u32; IOAPIC_REDIRECTION_COUNT * 2],
    /// Scratch for assembling IOWIN dword writes.
    iowin_scratch: [u8; 4],
}

impl Default for IoApicMmio {
    fn default() -> Self {
        Self::new()
    }
}

impl IoApicMmio {
    pub fn new() -> Self {
        Self {
            base: IOAPIC_DEFAULT_BASE,
            index: 0,
            apic_id: 0,
            redtbl: [0; IOAPIC_REDIRECTION_COUNT * 2],
            iowin_scratch: [0; 4],
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn base(&self) -> u64 {
        self.base
    }

    pub fn index(&self) -> u8 {
        self.index
    }

    pub fn owns(&self, addr: u64) -> bool {
        (self.base..self.base.saturating_add(IOAPIC_WINDOW_SIZE)).contains(&addr)
    }

    fn id_value(&self) -> u32 {
        u32::from(self.apic_id) << 24
    }

    fn indirect_read(&self) -> u32 {
        match self.index {
            IOAPIC_IND_ID => self.id_value(),
            IOAPIC_IND_VER => IOAPIC_VER_VALUE,
            IOAPIC_IND_ARB => self.id_value(),
            idx if idx >= IOAPIC_IND_REDTBL0 => {
                let entry = (idx - IOAPIC_IND_REDTBL0) as usize;
                if entry < self.redtbl.len() {
                    self.redtbl[entry]
                } else {
                    0
                }
            }
            _ => 0,
        }
    }

    fn indirect_write(&mut self, value: u32) {
        match self.index {
            IOAPIC_IND_ID => {
                // Spec: 82093AA IOAPICID — APIC ID in bits 27:24.
                self.apic_id = ((value >> 24) & 0x0F) as u8;
            }
            IOAPIC_IND_VER => {} // RO
            IOAPIC_IND_ARB => {} // RO in this stub
            idx if idx >= IOAPIC_IND_REDTBL0 => {
                let entry = (idx - IOAPIC_IND_REDTBL0) as usize;
                if entry < self.redtbl.len() {
                    self.redtbl[entry] = value;
                }
            }
            _ => {}
        }
    }

    /// Byte read within the claimed window, or `None` if unclaimed.
    pub fn mmio_read_u8(&self, addr: u64) -> Option<u8> {
        if !self.owns(addr) {
            return None;
        }
        let off = (addr - self.base) as u32;
        let dword_off = off & !3;
        let lane = (off & 3) as usize;
        let value = match dword_off {
            IOAPIC_IOREGSEL => u32::from(self.index),
            IOAPIC_IOWIN => self.indirect_read(),
            _ => 0,
        };
        Some(value.to_le_bytes()[lane])
    }

    /// Byte write within the claimed window.
    pub fn mmio_write_u8(&mut self, addr: u64, val: u8) -> bool {
        if !self.owns(addr) {
            return false;
        }
        let off = (addr - self.base) as u32;
        let dword_off = off & !3;
        let lane = (off & 3) as usize;
        match dword_off {
            IOAPIC_IOREGSEL if lane == 0 => {
                self.index = val;
            }
            IOAPIC_IOWIN => {
                self.iowin_scratch = self.indirect_read().to_le_bytes();
                self.iowin_scratch[lane] = val;
                self.indirect_write(u32::from_le_bytes(self.iowin_scratch));
            }
            _ => {}
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_u32(io: &IoApicMmio, off: u32) -> u32 {
        let mut b = [0u8; 4];
        for i in 0..4u64 {
            b[i as usize] = io
                .mmio_read_u8(IOAPIC_DEFAULT_BASE + u64::from(off) + i)
                .unwrap();
        }
        u32::from_le_bytes(b)
    }

    fn write_u32(io: &mut IoApicMmio, off: u32, value: u32) {
        for (i, byte) in value.to_le_bytes().into_iter().enumerate() {
            assert!(io.mmio_write_u8(IOAPIC_DEFAULT_BASE + u64::from(off) + i as u64, byte));
        }
    }

    fn select(io: &mut IoApicMmio, index: u8) {
        assert!(io.mmio_write_u8(IOAPIC_DEFAULT_BASE + u64::from(IOAPIC_IOREGSEL), index));
    }

    /// Spec: 82093AA — IOREGSEL/IOWIN + IOAPICVER MaxREDTBL=0x17 (24 entries).
    #[test]
    fn id_and_version_via_index_window() {
        let mut io = IoApicMmio::new();
        assert!(io.owns(IOAPIC_DEFAULT_BASE));
        assert!(!io.owns(IOAPIC_DEFAULT_BASE + IOAPIC_WINDOW_SIZE));

        select(&mut io, IOAPIC_IND_ID);
        assert_eq!(read_u32(&io, IOAPIC_IOWIN), 0);

        select(&mut io, IOAPIC_IND_VER);
        assert_eq!(read_u32(&io, IOAPIC_IOWIN), IOAPIC_VER_VALUE);
        assert_eq!(IOAPIC_VER_VALUE as u8, IOAPIC_VERSION_ID);
        assert_eq!((IOAPIC_VER_VALUE >> 16) as u8, IOAPIC_MAX_REDIRECTION_ENTRY);
        assert_eq!(read_u32(&io, IOAPIC_IOREGSEL) as u8, IOAPIC_IND_VER);
    }

    /// Spec: 82093AA — redirection table store/readback (24 entries × 2 dwords).
    #[test]
    fn redirection_table_store_readback() {
        let mut io = IoApicMmio::new();
        select(&mut io, IOAPIC_IND_REDTBL0);
        write_u32(&mut io, IOAPIC_IOWIN, 0x0001_00EF);
        select(&mut io, IOAPIC_IND_REDTBL0 + 1);
        write_u32(&mut io, IOAPIC_IOWIN, 0x0100_0000);

        select(&mut io, IOAPIC_IND_REDTBL0);
        assert_eq!(read_u32(&io, IOAPIC_IOWIN), 0x0001_00EF);
        select(&mut io, IOAPIC_IND_REDTBL0 + 1);
        assert_eq!(read_u32(&io, IOAPIC_IOWIN), 0x0100_0000);

        // Last entry (23): indexes 0x10 + 2*23 = 0x3E / 0x3F.
        let last_low = IOAPIC_IND_REDTBL0 + (23 * 2);
        select(&mut io, last_low);
        write_u32(&mut io, IOAPIC_IOWIN, 0xDEAD_BEEF);
        select(&mut io, last_low);
        assert_eq!(read_u32(&io, IOAPIC_IOWIN), 0xDEAD_BEEF);
    }

    #[test]
    fn id_bits_store_and_reset() {
        let mut io = IoApicMmio::new();
        select(&mut io, IOAPIC_IND_ID);
        write_u32(&mut io, IOAPIC_IOWIN, 0x0A00_0000);
        select(&mut io, IOAPIC_IND_ID);
        assert_eq!(read_u32(&io, IOAPIC_IOWIN), 0x0A00_0000);
        io.reset();
        assert_eq!(io, IoApicMmio::new());
    }
}
