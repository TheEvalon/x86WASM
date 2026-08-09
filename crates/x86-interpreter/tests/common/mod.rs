//! Shared guest-code harness for the round-4 paging tests.
//!
//! These tests build page tables in guest memory and execute real instruction
//! sequences, so the harness is deliberately thin: flat RAM, a byte-addressed
//! bus, and a couple of helpers for laying out paging structures and reading
//! back the resulting architectural state.

#![allow(dead_code)]

use x86_core::{CpuState, SegmentReg};
use x86_interpreter::{Bus, ExecError};

/// Flat guest RAM with no devices. Addresses outside the array are a bus
/// `MemoryFault`, which the interpreter classifies as `#GP`/`#SS`.
pub struct RamBus {
    pub mem: Vec<u8>,
    pub ports: Vec<(u16, u32)>,
}

impl RamBus {
    pub fn new(size: usize) -> Self {
        Self {
            mem: vec![0u8; size],
            ports: Vec::new(),
        }
    }

    pub fn write_bytes(&mut self, addr: usize, bytes: &[u8]) {
        self.mem[addr..addr + bytes.len()].copy_from_slice(bytes);
    }

    pub fn peek_u8(&self, addr: usize) -> u8 {
        self.mem[addr]
    }

    pub fn peek_u16(&self, addr: usize) -> u16 {
        u16::from_le_bytes([self.mem[addr], self.mem[addr + 1]])
    }

    pub fn peek_u32(&self, addr: usize) -> u32 {
        u32::from_le_bytes([
            self.mem[addr],
            self.mem[addr + 1],
            self.mem[addr + 2],
            self.mem[addr + 3],
        ])
    }

    pub fn poke_u32(&mut self, addr: usize, value: u32) {
        self.write_bytes(addr, &value.to_le_bytes());
    }
}

impl Bus for RamBus {
    fn read_u8(&mut self, addr: u64) -> Result<u8, ExecError> {
        let index = addr as usize;
        if index >= self.mem.len() {
            return Err(ExecError::MemoryFault(addr));
        }
        Ok(self.mem[index])
    }

    fn write_u8(&mut self, addr: u64, val: u8) -> Result<(), ExecError> {
        let index = addr as usize;
        if index >= self.mem.len() {
            return Err(ExecError::MemoryFault(addr));
        }
        self.mem[index] = val;
        Ok(())
    }

    fn port_in_u8(&mut self, _port: u16) -> Result<u8, ExecError> {
        Ok(0xFF)
    }

    fn port_out_u8(&mut self, port: u16, val: u8) -> Result<(), ExecError> {
        self.ports.push((port, u32::from(val)));
        Ok(())
    }
}

/// Paging-structure entry flags (SDM Vol. 3 Tables 4-4, 4-5, 4-6).
pub const P: u32 = 1 << 0;
pub const RW: u32 = 1 << 1;
pub const US: u32 = 1 << 2;
pub const A: u32 = 1 << 5;
pub const D: u32 = 1 << 6;
pub const PS: u32 = 1 << 7;
pub const G: u32 = 1 << 8;

/// `CR0` bits used by the tests (SDM Vol. 3 §2.5, §4.1.1).
pub const CR0_PE: u64 = 1 << 0;
pub const CR0_WP: u64 = 1 << 16;
pub const CR0_PG: u64 = 1 << 31;
/// `CR4` bits used by the tests (SDM Vol. 3 §4.1.3).
pub const CR4_PSE: u64 = 1 << 4;
pub const CR4_PGE: u64 = 1 << 7;

/// A conventional layout for the tests: page directory at 0x1000, one page
/// table at 0x2000 covering linear 0x0000_0000-0x003F_FFFF.
pub const PD_BASE: usize = 0x1000;
pub const PT_BASE: usize = 0x2000;

