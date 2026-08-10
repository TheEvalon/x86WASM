//! Round-7 slice 2: far `CALL` through a 32-bit GDT call gate.
//!
//! Spec: Intel SDM Vol. 2 "CALL"; Vol. 3 §5.8.2 (Figures 5-8 / 5-9).

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
const IDT: usize = 0x2800;
const TSS: usize = 0x3000;
const CODE: usize = 0x1000;
const GATE_TARGET: u32 = 0x0000_1A00;
const KERNEL_ESP0: u32 = 0x0000_9000;

const SEL_KCODE: u16 = 0x0008;
const SEL_KDATA: u16 = 0x0010;
const SEL_TSS: u16 = 0x0018;
const SEL_UCODE: u16 = 0x0023;
const SEL_UDATA: u16 = 0x002B;
const SEL_GATE_SAME: u16 = 0x0030; // index 6, RPL=0
const SEL_GATE_PRIV: u16 = 0x003B; // index 7, RPL=3 (user invokes)

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
    bus.write_bytes(GDT + 8, &encode_seg_desc(0, 0xF_FFFF, 0x9A, 0xC0)); // kcode
    bus.write_bytes(GDT + 16, &encode_seg_desc(0, 0xF_FFFF, 0x93, 0xC0)); // kdata
    bus.write_bytes(GDT + 24, &encode_seg_desc(TSS as u32, 0x67, 0x8B, 0)); // busy TSS
    bus.write_bytes(GDT + 32, &encode_seg_desc(0, 0xF_FFFF, 0xFA, 0xC0)); // ucode
    bus.write_bytes(GDT + 40, &encode_seg_desc(0, 0xF_FFFF, 0xF3, 0xC0)); // udata
                                                                          // Same-CPL call gate DPL=0 → kernel code
    bus.write_bytes(
        GDT + 48,
        &encode_call_gate32(GATE_TARGET, SEL_KCODE, 0x8C, 0),
    );
    // Privilege-changing call gate DPL=3 → kernel code
    bus.write_bytes(
        GDT + 56,
        &encode_call_gate32(GATE_TARGET, SEL_KCODE, 0xEC, 0),
    );

    bus.poke_u32(TSS + 4, KERNEL_ESP0);
    bus.poke_u16(TSS + 8, SEL_KDATA);

    bus.write_bytes(IDT + 13 * 8, &encode_idt_gate32(0x1900, SEL_KCODE, 0x8E));
    bus.mem[0x1900] = 0xF4;
    bus.mem[GATE_TARGET as usize] = 0xF4;
}

fn kernel_fixture(code: &[u8]) -> (CpuState, RamBus) {
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
    cpu.gdtr.limit = 63;
    cpu.idtr.base = IDT as u64;
    cpu.idtr.limit = 0x7FF;
    cpu.rip = CODE as u64;
    cpu.set_gpr_u32(CpuState::RSP, 0x0000_8000);
    cpu.rflags = 0x202;
    (cpu, bus)
}

fn user_fixture(code: &[u8]) -> (CpuState, RamBus) {
    let (mut cpu, bus) = kernel_fixture(code);
    cpu.cs = x86_core::SegmentReg {
        selector: SEL_UCODE,
        base: 0,
        limit: 0xFFFF_FFFF,
        flags: 0xC0FA,
    };
    cpu.ss = x86_core::SegmentReg {
        selector: SEL_UDATA,
        base: 0,
        limit: 0xFFFF_FFFF,
        flags: 0xC0F3,
    };
    cpu.ds = cpu.ss.clone();
    (cpu, bus)
}

/// Same-CPL `CALL` through a 32-bit call gate pushes CS:EIP and enters the gate offset.
#[test]
fn call_far_same_cpl_through_call_gate32() {
    let mut bytes = vec![0x9A];
    bytes.extend_from_slice(&0xDEAD_BEEFu32.to_le_bytes()); // offset ignored
    bytes.extend_from_slice(&SEL_GATE_SAME.to_le_bytes());
    let (mut cpu, mut bus) = kernel_fixture(&bytes);
    let old_esp = cpu.gpr_u32(CpuState::RSP);

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.rip, u64::from(GATE_TARGET));
    assert_eq!(cpu.cs.selector, SEL_KCODE);
    assert_eq!(cpu.gpr_u32(CpuState::RSP), old_esp - 8); // CS + EIP
    assert_eq!(bus.peek_u32((old_esp - 4) as usize), u32::from(SEL_KCODE));
    assert_eq!(bus.peek_u32((old_esp - 8) as usize), CODE as u32 + 7);
}

