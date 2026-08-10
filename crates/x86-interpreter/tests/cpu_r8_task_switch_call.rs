//! Round-8 slice 1: CALL-form hardware task switch to a 32-bit TSS or task gate.
//!
//! Spec: Intel SDM Vol. 2 "CALL"; Vol. 3 §§7.2–7.3 (Figure 7-5, Table 7-1 —
//! nested-task CALL sets NT, writes the previous-task link, leaves the old TSS
//! busy).

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

    fn peek_u16(&self, addr: usize) -> u16 {
        u16::from_le_bytes([self.mem[addr], self.mem[addr + 1]])
    }

    fn peek_u8(&self, addr: usize) -> u8 {
        self.mem[addr]
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
const OLD_TSS: usize = 0x3000;
const NEW_TSS: usize = 0x3100;
const CODE: usize = 0x1000;
const TASK_CODE: usize = 0x1800;

const SEL_KCODE: u16 = 0x0008;
const SEL_KDATA: u16 = 0x0010;
const SEL_OLD_TSS: u16 = 0x0018;
const SEL_NEW_TSS: u16 = 0x0020;
const SEL_TASK_GATE: u16 = 0x0028;

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

fn encode_task_gate(tss_selector: u16, access: u8) -> [u8; 8] {
    let sel = tss_selector.to_le_bytes();
    [0, 0, sel[0], sel[1], 0, access, 0, 0]
}

fn install_gdt(bus: &mut RamBus) {
    bus.write_bytes(GDT, &[0u8; 8]);
    bus.write_bytes(GDT + 8, &encode_seg_desc(0, 0xF_FFFF, 0x9A, 0xC0));
    bus.write_bytes(GDT + 16, &encode_seg_desc(0, 0xF_FFFF, 0x93, 0xC0));
    bus.write_bytes(GDT + 24, &encode_seg_desc(OLD_TSS as u32, 0x67, 0x8B, 0));
    bus.write_bytes(GDT + 32, &encode_seg_desc(NEW_TSS as u32, 0x67, 0x89, 0));
    bus.write_bytes(GDT + 40, &encode_task_gate(SEL_NEW_TSS, 0x85));
}

fn fill_new_tss(bus: &mut RamBus) {
    bus.poke_u32(NEW_TSS + 28, 0); // CR3
    bus.poke_u32(NEW_TSS + 32, TASK_CODE as u32); // EIP
    bus.poke_u32(NEW_TSS + 36, 0x202); // EFLAGS IF=1 (NT clear in image)
    bus.poke_u32(NEW_TSS + 40, 0x1111_1111);
    bus.poke_u32(NEW_TSS + 44, 0x2222_2222);
    bus.poke_u32(NEW_TSS + 48, 0x3333_3333);
    bus.poke_u32(NEW_TSS + 52, 0x4444_4444);
    bus.poke_u32(NEW_TSS + 56, 0x0000_A000);
    bus.poke_u32(NEW_TSS + 60, 0x5555_5555);
    bus.poke_u32(NEW_TSS + 64, 0x6666_6666);
    bus.poke_u32(NEW_TSS + 68, 0x7777_7777);
    bus.poke_u16(NEW_TSS + 72, SEL_KDATA);
    bus.poke_u16(NEW_TSS + 76, SEL_KCODE);
    bus.poke_u16(NEW_TSS + 80, SEL_KDATA);
    bus.poke_u16(NEW_TSS + 84, SEL_KDATA);
    bus.poke_u16(NEW_TSS + 88, 0);
    bus.poke_u16(NEW_TSS + 92, 0);
    bus.poke_u16(NEW_TSS + 96, 0);
}

fn fixture(code: &[u8]) -> (CpuState, RamBus) {
    let mut bus = RamBus::new(0x10000);
    install_gdt(&mut bus);
    fill_new_tss(&mut bus);
    bus.write_bytes(CODE, code);
    bus.mem[TASK_CODE] = 0xF4;

    let gp_gate = {
        let off = (0x1900u32).to_le_bytes();
        let sel = SEL_KCODE.to_le_bytes();
        [off[0], off[1], sel[0], sel[1], 0, 0x8E, off[2], off[3]]
    };
    bus.write_bytes(IDT + 13 * 8, &gp_gate);
    bus.mem[0x1900] = 0xF4;

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
    cpu.es = cpu.ss.clone();
    cpu.tr = x86_core::SegmentReg {
        selector: SEL_OLD_TSS,
        base: OLD_TSS as u64,
        limit: 0x67,
        flags: 0x008B,
    };
    cpu.gdtr.base = GDT as u64;
    cpu.gdtr.limit = 47;
    cpu.idtr.base = IDT as u64;
    cpu.idtr.limit = 0x7FF;
    cpu.rip = CODE as u64;
    cpu.set_gpr_u32(CpuState::RSP, 0x0000_8000);
    cpu.set_gpr_u32(CpuState::RAX, 0xAAAA_AAAA);
    cpu.rflags = 0x202;
    (cpu, bus)
}

/// Far `CALL` to an available 32-bit TSS nests: old stays busy, NT=1, back-link.
#[test]
fn call_far_to_available_tss32_nests_task() {
    // 9A imm32:imm16 under CS.D=1
    let mut bytes = vec![0x9A];
    bytes.extend_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
    bytes.extend_from_slice(&SEL_NEW_TSS.to_le_bytes());
    let (mut cpu, mut bus) = fixture(&bytes);

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.tr.selector, SEL_NEW_TSS);
    assert_eq!(cpu.tr.flags & 0x0F, 0xB, "new TSS busy");
    assert_eq!(bus.peek_u8(GDT + 32 + 5) & 0x0F, 0xB);
    assert_eq!(
        bus.peek_u8(GDT + 24 + 5) & 0x0F,
        0xB,
        "old TSS stays busy on CALL"
    );
    assert_ne!(cpu.rflags & (1 << 14), 0, "NT set on nested CALL");
    assert_eq!(
        bus.peek_u16(NEW_TSS),
        SEL_OLD_TSS,
        "previous-task link = old TR"
    );
    assert_eq!(cpu.rip, TASK_CODE as u64);
    assert_eq!(cpu.gpr_u32(CpuState::RAX), 0x1111_1111);
    assert_ne!(cpu.cr0 & (1 << 3), 0, "CR0.TS set");
    // Outgoing next-IP saved; no far-call stack push for task targets.
    assert_eq!(bus.peek_u32(OLD_TSS + 32), CODE as u32 + 7);
    assert_eq!(cpu.gpr_u32(CpuState::RSP), 0x0000_A000);
}