/// Build an identity mapping for the first `pages` 4-KiB pages through a
/// directory at [`PD_BASE`] and a page table at [`PT_BASE`].
pub fn identity_map_first_4mib(bus: &mut RamBus, pages: usize, pte_flags: u32) {
    bus.poke_u32(PD_BASE, PT_BASE as u32 | P | RW | US);
    for page in 0..pages {
        bus.poke_u32(PT_BASE + page * 4, (page as u32) << 12 | pte_flags);
    }
}

/// Physical address of the PTE that maps `linear` in the [`PT_BASE`] table.
pub fn pte_addr(linear: u32) -> usize {
    PT_BASE + (((linear >> 12) & 0x3FF) as usize) * 4
}

/// A flat 32-bit protected-mode CPU with paging enabled, CS/DS/SS/ES all base 0
/// limit 4 GiB, executing at `eip` with `ESP = esp`.
///
/// The GDT is not walked: the descriptor caches are loaded directly, which is
/// the state a real GDT load would leave and keeps these tests about paging.
/// Spec: SDM Vol. 3 §3.4.3 (descriptor cache).
pub fn flat_protected_cpu(eip: u32, esp: u32) -> CpuState {
    let code = SegmentReg {
        selector: 0x0008,
        base: 0,
        limit: 0xFFFF_FFFF,
        flags: 0xC09B,
    };
    let data = SegmentReg {
        selector: 0x0010,
        base: 0,
        limit: 0xFFFF_FFFF,
        flags: 0xC093,
    };
    let mut cpu = CpuState::reset();
    cpu.cs = code;
    cpu.ds = data.clone();
    cpu.es = data.clone();
    cpu.ss = data.clone();
    cpu.fs = data.clone();
    cpu.gs = data;
    cpu.cr0 = CR0_PE | CR0_PG;
    cpu.cr3 = PD_BASE as u64;
    cpu.cr4 = 0;
    cpu.rip = u64::from(eip);
    cpu.set_gpr_u32(CpuState::RSP, esp);
    cpu
}

/// Install a 32-bit interrupt gate for `vector` targeting `offset` in the
/// current code selector, plus the IDTR that finds it.
///
/// Spec: SDM Vol. 3 §6.11 Figure 6-2 (386 interrupt gate, type 0xE).
pub fn install_386_idt(
    bus: &mut RamBus,
    cpu: &mut CpuState,
    idt_base: usize,
    entries: &[(u8, u32)],
) {
    for &(vector, offset) in entries {
        let gate = idt_base + usize::from(vector) * 8;
        let bytes = [
            offset as u8,
            (offset >> 8) as u8,
            0x08,
            0x00,
            0x00,
            0x8E,
            (offset >> 16) as u8,
            (offset >> 24) as u8,
        ];
        bus.write_bytes(gate, &bytes);
    }
    cpu.idtr.base = idt_base as u64;
    cpu.idtr.limit = 0x7FF;
}

/// Flat 4-GiB GDT: `0x08` ring-0 code, `0x10` ring-0 data, `0x18` ring-3 code,
/// `0x20` ring-3 data. Spec: SDM Vol. 3 §3.4.5 (descriptor format).
pub fn install_flat_gdt(bus: &mut RamBus, cpu: &mut CpuState, gdt_base: usize) {
    let descriptor = |access: u8| -> [u8; 8] { [0xFF, 0xFF, 0x00, 0x00, 0x00, access, 0xCF, 0x00] };
    bus.write_bytes(gdt_base, &[0u8; 8]);
    bus.write_bytes(gdt_base + 0x08, &descriptor(0x9B));
    bus.write_bytes(gdt_base + 0x10, &descriptor(0x93));
    bus.write_bytes(gdt_base + 0x18, &descriptor(0xFB));
    bus.write_bytes(gdt_base + 0x20, &descriptor(0xF3));
    cpu.gdtr.base = gdt_base as u64;
    cpu.gdtr.limit = 0x27;
}

