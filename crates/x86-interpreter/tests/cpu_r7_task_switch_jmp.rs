//! Round-7 slice 1: hardware task switch via far `JMP` to a 32-bit TSS or
//! task gate (JMP form only).
//!
//! Spec: Intel SDM Vol. 2 "JMP"; Vol. 3 §§7.2–7.3 (Figure 7-5, Table 7-1).

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
    bus.write_bytes(GDT + 8, &encode_seg_desc(0, 0xF_FFFF, 0x9A, 0xC0)); // code D=1
    bus.write_bytes(GDT + 16, &encode_seg_desc(0, 0xF_FFFF, 0x93, 0xC0)); // data B=1
    bus.write_bytes(GDT + 24, &encode_seg_desc(OLD_TSS as u32, 0x67, 0x8B, 0)); // busy old TSS
    bus.write_bytes(GDT + 32, &encode_seg_desc(NEW_TSS as u32, 0x67, 0x89, 0)); // available new TSS
    bus.write_bytes(GDT + 40, &encode_task_gate(SEL_NEW_TSS, 0x85)); // P|DPL0|type5
}

fn fill_new_tss(bus: &mut RamBus) {
    bus.poke_u32(NEW_TSS + 28, 0); // CR3
    bus.poke_u32(NEW_TSS + 32, TASK_CODE as u32); // EIP
    bus.poke_u32(NEW_TSS + 36, 0x202); // EFLAGS IF=1
    bus.poke_u32(NEW_TSS + 40, 0x1111_1111); // EAX
    bus.poke_u32(NEW_TSS + 44, 0x2222_2222); // ECX
    bus.poke_u32(NEW_TSS + 48, 0x3333_3333); // EDX
    bus.poke_u32(NEW_TSS + 52, 0x4444_4444); // EBX
    bus.poke_u32(NEW_TSS + 56, 0x0000_A000); // ESP
    bus.poke_u32(NEW_TSS + 60, 0x5555_5555); // EBP
    bus.poke_u32(NEW_TSS + 64, 0x6666_6666); // ESI
    bus.poke_u32(NEW_TSS + 68, 0x7777_7777); // EDI
    bus.poke_u16(NEW_TSS + 72, SEL_KDATA); // ES
    bus.poke_u16(NEW_TSS + 76, SEL_KCODE); // CS
    bus.poke_u16(NEW_TSS + 80, SEL_KDATA); // SS
    bus.poke_u16(NEW_TSS + 84, SEL_KDATA); // DS
    bus.poke_u16(NEW_TSS + 88, 0); // FS null
    bus.poke_u16(NEW_TSS + 92, 0); // GS null
    bus.poke_u16(NEW_TSS + 96, 0); // LDTR null
}

fn fixture(code: &[u8]) -> (CpuState, RamBus) {
    let mut bus = RamBus::new(0x10000);
    install_gdt(&mut bus);
    fill_new_tss(&mut bus);
    bus.write_bytes(CODE, code);
    bus.mem[TASK_CODE] = 0xF4; // HLT in new task

    // Vector 13 #GP for architectural faults delivered through IDT.
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

/// Far `JMP` to an available 32-bit TSS switches tasks and marks the new TSS busy.
#[test]
fn jmp_far_to_available_tss32_switches_task() {
    // EA imm32:imm16 under CS.D=1 — offset ignored for TSS targets.
    let mut bytes = vec![0xEA];
    bytes.extend_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
    bytes.extend_from_slice(&SEL_NEW_TSS.to_le_bytes());
    let (mut cpu, mut bus) = fixture(&bytes);

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.tr.selector, SEL_NEW_TSS);
    assert_eq!(cpu.tr.flags & 0x0F, 0xB, "new TSS busy");
    assert_eq!(bus.peek_u8(GDT + 32 + 5) & 0x0F, 0xB);
    assert_eq!(bus.peek_u8(GDT + 24 + 5) & 0x0F, 0x9, "old TSS available");
    assert_eq!(cpu.rip, TASK_CODE as u64);
    assert_eq!(cpu.gpr_u32(CpuState::RAX), 0x1111_1111);
    assert_eq!(cpu.gpr_u32(CpuState::RSP), 0x0000_A000);
    assert_eq!(cpu.cs.selector, SEL_KCODE);
    assert_eq!(cpu.ss.selector, SEL_KDATA);
    assert_ne!(cpu.cr0 & (1 << 3), 0, "CR0.TS set");
    // Outgoing EIP (next after JMP) saved in old TSS.
    assert_eq!(bus.peek_u32(OLD_TSS + 32), CODE as u32 + 7);
    assert_eq!(bus.peek_u32(OLD_TSS + 40), 0xAAAA_AAAA);
}

