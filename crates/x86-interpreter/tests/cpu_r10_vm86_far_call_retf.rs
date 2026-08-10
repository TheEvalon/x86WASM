//! Round-10 slice 3: far `CALL` / `RETF` while `EFLAGS.VM=1`.
//!
//! Virtual-8086 far CALL/RETF are real-address-like: push/pop CS:IP on the
//! VM86 stack with `base = selector << 4`, and remain in VM86.
//!
//! Spec: Intel SDM Vol. 2 "CALL"/"RET" (far); Vol. 3 §20.1 / §20.1.3; §3.4.2.

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
const MONITOR_CODE: usize = 0x1000;
const KERNEL_ESP: u32 = 0x0000_9000;
const SEL_KCODE: u16 = 0x0008;
const SEL_KDATA: u16 = 0x0010;

const VM86_CS: u16 = 0x1000;
const VM86_IP: u16 = 0x0100;
const VM86_SS: u16 = 0x0100;
const VM86_SP: u16 = 0x8000;
const VM86_DS: u16 = 0x3000;
const TARGET_CS: u16 = 0x1800;
const TARGET_IP: u16 = 0x0200;

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

fn enter_vm86(guest: &[u8]) -> (CpuState, RamBus) {
    // Cover CS/SS/DS/target linear addresses (SS:SP ≈ 0x9000; DS:0100 ≈ 0x30100).
    let mut bus = RamBus::new(0x40000);
    bus.write_bytes(GDT, &[0u8; 8]);
    bus.write_bytes(GDT + 8, &encode_seg_desc(0, 0xF_FFFF, 0x9A, 0xC0));
    bus.write_bytes(GDT + 16, &encode_seg_desc(0, 0xF_FFFF, 0x93, 0xC0));
    bus.mem[MONITOR_CODE] = 0xCF;

    let linear = (u32::from(VM86_CS) << 4) + u32::from(VM86_IP);
    bus.write_bytes(linear as usize, guest);

    let eflags = 0x0002 | (3 << 12) | (1 << 9) | (1 << 17);
    let frame = (KERNEL_ESP - 36) as usize;
    bus.poke_u32(frame, u32::from(VM86_IP));
    bus.poke_u32(frame + 4, u32::from(VM86_CS));
    bus.poke_u32(frame + 8, eflags);
    bus.poke_u32(frame + 12, u32::from(VM86_SP));
    bus.poke_u32(frame + 16, u32::from(VM86_SS));
    bus.poke_u32(frame + 20, 0x3000);
    bus.poke_u32(frame + 24, u32::from(VM86_DS));
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
    cpu.rip = MONITOR_CODE as u64;
    cpu.set_gpr_u32(CpuState::RSP, KERNEL_ESP - 36);
    cpu.rflags = 0x2;
    step(&mut cpu, &mut bus).unwrap();
    assert_ne!(cpu.rflags & (1 << 17), 0);
    (cpu, bus)
}

/// Direct far `CALL ptr16:16` (`9A`) pushes CS:IP and stays in VM86.
#[test]
fn vm86_far_call_direct_pushes_cs_ip_stays_vm() {
    // 9A 00 02 00 18 — CALL 1800:0200
    let (mut cpu, mut bus) = enter_vm86(&[0x9A, 0x00, 0x02, 0x00, 0x18]);
    let target = (u32::from(TARGET_CS) << 4) + u32::from(TARGET_IP);
    bus.mem[target as usize] = 0xF4;

    step(&mut cpu, &mut bus).unwrap();

    assert_ne!(cpu.rflags & (1 << 17), 0, "must stay in VM86");
    assert_eq!(cpu.cs.selector, TARGET_CS);
    assert_eq!(cpu.cs.base, u64::from(TARGET_CS) << 4);
    assert_eq!(cpu.rip, u64::from(TARGET_IP));
    assert_eq!(cpu.gpr_u16(CpuState::RSP), VM86_SP - 4);
    let sp_base = (u32::from(VM86_SS) << 4) as usize;
    let sp = cpu.gpr_u16(CpuState::RSP) as usize;
    // Low→high: return IP, then CS (Vol. 2 CALL far real/VM86).
    assert_eq!(bus.peek_u16(sp_base + sp), VM86_IP + 5);
    assert_eq!(bus.peek_u16(sp_base + sp + 2), VM86_CS);
}

/// `RETF` restores CS:IP from the far-call frame and stays in VM86.
#[test]
fn vm86_retf_restores_cs_ip_stays_vm() {
    let (mut cpu, mut bus) = enter_vm86(&[0xCB]); // RETF
    let resume_ip = 0x0300u16;
    let sp = VM86_SP - 4;
    let base = (u32::from(VM86_SS) << 4) as usize;
    bus.poke_u16(base + sp as usize, resume_ip);
    bus.poke_u16(base + sp as usize + 2, VM86_CS);
    cpu.set_gpr_u16(CpuState::RSP, sp);
    let resume = (u32::from(VM86_CS) << 4) + u32::from(resume_ip);
    bus.mem[resume as usize] = 0xF4;

    step(&mut cpu, &mut bus).unwrap();

    assert_ne!(cpu.rflags & (1 << 17), 0);
    assert_eq!(cpu.cs.selector, VM86_CS);
    assert_eq!(cpu.rip, u64::from(resume_ip));
    assert_eq!(cpu.gpr_u16(CpuState::RSP), VM86_SP);
}

/// Far CALL then RETF round-trip while VM=1.
#[test]
fn vm86_far_call_then_retf_round_trip() {
    // CALL target; target does RETF; resume is HLT after the CALL.
    let (mut cpu, mut bus) = enter_vm86(&[0x9A, 0x00, 0x02, 0x00, 0x18, 0xF4]);
    let target = (u32::from(TARGET_CS) << 4) + u32::from(TARGET_IP);
    bus.mem[target as usize] = 0xCB; // RETF

    step(&mut cpu, &mut bus).unwrap(); // CALL
    assert_eq!(cpu.cs.selector, TARGET_CS);
    step(&mut cpu, &mut bus).unwrap(); // RETF
    assert_ne!(cpu.rflags & (1 << 17), 0);
    assert_eq!(cpu.cs.selector, VM86_CS);
    assert_eq!(cpu.rip, u64::from(VM86_IP + 5));
    step(&mut cpu, &mut bus).unwrap(); // HLT
    assert!(cpu.halted);
}

/// Indirect far `CALL m16:16` (Group 5 `/3`) in VM86.
#[test]
fn vm86_far_call_indirect_m16_16_stays_vm() {
    // FF 1E 00 01 — CALL FAR [0x0100] (DS=0x3000 → linear 0x30100)
    let (mut cpu, mut bus) = enter_vm86(&[0xFF, 0x1E, 0x00, 0x01]);
    let ptr = (u32::from(VM86_DS) << 4) + 0x0100;
    bus.poke_u16(ptr as usize, TARGET_IP);
    bus.poke_u16(ptr as usize + 2, TARGET_CS);
    let target = (u32::from(TARGET_CS) << 4) + u32::from(TARGET_IP);
    bus.mem[target as usize] = 0xF4;

    step(&mut cpu, &mut bus).unwrap();

    assert_ne!(cpu.rflags & (1 << 17), 0);
    assert_eq!(cpu.cs.selector, TARGET_CS);
    assert_eq!(cpu.rip, u64::from(TARGET_IP));
    assert_eq!(cpu.gpr_u16(CpuState::RSP), VM86_SP - 4);
}