/// [`flat_protected_cpu`] moved to ring 3: CS/SS/DS take the RPL-3 selectors of
/// [`install_flat_gdt`], which makes CPL 3 (SDM Vol. 3 §5.5).
pub fn to_ring3(cpu: &mut CpuState) {
    cpu.cs.selector = 0x001B;
    cpu.cs.flags = 0xC0FB;
    for seg in [
        &mut cpu.ds,
        &mut cpu.es,
        &mut cpu.ss,
        &mut cpu.fs,
        &mut cpu.gs,
    ] {
        seg.selector = 0x0023;
        seg.flags = 0xC0F3;
    }
}

/// Install a real-mode IVT entry (`vector` → `segment:offset`).
pub fn install_ivt(bus: &mut RamBus, cpu: &mut CpuState, vector: u8, segment: u16, offset: u16) {
    let entry = usize::from(vector) * 4;
    bus.write_bytes(entry, &offset.to_le_bytes());
    bus.write_bytes(entry + 2, &segment.to_le_bytes());
    cpu.idtr.base = 0;
    cpu.idtr.limit = 0x3FF;
}

/// Physical layout shared by the paging tests. Everything below `0x2_0000` is
/// identity mapped by the first page table, so a physical address in this list
/// is also its linear address unless a test says otherwise.
pub const CODE: u32 = 0x5000;
pub const HANDLER: u32 = 0x6000;
pub const STACK_TOP: u32 = 0x8000;
pub const DATA: u32 = 0x9000;
/// Second page table, for the linear 4-MiB region starting at [`HIGH`].
pub const PT2: usize = 0xA000;
/// A linear address outside the identity-mapped region, so a test can prove
/// the physical address really came from the page tables.
pub const HIGH: u32 = 0x0040_0000;

/// Stack slots of a `#PF`-style frame built by a 386 interrupt gate entered
/// with `ESP = STACK_TOP` (SDM Vol. 3 §6.12.1 Figure 6-4).
pub const FRAME_ERROR_CODE: usize = (STACK_TOP - 16) as usize;
pub const FRAME_EIP: usize = (STACK_TOP - 12) as usize;

/// 32-bit protected mode with paging on, the low 128 KiB identity mapped, a
/// flat GDT, and a 386 IDT whose `#GP` and `#PF` gates both reach `HANDLER`.
pub fn paged_fixture(code: &[u8]) -> (CpuState, RamBus, x86_mmu::paging::Mmu) {
    let mut bus = RamBus::new(0x2_0000);
    identity_map_first_4mib(&mut bus, 0x20, P | RW | US);
    let mut cpu = flat_protected_cpu(CODE, STACK_TOP);
    install_flat_gdt(&mut bus, &mut cpu, 0x3000);
    install_386_idt(&mut bus, &mut cpu, 0x4000, &[(13, HANDLER), (14, HANDLER)]);
    bus.write_bytes(CODE as usize, code);
    bus.write_bytes(HANDLER as usize, &[0xF4]);
    (cpu, bus, x86_mmu::paging::Mmu::new())
}

/// Point the second page directory entry at [`PT2`] and map [`HIGH`] there.
pub fn map_high_page(bus: &mut RamBus, pte: u32) {
    bus.poke_u32(PD_BASE + 4, PT2 as u32 | P | RW | US);
    bus.poke_u32(PT2, pte);
}

/// A real-address-mode CPU executing at `0x0000:eip` with a flat 64-KiB stack.
pub fn real_mode_cpu(eip: u16, sp: u16) -> CpuState {
    let mut cpu = CpuState::reset();
    cpu.cs = SegmentReg::real_mode_code(0);
    cpu.ds = SegmentReg::real_mode(0);
    cpu.es = SegmentReg::real_mode(0);
    cpu.ss = SegmentReg::real_mode(0);
    cpu.rip = u64::from(eip);
    cpu.set_gpr_u16(CpuState::RSP, sp);
    cpu
}
