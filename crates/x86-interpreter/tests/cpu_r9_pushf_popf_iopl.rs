//! Round-9 slice 3: `PUSHF`/`POPF` IOPL privilege in PM and VM86.
//!
//! Spec: Intel SDM Vol. 2 "PUSHF/PUSHFD", "POPF/POPFD"; Vol. 3 §20.2.2
//! (VM86 without VME: IOPL<3 → `#GP(0)`).

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

    fn poke_u16(&mut self, addr: usize, value: u16) {
        self.write_bytes(addr, &value.to_le_bytes());
    }

    fn poke_u32(&mut self, addr: usize, value: u32) {
        self.write_bytes(addr, &value.to_le_bytes());
    }

    fn peek_u16(&self, addr: usize) -> u16 {
        u16::from_le_bytes([self.mem[addr], self.mem[addr + 1]])
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
const CODE: usize = 0x1000;
const KERNEL_ESP: u32 = 0x0000_9000;
const SEL_KCODE: u16 = 0x0008;
const SEL_KDATA: u16 = 0x0010;
const SEL_UCODE: u16 = 0x001B;
const SEL_UDATA: u16 = 0x0023;
const VM86_CS: u16 = 0x1000;
const VM86_IP: u16 = 0x0100;
const VM86_SS: u16 = 0x2000;
/// Keep SP mid-segment so SS:SP linear stays inside a 256 KiB lab RAM image.
const VM86_SP: u16 = 0x7FFE;

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

fn enter_vm86(iopl: u8, guest: &[u8]) -> (CpuState, RamBus) {
    let mut bus = RamBus::new(0x40000);
    bus.write_bytes(GDT, &[0u8; 8]);
    bus.write_bytes(GDT + 8, &encode_seg_desc(0, 0xF_FFFF, 0x9A, 0xC0));
    bus.write_bytes(GDT + 16, &encode_seg_desc(0, 0xF_FFFF, 0x93, 0xC0));
    bus.mem[CODE] = 0xCF;

    let linear = (u32::from(VM86_CS) << 4) + u32::from(VM86_IP);
    bus.write_bytes(linear as usize, guest);

    let eflags = 0x0002 | (u32::from(iopl) << 12) | (1 << 9) | (1 << 17);
    let frame = (KERNEL_ESP - 36) as usize;
    bus.poke_u32(frame, u32::from(VM86_IP));
    bus.poke_u32(frame + 4, u32::from(VM86_CS));
    bus.poke_u32(frame + 8, eflags);
    bus.poke_u32(frame + 12, u32::from(VM86_SP));
    bus.poke_u32(frame + 16, u32::from(VM86_SS));
    bus.poke_u32(frame + 20, 0x3000);
    bus.poke_u32(frame + 24, 0x4000);
    bus.poke_u32(frame + 28, 0x5000);
    bus.poke_u32(frame + 32, 0x6000);

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
    cpu.gdtr.base = GDT as u64;
    cpu.gdtr.limit = 23;
    cpu.rip = CODE as u64;
    cpu.set_gpr_u32(CpuState::RSP, KERNEL_ESP - 36);
    cpu.rflags = 0x2;
    step(&mut cpu, &mut bus).unwrap();
    (cpu, bus)
}

fn ring3_fixture(code: &[u8], flags: u64) -> (CpuState, RamBus) {
    let mut bus = RamBus::new(0x10000);
    bus.write_bytes(GDT, &[0u8; 8]);
    bus.write_bytes(GDT + 8, &encode_seg_desc(0, 0xF_FFFF, 0x9A, 0xC0));
    bus.write_bytes(GDT + 16, &encode_seg_desc(0, 0xF_FFFF, 0x93, 0xC0));
    bus.write_bytes(GDT + 24, &encode_seg_desc(0, 0xF_FFFF, 0xFA, 0xC0));
    bus.write_bytes(GDT + 32, &encode_seg_desc(0, 0xF_FFFF, 0xF3, 0xC0));
    bus.write_bytes(CODE, code);

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
    cpu.gdtr.base = GDT as u64;
    cpu.gdtr.limit = 39;
    cpu.rip = CODE as u64;
    cpu.set_gpr_u32(CpuState::RSP, 0x8000);
    cpu.rflags = flags;
    (cpu, bus)
}

fn assert_gp0(err: ExecError) {
    assert!(
        matches!(
            err,
            ExecError::ArchFault {
                vector: 13,
                error_code: Some(0),
            } | ExecError::TripleFault { .. }
        ),
        "got {err:?}"
    );
}

/// VM86 + IOPL=0: `PUSHF` → `#GP(0)`.
#[test]
fn vm86_pushf_with_iopl_below_3_raises_gp0() {
    let (mut cpu, mut bus) = enter_vm86(0, &[0x9C]);
    let sp_before = cpu.gpr_u16(CpuState::RSP);
    let err = step(&mut cpu, &mut bus).expect_err("PUSHF");
    assert_gp0(err);
    assert_eq!(cpu.gpr_u16(CpuState::RSP), sp_before);
}

/// VM86 + IOPL=3: `PUSHF` reflects IOPL on the stack.
#[test]
fn vm86_pushf_with_iopl3_reflects_iopl() {
    let (mut cpu, mut bus) = enter_vm86(3, &[0x9C, 0xF4]);
    step(&mut cpu, &mut bus).unwrap();
    let sp = cpu.gpr_u16(CpuState::RSP);
    let linear = (u32::from(VM86_SS) << 4) + u32::from(sp);
    let pushed = bus.peek_u16(linear as usize);
    assert_eq!((pushed >> 12) & 3, 3);
    assert_ne!(pushed & (1 << 9), 0);
}

/// VM86 + IOPL=0: `POPF` → `#GP(0)`.
#[test]
fn vm86_popf_with_iopl_below_3_raises_gp0() {
    let (mut cpu, mut bus) = enter_vm86(0, &[0x9D]);
    let linear = (u32::from(VM86_SS) << 4) + u32::from(VM86_SP) - 2;
    bus.poke_u16(linear as usize, 0x3202); // try IOPL=3, IF=1
    cpu.set_gpr_u16(CpuState::RSP, VM86_SP - 2);

    let err = step(&mut cpu, &mut bus).expect_err("POPF");
    assert_gp0(err);
    assert_eq!((cpu.rflags >> 12) & 3, 0);
}

/// VM86 + IOPL=3: `POPF` may clear IF; IOPL stays 3.
#[test]
fn vm86_popf_with_iopl3_clears_if_keeps_iopl() {
    let (mut cpu, mut bus) = enter_vm86(3, &[0x9D, 0xF4]);
    let linear = (u32::from(VM86_SS) << 4) + u32::from(VM86_SP) - 2;
    bus.poke_u16(linear as usize, 0x3002); // IOPL=3 in image (ignored), IF=0
    cpu.set_gpr_u16(CpuState::RSP, VM86_SP - 2);
    assert!(cpu.interrupt_flag());

    step(&mut cpu, &mut bus).unwrap();
    assert!(!cpu.interrupt_flag(), "IF cleared by POPF");
    assert_eq!((cpu.rflags >> 12) & 3, 3, "IOPL sticky under VM86");
    assert!(cpu.rflags & (1 << 17) != 0, "VM sticky");
}

/// Ring-3 POPF cannot raise IOPL (no exception; privileged bits ignored).
#[test]
fn protected_popf_cannot_raise_iopl_from_ring3() {
    // IOPL=0 at CPL=3; stack image tries IOPL=3 and clears IF.
    let (mut cpu, mut bus) = ring3_fixture(&[0x9D, 0xF4], 0x202);
    bus.poke_u32(0x7FFC, 0x3202); // IOPL=3, IF=1, bit1
    cpu.set_gpr_u32(CpuState::RSP, 0x7FFC);

    step(&mut cpu, &mut bus).unwrap();
    assert_eq!((cpu.rflags >> 12) & 3, 0, "IOPL must stay 0");
    // CPL > IOPL: IF also unchanged by POPF.
    assert!(cpu.interrupt_flag(), "IF unchanged when CPL > IOPL");
}

/// Ring-3 PUSHF reflects current IOPL.
#[test]
fn protected_pushf_reflects_iopl_at_ring3() {
    let (mut cpu, mut bus) = ring3_fixture(&[0x9C, 0xF4], 0x202 | (2 << 12));
    step(&mut cpu, &mut bus).unwrap();
    let esp = cpu.gpr_u32(CpuState::RSP) as usize;
    let pushed = u32::from_le_bytes(bus.mem[esp..esp + 4].try_into().unwrap());
    assert_eq!((pushed >> 12) & 3, 2);
}
