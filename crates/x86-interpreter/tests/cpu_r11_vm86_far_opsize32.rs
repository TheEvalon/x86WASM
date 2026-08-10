//! Round-11 slice 4: operand-size 32 far `JMP` / `CALL` / `RETF` while VM=1.
//!
//! With `66H` (or a D=1 code segment), far transfers use the real-address-like
//! `ptr16:32` / `m16:32` forms: offset is truncated to IP16 on commit; CALL
//! pushes a 6-byte frame (EIP32 then CS16); RETF pops that frame. Execution
//! **stays** in VM86 (`EFLAGS.VM` sticky). This is the SDM-clear subset; it is
//! not a privileged exit path.
//!
//! Spec: Intel SDM Vol. 2 "JMP"/"CALL"/"RET" (far) + Ch. 2 (66H);
//! Vol. 3 §20.1 / §20.1.3; §3.4.2.

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
    bus.poke_u32(frame + 20, 0x5000);
    bus.poke_u32(frame + 24, u32::from(VM86_DS));
    bus.poke_u32(frame + 28, 0x6000);
    bus.poke_u32(frame + 32, 0x7000);

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
    // VM86 code cache is real-mode-like (D=0); opsize-32 needs 66H.
    assert_eq!(cpu.cs.flags & (1 << 14), 0, "CS.D clear in VM86");
    (cpu, bus)
}

/// `66 EA` far `JMP ptr16:32` truncates offset to IP16 and stays in VM86.
#[test]
fn vm86_opsize32_far_jmp_direct_stays_vm() {
    // 66 EA 00 02 00 00 00 18 — JMP 1800:00000200 (high half of offset ignored)
    let (mut cpu, mut bus) = enter_vm86(&[
        0x66, 0xEA, 0x00, 0x02, 0x00, 0x00, 0x00, 0x18,
    ]);
    let target = (u32::from(TARGET_CS) << 4) + u32::from(TARGET_IP);
    bus.mem[target as usize] = 0xF4;

    step(&mut cpu, &mut bus).unwrap();

    assert_ne!(cpu.rflags & (1 << 17), 0);
    assert_eq!(cpu.cs.selector, TARGET_CS);
    assert_eq!(cpu.cs.base, u64::from(TARGET_CS) << 4);
    assert_eq!(cpu.rip, u64::from(TARGET_IP));
}

/// High half of a `ptr16:32` offset is truncated (not a #GP exit).
#[test]
fn vm86_opsize32_far_jmp_truncates_high_offset() {
    // Offset 0x0001_0200 → IP = 0x0200 after truncate.
    let (mut cpu, mut bus) = enter_vm86(&[
        0x66, 0xEA, 0x00, 0x02, 0x01, 0x00, 0x00, 0x18,
    ]);
    let target = (u32::from(TARGET_CS) << 4) + u32::from(TARGET_IP);
    bus.mem[target as usize] = 0xF4;

    step(&mut cpu, &mut bus).unwrap();

    assert_ne!(cpu.rflags & (1 << 17), 0);
    assert_eq!(cpu.rip, u64::from(TARGET_IP));
}

/// Indirect `66 FF /5` far `JMP m16:32` in VM86.
#[test]
fn vm86_opsize32_far_jmp_indirect_m16_32_stays_vm() {
    // 66 FF 2E 00 01 — JMP FAR dword [0x0100] (DS=0x3000 → 0x30100)
    let (mut cpu, mut bus) = enter_vm86(&[0x66, 0xFF, 0x2E, 0x00, 0x01]);
    let ptr = (u32::from(VM86_DS) << 4) + 0x0100;
    bus.poke_u32(ptr as usize, u32::from(TARGET_IP));
    bus.poke_u16(ptr as usize + 4, TARGET_CS);
    let target = (u32::from(TARGET_CS) << 4) + u32::from(TARGET_IP);
    bus.mem[target as usize] = 0xF4;

    step(&mut cpu, &mut bus).unwrap();

    assert_ne!(cpu.rflags & (1 << 17), 0);
    assert_eq!(cpu.cs.selector, TARGET_CS);
    assert_eq!(cpu.rip, u64::from(TARGET_IP));
}