/// Ring-3 `CALL` through a DPL=3 call gate switches to `SS0:ESP0` and pushes the outer stack.
#[test]
fn call_far_privilege_change_through_call_gate32() {
    let mut bytes = vec![0x9A];
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&SEL_GATE_PRIV.to_le_bytes());
    let (mut cpu, mut bus) = user_fixture(&bytes);
    let old_esp = cpu.gpr_u32(CpuState::RSP);
    let old_ss = cpu.ss.selector;

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.cs.selector & 3, 0);
    assert_eq!(cpu.cs.selector & !3, SEL_KCODE);
    assert_eq!(cpu.ss.selector, SEL_KDATA);
    assert_eq!(cpu.rip, u64::from(GATE_TARGET));
    let esp = cpu.gpr_u32(CpuState::RSP);
    assert_eq!(esp, KERNEL_ESP0 - 16); // SS, ESP, CS, EIP
    assert_eq!(bus.peek_u32((esp + 12) as usize), u32::from(old_ss));
    assert_eq!(bus.peek_u32((esp + 8) as usize), old_esp);
    assert_eq!(bus.peek_u32((esp + 4) as usize), u32::from(SEL_UCODE));
    assert_eq!(bus.peek_u32(esp as usize), CODE as u32 + 7);
}

/// `FF /3` memory-indirect far CALL through a call gate works.
#[test]
fn call_far_indirect_through_call_gate32() {
    let code = [0x67, 0xFF, 0x1E, 0x00, 0x40]; // CALL FAR [0x4000]
    let (mut cpu, mut bus) = kernel_fixture(&code);
    bus.poke_u32(0x4000, 0xCAFE_BABE);
    bus.poke_u16(0x4004, SEL_GATE_SAME);

    step(&mut cpu, &mut bus).unwrap();
    assert_eq!(cpu.rip, u64::from(GATE_TARGET));
}

/// Gate DPL too low for CPL → `#GP(gate selector)`.
#[test]
fn call_far_call_gate_dpl_violation_raises_gp() {
    // Kernel (CPL0) calling a DPL=3 gate with RPL=0 is fine on DPL check
    // (CPL≤DPL). Use ring-3 with a DPL=0 gate instead.
    let mut bytes = vec![0x9A];
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&(SEL_GATE_SAME | 3).to_le_bytes()); // RPL=3, gate DPL=0
    let (mut cpu, mut bus) = user_fixture(&bytes);

    step(&mut cpu, &mut bus).unwrap();
    assert_eq!(cpu.rip, 0x1900, "vectored #GP");
}

/// Non-zero parameter count remains unsupported in this slice.
#[test]
fn call_far_call_gate_with_params_unsupported() {
    let mut bytes = vec![0x9A];
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&SEL_GATE_SAME.to_le_bytes());
    let (mut cpu, mut bus) = kernel_fixture(&bytes);
    bus.mem[GDT + 48 + 4] = 2; // param count = 2

    let err = step(&mut cpu, &mut bus).unwrap_err();
    assert!(matches!(err, ExecError::Unsupported(_)));
    assert_eq!(cpu.rip, CODE as u64);
}

/// 16-bit call gates (type 4) are out of scope.
#[test]
fn call_far_16bit_call_gate_unsupported() {
    let mut bytes = vec![0x9A];
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&SEL_GATE_SAME.to_le_bytes());
    let (mut cpu, mut bus) = kernel_fixture(&bytes);
    bus.mem[GDT + 48 + 5] = 0x84; // P|type4

    let err = step(&mut cpu, &mut bus).unwrap_err();
    assert!(matches!(err, ExecError::Unsupported(_)));
}