/// Far `JMP` through a GDT task gate resolves the TSS and switches.
#[test]
fn jmp_far_through_task_gate_switches_task() {
    let mut bytes = vec![0xEA];
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&SEL_TASK_GATE.to_le_bytes());
    let (mut cpu, mut bus) = fixture(&bytes);

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.tr.selector, SEL_NEW_TSS);
    assert_eq!(cpu.rip, TASK_CODE as u64);
    assert_eq!(cpu.gpr_u32(CpuState::RCX), 0x2222_2222);
}

/// `FF /5` memory-indirect far JMP to a TSS switches the same way.
#[test]
fn jmp_far_indirect_to_tss32_switches_task() {
    // FF 2E 00 40   JMP FAR [0x4000]  (mod=00 r/m=110 disp16 under asize16…;
    // under CS.D=1 default asize32: need 67 override or use [disp32].
    // Use 67 FF 2E disp16: address-size override → m16:32 pointer at DS:0x4000.
    let code = [0x67, 0xFF, 0x2E, 0x00, 0x40];
    let (mut cpu, mut bus) = fixture(&code);
    bus.poke_u32(0x4000, 0xCAFE_BABE);
    bus.poke_u16(0x4004, SEL_NEW_TSS);

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.tr.selector, SEL_NEW_TSS);
    assert_eq!(cpu.rip, TASK_CODE as u64);
}

/// Busy target TSS raises `#GP(selector)`.
#[test]
fn jmp_far_to_busy_tss_raises_gp() {
    let mut bytes = vec![0xEA];
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&SEL_NEW_TSS.to_le_bytes());
    let (mut cpu, mut bus) = fixture(&bytes);
    // Force new TSS busy in the GDT.
    bus.mem[GDT + 32 + 5] = 0x8B;

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.rip, 0x1900, "vectored through #GP gate");
    // Error code on stack: selector with EXT=0.
}

/// Far `CALL` to a TSS is explicitly unsupported in this slice (no nested task).
#[test]
fn call_far_to_tss_is_unsupported() {
    // 9A ptr16:32
    let mut bytes = vec![0x9A];
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&SEL_NEW_TSS.to_le_bytes());
    let (mut cpu, mut bus) = fixture(&bytes);

    let err = step(&mut cpu, &mut bus).unwrap_err();
    assert!(
        matches!(err, ExecError::Unsupported(_)),
        "CALL→TSS must stay unsupported, got {err:?}"
    );
    assert_eq!(cpu.tr.selector, SEL_OLD_TSS, "TR unchanged");
    assert_eq!(cpu.rip, CODE as u64, "EIP unchanged");
}

/// New-task `EFLAGS.VM=1` is unsupported (VM86 task switch deferred).
#[test]
fn jmp_far_to_tss_with_vm_flag_unsupported() {
    let mut bytes = vec![0xEA];
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&SEL_NEW_TSS.to_le_bytes());
    let (mut cpu, mut bus) = fixture(&bytes);
    bus.poke_u32(NEW_TSS + 36, 0x202 | (1 << 17));

    let err = step(&mut cpu, &mut bus).unwrap_err();
    assert!(matches!(err, ExecError::Unsupported(_)));
    assert_eq!(cpu.tr.selector, SEL_OLD_TSS);
}
