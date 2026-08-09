//! Shared test support: a flat physical memory and a 32-bit page-table
//! builder.
//!
//! Layout used by every paging test: the page directory sits at
//! [`PD_BASE`], and page tables are bump-allocated after it.

#![allow(dead_code)]

use x86_mmu::paging::PageTableMemory;

/// Physical address of the page directory used by the tests.
pub const PD_BASE: u64 = 0x0001_0000;
/// First page table handed out by the bump allocator.
pub const FIRST_TABLE: u64 = 0x0002_0000;
/// Size of the flat memory the tests allocate. Large enough that a frame
/// address a test hands to a large PDE can also be read back as a (zeroed)
/// page table when `CR4.PSE` is cleared.
pub const MEM_SIZE: usize = 16 << 20;

/// Flat physical memory that records every paging-structure write, so a test
/// can assert that a faulting translation wrote nothing at all.
pub struct FlatMemory {
    bytes: Vec<u8>,
    /// Every `write_entry_u32`, in order.
    pub writes: Vec<(u64, u32)>,
    /// Every `read_entry_u32`, in order.
    pub reads: Vec<u64>,
}

impl FlatMemory {
    pub fn new(size: usize) -> Self {
        Self {
            bytes: vec![0; size],
            writes: Vec::new(),
            reads: Vec::new(),
        }
    }

    pub fn peek_u32(&self, phys_addr: u64) -> u32 {
        let index = self.index(phys_addr);
        u32::from_le_bytes([
            self.bytes[index],
            self.bytes[index + 1],
            self.bytes[index + 2],
            self.bytes[index + 3],
        ])
    }

    pub fn poke_u32(&mut self, phys_addr: u64, value: u32) {
        let index = self.index(phys_addr);
        self.bytes[index..index + 4].copy_from_slice(&value.to_le_bytes());
    }

    /// Forget the recorded access log.
    pub fn clear_log(&mut self) {
        self.writes.clear();
        self.reads.clear();
    }

    fn index(&self, phys_addr: u64) -> usize {
        let index = usize::try_from(phys_addr).expect("physical address fits a usize in tests");
        assert!(
            index + 4 <= self.bytes.len(),
            "test memory too small for physical address {phys_addr:#x}"
        );
        index
    }
}

impl PageTableMemory for FlatMemory {
    fn read_entry_u32(&mut self, phys_addr: u64) -> u32 {
        self.reads.push(phys_addr);
        self.peek_u32(phys_addr)
    }

    fn write_entry_u32(&mut self, phys_addr: u64, value: u32) {
        self.writes.push((phys_addr, value));
        self.poke_u32(phys_addr, value);
    }
}

/// A 32-bit page directory plus bump-allocated page tables in [`FlatMemory`].
pub struct PageTables {
    pub mem: FlatMemory,
    pub pd_base: u64,
    next_table: u64,
}

impl Default for PageTables {
    fn default() -> Self {
        Self::new()
    }
}

impl PageTables {
    pub fn new() -> Self {
        Self {
            mem: FlatMemory::new(MEM_SIZE),
            pd_base: PD_BASE,
            next_table: FIRST_TABLE,
        }
    }

    /// Physical address of the PDE that translates `linear` (SDM §4.3: PDE
    /// index is linear bits 31:22).
    pub fn pde_addr(&self, linear: u32) -> u64 {
        self.pd_base + 4 * u64::from(linear >> 22)
    }

    pub fn pde(&self, linear: u32) -> u32 {
        self.mem.peek_u32(self.pde_addr(linear))
    }

    pub fn set_pde(&mut self, linear: u32, value: u32) {
        let addr = self.pde_addr(linear);
        self.mem.poke_u32(addr, value);
    }

    /// Physical address of the PTE that translates `linear` (SDM §4.3: PTE
    /// index is linear bits 21:12). Panics if no page table is installed.
    pub fn pte_addr(&self, linear: u32) -> u64 {
        let pde = self.pde(linear);
        assert!(pde & 1 != 0, "no page table installed for {linear:#x}");
        u64::from(pde & 0xFFFF_F000) + 4 * u64::from((linear >> 12) & 0x3FF)
    }

    pub fn pte(&self, linear: u32) -> u32 {
        self.mem.peek_u32(self.pte_addr(linear))
    }

    pub fn set_pte(&mut self, linear: u32, value: u32) {
        let addr = self.pte_addr(linear);
        self.mem.poke_u32(addr, value);
    }

    /// Install a page table for the 4-MiB region containing `linear`, if there
    /// is not one already, and return its physical base.
    pub fn ensure_page_table(&mut self, linear: u32, pde_flags: u32) -> u64 {
        let pde = self.pde(linear);
        if pde & 1 != 0 && pde & (1 << 7) == 0 {
            let base = u64::from(pde & 0xFFFF_F000);
            self.set_pde(linear, (pde & 0xFFFF_F000) | pde_flags | 1);
            return base;
        }
        let base = self.next_table;
        self.next_table += 0x1000;
        let value = u32::try_from(base).expect("page table below 4 GiB") | pde_flags | 1;
        self.set_pde(linear, value);
        base
    }

    /// Map `linear` to `frame` with a 4-KiB page.
    ///
    /// `pde_flags` and `pte_flags` are OR'd over the present bit, so a caller
    /// passes exactly the R/W, U/S, A, D, G bits it wants.
    pub fn map_4kib(&mut self, linear: u32, frame: u32, pde_flags: u32, pte_flags: u32) {
        self.ensure_page_table(linear, pde_flags);
        let value = (frame & 0xFFFF_F000) | pte_flags | 1;
        self.set_pte(linear, value);
    }

    /// Map the 4-MiB region containing `linear` with a large PDE (PS = 1).
    pub fn map_4mib(&mut self, linear: u32, frame: u32, pde_flags: u32) {
        let value = (frame & 0xFFC0_0000) | pde_flags | (1 << 7) | 1;
        self.set_pde(linear, value);
    }
}
