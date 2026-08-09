//! Round-5 slice 3: outer-privilege `IRETD` after a stack-switching gate.
//!
//! Spec: Intel SDM Vol. 2 "IRET/IRETD"; Vol. 3 §6.12.1.

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
const HANDLER: u32 = 0x0000_1800;
const USER_CONTINUE: u32 = 0x0000_1100;
const KERNEL_ESP0: u32 = 0x0000_9000;
const SEL_KCODE: u16 = 0x0008;
const SEL_KDATA: u16 = 0x0010;
const SEL_TSS: u16 = 0x0018;
const SEL_UCODE: u16 = 0x0023;
const SEL_UDATA: u16 = 0x002B;

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

fn encode_idt_gate32(offset: u32, selector: u16, access: u8) -> [u8; 8] {
    let off = offset.to_le_bytes();
    let sel = selector.to_le_bytes();
    [
        off[0], off[1], sel[0], sel[1], 0, access, off[2], off[3],
    ]
}

fn install_tables(bus: &mut RamBus) {
    bus.write_bytes(GDT, &[0u8; 8]);
    bus.write_bytes(GDT + 8, &encode_seg_desc(0, 0xF_FFFF, 0x9A, 0xC0));
    bus.write_bytes(GDT + 16, &encode_seg_desc(0, 0xF_FFFF, 0x93, 0xC0));
    bus.write_bytes(GDT + 24, &encode_seg_desc(TSS as u32, 0x67, 0x8B, 0));
    bus.write_bytes(GDT + 32, &encode_seg_desc(0, 0xF_FFFF, 0xFA, 0xC0));
    bus.write_bytes(GDT + 40, &encode_seg_desc(0, 0xF_FFFF, 0xF3, 0xC0));
    bus.poke_u32(TSS + 4, KERNEL_ESP0);
    bus.poke_u16(TSS + 8, SEL_KDATA);
    bus.write_bytes(
        IDT + 0x80 * 8,
        &encode_idt_gate32(HANDLER, SEL_KCODE, 0xEE),
    );
}

/// Ring-3 `INT 0x80` then handler `IRETD` restores user `CS:EIP` and `SS:ESP`.
#[test]
fn outer_iretd_returns_to_ring3_with_restored_stack() {
    let mut bus = RamBus::new(0x10000);
    install_tables(&mut bus);
    // User: CD 80 / F4 at continue
    bus.write_bytes(CODE, &[0xCD, 0x80]);
    bus.mem[USER_CONTINUE as usize] = 0xF4;
    // Handler: IRETD
    bus.mem[HANDLER as usize] = 0xCF;

    let mut cpu = CpuState::reset();
    cpu.cr0 |= 1;
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
    cpu.tr = x86_core::SegmentReg {
        selector: SEL_TSS,
        base: TSS as u64,
        limit: 0x67,
        flags: 0x008B,
    };
    cpu.gdtr.base = GDT as u64;
    cpu.gdtr.limit = 47;
    cpu.idtr.base = IDT as u64;
    cpu.idtr.limit = 0x7FF;
    cpu.rip = CODE as u64;
    cpu.set_gpr_u32(CpuState::RSP, 0x0000_8000);
    cpu.rflags = 0x202;

    step(&mut cpu, &mut bus).unwrap(); // INT 0x80 → ring 0
    assert_eq!(cpu.cs.selector & 3, 0);
    assert_eq!(cpu.rip, u64::from(HANDLER));

    // Patch the saved EIP on the kernel stack to USER_CONTINUE so IRETD
    // resumes at a distinct user address (simulating a syscall that advances).
    let esp = cpu.gpr_u32(CpuState::RSP);
    bus.poke_u32(esp as usize, USER_CONTINUE);

    step(&mut cpu, &mut bus).unwrap(); // IRETD → ring 3

    assert_eq!(cpu.cs.selector, SEL_UCODE);
    assert_eq!(cpu.cs.selector & 3, 3);
    assert_eq!(cpu.ss.selector, SEL_UDATA);
    assert_eq!(cpu.rip, u64::from(USER_CONTINUE));
    assert_eq!(cpu.gpr_u32(CpuState::RSP), 0x0000_8000);
    assert!(cpu.interrupt_flag(), "IRETD restores saved IF");
}

/// Outer `IRETD` with a mismatched return SS.RPL raises `#GP` and commits nothing.
#[test]
fn outer_iretd_rejects_bad_outer_ss_atomically() {
    let mut bus = RamBus::new(0x10000);
    install_tables(&mut bus);
    bus.write_bytes(CODE, &[0xCD, 0x80]);
    bus.mem[HANDLER as usize] = 0xCF;
    // #GP gate for the nested failure observation path — keep delivery local.
    bus.write_bytes(
        IDT + 13 * 8,
        &encode_idt_gate32(0x1F00, SEL_KCODE, 0x8E),
    );
    bus.mem[0x1F00] = 0xF4;

    let mut cpu = CpuState::reset();
    cpu.cr0 |= 1;
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
    cpu.tr = x86_core::SegmentReg {
        selector: SEL_TSS,
        base: TSS as u64,
        limit: 0x67,
        flags: 0x008B,
    };
    cpu.gdtr.base = GDT as u64;
    cpu.gdtr.limit = 47;
    cpu.idtr.base = IDT as u64;
    cpu.idtr.limit = 0x7FF;
    cpu.rip = CODE as u64;
    cpu.set_gpr_u32(CpuState::RSP, 0x0000_8000);
    cpu.rflags = 0x202;

    step(&mut cpu, &mut bus).unwrap(); // enter handler
    let esp = cpu.gpr_u32(CpuState::RSP);
    // Corrupt saved SS to kernel data with RPL 0 while CS.RPL stays 3.
    bus.poke_u32((esp + 16) as usize, u32::from(SEL_KDATA));
    let before = cpu.clone();

    // IRETD → #GP(selector) while still CPL 0; delivered through ring-0 gate.
    step(&mut cpu, &mut bus).unwrap();
    assert_eq!(cpu.rip, 0x1F00, "nested #GP handler entered");
    assert_eq!(before.ss.selector, SEL_KDATA);
    assert_ne!(cpu.gpr_u32(CpuState::RSP), 0x8000, "did not return to user stack");
}