/// `66 9A` far `CALL ptr16:32` pushes 6-byte frame; stays in VM86.
#[test]
fn vm86_opsize32_far_call_pushes_eip32_cs_stays_vm() {
    // 66 9A 00 02 00 00 00 18 — CALL 1800:00000200
    let (mut cpu, mut bus) = enter_vm86(&[
        0x66, 0x9A, 0x00, 0x02, 0x00, 0x00, 0x00, 0x18,
    ]);
    let target = (u32::from(TARGET_CS) << 4) + u32::from(TARGET_IP);
    bus.mem[target as usize] = 0xF4;

    step(&mut cpu, &mut bus).unwrap();

    assert_ne!(cpu.rflags & (1 << 17), 0);
    assert_eq!(cpu.cs.selector, TARGET_CS);
    assert_eq!(cpu.rip, u64::from(TARGET_IP));
    // Push CS (−2) then EIP32 (−4) → SP − 6.
    assert_eq!(cpu.gpr_u16(CpuState::RSP), VM86_SP - 6);
    let base = (u32::from(VM86_SS) << 4) as usize;
    let sp = cpu.gpr_u16(CpuState::RSP) as usize;
    assert_eq!(bus.peek_u32(base + sp), u32::from(VM86_IP) + 8); // after 66 9A …
    assert_eq!(bus.peek_u16(base + sp + 4), VM86_CS);
}

/// `66 CB` RETF pops EIP32+CS16 and stays in VM86.
#[test]
fn vm86_opsize32_retf_restores_cs_ip_stays_vm() {
    let (mut cpu, mut bus) = enter_vm86(&[0x66, 0xCB]);
    let resume_ip = 0x0300u16;
    let sp = VM86_SP - 6;
    let base = (u32::from(VM86_SS) << 4) as usize;
    bus.poke_u32(base + sp as usize, u32::from(resume_ip));
    bus.poke_u16(base + sp as usize + 4, VM86_CS);
    cpu.set_gpr_u16(CpuState::RSP, sp);
    let resume = (u32::from(VM86_CS) << 4) + u32::from(resume_ip);
    bus.mem[resume as usize] = 0xF4;

    step(&mut cpu, &mut bus).unwrap();

    assert_ne!(cpu.rflags & (1 << 17), 0);
    assert_eq!(cpu.cs.selector, VM86_CS);
    assert_eq!(cpu.rip, u64::from(resume_ip));
    assert_eq!(cpu.gpr_u16(CpuState::RSP), VM86_SP);
}

/// Opsize-32 far CALL then RETF round-trip while VM=1.
#[test]
fn vm86_opsize32_far_call_retf_round_trip() {
    let (mut cpu, mut bus) = enter_vm86(&[
        0x66, 0x9A, 0x00, 0x02, 0x00, 0x00, 0x00, 0x18, 0xF4,
    ]);
    let target = (u32::from(TARGET_CS) << 4) + u32::from(TARGET_IP);
    bus.mem[target as usize] = 0x66;
    bus.mem[target as usize + 1] = 0xCB;

    step(&mut cpu, &mut bus).unwrap(); // CALL
    assert_eq!(cpu.cs.selector, TARGET_CS);
    step(&mut cpu, &mut bus).unwrap(); // RETF
    assert_ne!(cpu.rflags & (1 << 17), 0);
    assert_eq!(cpu.cs.selector, VM86_CS);
    assert_eq!(cpu.rip, u64::from(VM86_IP) + 8);
}

/// Indirect `66 FF /3` far `CALL m16:32` in VM86.
#[test]
fn vm86_opsize32_far_call_indirect_m16_32_stays_vm() {
    let (mut cpu, mut bus) = enter_vm86(&[0x66, 0xFF, 0x1E, 0x00, 0x01]);
    let ptr = (u32::from(VM86_DS) << 4) + 0x0100;
    bus.poke_u32(ptr as usize, u32::from(TARGET_IP));
    bus.poke_u16(ptr as usize + 4, TARGET_CS);
    let target = (u32::from(TARGET_CS) << 4) + u32::from(TARGET_IP);
    bus.mem[target as usize] = 0xF4;

    step(&mut cpu, &mut bus).unwrap();

    assert_ne!(cpu.rflags & (1 << 17), 0);
    assert_eq!(cpu.cs.selector, TARGET_CS);
    assert_eq!(cpu.gpr_u16(CpuState::RSP), VM86_SP - 6);
}
