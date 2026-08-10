//! Round-7 slice 4: `LLDT`/`SLDT` and far `CALL` through an LDT call gate.
//!
//! Spec: Intel SDM Vol. 2 "LLDT"/"SLDT"/"CALL"; Vol. 3 §§2.4.2, 3.5.1, 5.8.2.

use x86_core::CpuState;
use x86_interpreter::{step, Bus, ExecError};

struct RamBus {
    mem: Vec<u8>,
}

impl RamBus {
    fn new(size: usize) -> Self {
        Self {
            mem: vec![0u8; size],
        }
    }

    fn write_bytes(&mut self, addr: usize, bytes: &[u8]) {
        self.mem[addr..addr + bytes.len()].copy_from_slice(bytes);
    }

    fn poke_u32(&mut self, addr: usize, value: u32) {
        self.write_bytes(addr, &value.to_le_bytes());
    }

    fn poke_u16(&mut self, addr: usize, value: u16) {
        self.write_bytes(addr, &value.to_le_bytes());
    }

    fn peek_u32(&self, addr: usize) -> u32 {
        u32::from_le_bytes([
            self.mem[addr],
            self.mem[addr + 1],
            self.mem[addr + 2],
            self.mem[addr + 3],
        ])
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

    fn port_out_u8(&mut self, _port: u16, _val: u8) -> Result<(), ExecError> {
        Ok(())
    }
}

const GDT: usize = 0x2000;
const LDT: usize = 0x2400;
const IDT: usize = 0x2800;
const TSS: usize = 0x3000;
const CODE: usize = 0x1000;
const GATE_TARGET: u32 = 0x0000_1A00;

const SEL_KCODE: u16 = 0x0008;
const SEL_KDATA: u16 = 0x0010;
const SEL_TSS: u16 = 0x0018;
const SEL_LDT: u16 = 0x0020;
const SEL_LDT_GATE: u16 = 0x0004; // LDT index 0, TI=1

fn encode_seg_desc(base: u32, limit20: u32, access: u8, gran_flags: u8) -> [u8; 8] {
    let lim = limit20 & 0xF_FFFF;
    [
        lim as u8,
        (lim >> 8) as u8,
        base as u8,
        (base >> 8) as u8,
        (base >> 16) as u8,
        access,
        ((lim >> 16) as u8 & 0x0F) | (gran_flags & 0xF0),
        (base >> 24) as u8,
    ]
}

fn encode_call_gate32(offset: u32, selector: u16, access: u8, param_count: u8) -> [u8; 8] {
    let off = offset.to_le_bytes();
    let sel = selector.to_le_bytes();
    [
        off[0],
        off[1],
        sel[0],
        sel[1],
        param_count & 0x1F,
        access,
        off[2],
        off[3],
    ]
}

fn encode_idt_gate32(offset: u32, selector: u16, access: u8) -> [u8; 8] {
    let off = offset.to_le_bytes();
    let sel = selector.to_le_bytes();
    [off[0], off[1], sel[0], sel[1], 0, access, off[2], off[3]]
}

fn install_tables(bus: &mut RamBus) {
    bus.write_bytes(GDT, &[0u8; 8]);
    bus.write_bytes(GDT + 8, &encode_seg_desc(0, 0xF_FFFF, 0x9A, 0xC0));
    bus.write_bytes(GDT + 16, &encode_seg_desc(0, 0xF_FFFF, 0x93, 0xC0));
    bus.write_bytes(GDT + 24, &encode_seg_desc(TSS as u32, 0x67, 0x8B, 0));
    bus.write_bytes(GDT + 32, &encode_seg_desc(LDT as u32, 15, 0x82, 0)); // LDT type=2

    bus.write_bytes(
        LDT,
        &encode_call_gate32(GATE_TARGET, SEL_KCODE, 0x8C, 0),
    );

    bus.poke_u32(TSS + 4, 0x9000);
    bus.poke_u16(TSS + 8, SEL_KDATA);
    bus.write_bytes(IDT + 13 * 8, &encode_idt_gate32(0x1900, SEL_KCODE, 0x8E));
    bus.mem[0x1900] = 0xF4;
    bus.mem[GATE_TARGET as usize] = 0xF4;
}

fn fixture(code: &[u8]) -> (CpuState, RamBus) {
    let mut bus = RamBus::new(0x10000);
    install_tables(&mut bus);
    bus.write_bytes(CODE, code);
    let mut cpu = CpuState::reset();
    cpu.cr0 |= 1;
    cpu.cs = x86_core::SegmentReg {
        selector: SEL_KCODE,
        base: 0,
        limit: 0xFFFF_FFFF,
        flags: 0xC09A,
    };
    cpu.ss = x86_core::SegmentReg {
        selector: SEL_KDATA,
        base: 0,
        limit: 0xFFFF_FFFF,
        flags: 0xC093,
    };
    cpu.ds = cpu.ss.clone();
    cpu.tr = x86_core::SegmentReg {
        selector: SEL_TSS,
        base: TSS as u64,
        limit: 0x67,
        flags: 0x008B,
    };
    cpu.gdtr.base = GDT as u64;
    cpu.gdtr.limit = 39;
    cpu.idtr.base = IDT as u64;
    cpu.idtr.limit = 0x7FF;
    cpu.rip = CODE as u64;
    cpu.set_gpr_u32(CpuState::RSP, 0x8000);
    cpu.rflags = 0x202;
    (cpu, bus)
}

#[test]
fn lldt_loads_ldt_descriptor_and_sldt_stores_selector() {
    // 66 B8 20 00   MOV AX, 0x0020
    // 0F 00 D0      LLDT AX
    // 0F 00 C3      SLDT BX
    let code = [
        0x66, 0xB8, 0x20, 0x00, 0x0F, 0x00, 0xD0, 0x0F, 0x00, 0xC3, 0xF4,
    ];
    let (mut cpu, mut bus) = fixture(&code);
    step(&mut cpu, &mut bus).unwrap(); // MOV
    step(&mut cpu, &mut bus).unwrap(); // LLDT
    assert_eq!(cpu.ldtr.selector, SEL_LDT);
    assert_eq!(cpu.ldtr.base, LDT as u64);
    assert_eq!(cpu.ldtr.limit, 15);
    assert_eq!(cpu.ldtr.flags & 0x0F, 0x2);
    step(&mut cpu, &mut bus).unwrap(); // SLDT
    assert_eq!(cpu.gpr_u16(CpuState::RBX), SEL_LDT);
}

#[test]
fn lldt_null_clears_ldtr_cache() {
    let code = [0x66, 0xB8, 0x00, 0x00, 0x0F, 0x00, 0xD0, 0xF4];
    let (mut cpu, mut bus) = fixture(&code);
    cpu.ldtr = x86_core::SegmentReg {
        selector: SEL_LDT,
        base: LDT as u64,
        limit: 15,
        flags: 0x0082,
    };
    step(&mut cpu, &mut bus).unwrap(); // MOV
    step(&mut cpu, &mut bus).unwrap(); // LLDT null
    assert_eq!(cpu.ldtr.selector, 0);
    assert_eq!(cpu.ldtr.base, 0);
    assert_eq!(cpu.ldtr.limit, 0);
    assert_eq!(cpu.ldtr.flags, 0);
}

#[test]
fn lldt_rejects_wrong_type_and_cpl3() {
    // Wrong type: try LLDT of the code selector 0x08
    let code = [0x66, 0xB8, 0x08, 0x00, 0x0F, 0x00, 0xD0];
    let (mut cpu, mut bus) = fixture(&code);
    step(&mut cpu, &mut bus).unwrap(); // MOV
    step(&mut cpu, &mut bus).unwrap(); // LLDT → #GP
    assert_eq!(cpu.rip, 0x1900);

    // CPL3 → #GP(0)
    let code = [0x66, 0xB8, 0x20, 0x00, 0x0F, 0x00, 0xD0];
    let (mut cpu, mut bus) = fixture(&code);
    cpu.cs.selector = SEL_KCODE | 3;
    cpu.cs.flags = 0xC0FA;
    cpu.ss.selector = SEL_KDATA | 3;
    cpu.ss.flags = 0xC0F3;
    step(&mut cpu, &mut bus).unwrap(); // MOV
    step(&mut cpu, &mut bus).unwrap(); // LLDT → #GP(0)
    assert_eq!(cpu.rip, 0x1900);
}

#[test]
fn call_far_through_ldt_call_gate() {
    // Load LDT then CALL through LDT gate selector 0x0004 (TI=1).
    let mut bytes = vec![0x66, 0xB8, 0x20, 0x00, 0x0F, 0x00, 0xD0]; // MOV AX,LDT; LLDT
    bytes.push(0x9A);
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&SEL_LDT_GATE.to_le_bytes());
    let (mut cpu, mut bus) = fixture(&bytes);
    step(&mut cpu, &mut bus).unwrap(); // MOV
    step(&mut cpu, &mut bus).unwrap(); // LLDT
    let old_esp = cpu.gpr_u32(CpuState::RSP);
    let call_ip = cpu.rip as u32;
    step(&mut cpu, &mut bus).unwrap(); // CALL gate

    assert_eq!(cpu.rip, u64::from(GATE_TARGET));
    assert_eq!(cpu.cs.selector, SEL_KCODE);
    assert_eq!(cpu.gpr_u32(CpuState::RSP), old_esp - 8);
    assert_eq!(bus.peek_u32((old_esp - 8) as usize), call_ip + 7);
}

#[test]
fn lldt_in_real_mode_is_ud() {
    let mut bus = RamBus::new(0x10000);
    bus.write_bytes(6 * 4, &[0x00, 0x18, 0x00, 0x00]);
    bus.write_bytes(0x7C00, &[0x0F, 0x00, 0xD0, 0xF4]);
    bus.mem[0x1800] = 0xF4;
    let mut cpu = CpuState::reset();
    cpu.cs = x86_core::SegmentReg::real_mode_code(0);
    cpu.ss = x86_core::SegmentReg::real_mode(0);
    cpu.rip = 0x7C00;
    cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
    step(&mut cpu, &mut bus).unwrap();
    assert_eq!(cpu.ip16(), 0x1800);
}