/// Far `CALL` through a GDT task gate resolves the TSS and nests the same way.
#[test]
fn call_far_through_task_gate_nests_task() {
    let mut bytes = vec![0x9A];
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&SEL_TASK_GATE.to_le_bytes());
    let (mut cpu, mut bus) = fixture(&bytes);

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.tr.selector, SEL_NEW_TSS);
    assert_ne!(cpu.rflags & (1 << 14), 0);
    assert_eq!(bus.peek_u16(NEW_TSS), SEL_OLD_TSS);
    assert_eq!(bus.peek_u8(GDT + 24 + 5) & 0x0F, 0xB);
}

/// `FF /3` memory-indirect far CALL to a TSS nests.
#[test]
fn call_far_indirect_to_tss32_nests_task() {
    // 67 FF 1E disp16 — address-size override → m16:32 pointer at DS:0x4000.
    let code = [0x67, 0xFF, 0x1E, 0x00, 0x40];
    let (mut cpu, mut bus) = fixture(&code);
    bus.poke_u32(0x4000, 0xCAFE_BABE);
    bus.poke_u16(0x4004, SEL_NEW_TSS);

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.tr.selector, SEL_NEW_TSS);
    assert_ne!(cpu.rflags & (1 << 14), 0);
    assert_eq!(bus.peek_u16(NEW_TSS), SEL_OLD_TSS);
}

/// Busy target TSS still raises `#GP(selector)` on CALL.
#[test]
fn call_far_to_busy_tss_raises_gp() {
    let mut bytes = vec![0x9A];
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&SEL_NEW_TSS.to_le_bytes());
    let (mut cpu, mut bus) = fixture(&bytes);
    bus.mem[GDT + 32 + 5] = 0x8B;

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.rip, 0x1900, "vectored through #GP gate");
    assert_eq!(cpu.tr.selector, SEL_OLD_TSS);
}

/// New-task `EFLAGS.VM=1` remains unsupported on CALL-form switches.
#[test]
fn call_far_to_tss_with_vm_flag_unsupported() {
    let mut bytes = vec![0x9A];
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&SEL_NEW_TSS.to_le_bytes());
    let (mut cpu, mut bus) = fixture(&bytes);
    bus.poke_u32(NEW_TSS + 36, 0x202 | (1 << 17));

    let err = step(&mut cpu, &mut bus).unwrap_err();
    assert!(matches!(err, ExecError::Unsupported(_)));
    assert_eq!(cpu.tr.selector, SEL_OLD_TSS);
}
