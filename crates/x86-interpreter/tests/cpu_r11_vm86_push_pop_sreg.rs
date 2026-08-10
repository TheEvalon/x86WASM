//! Round-11 slice 1: VM86 `PUSH`/`POP` segment registers.
//!
//! While `EFLAGS.VM=1`, segment register loads are real-address-like
//! (`base = selector << 4`); they must **not** consult the GDT. PUSH merely
//! writes the selector; POP reloads the cache and stays in VM86.
//!
//! Spec: Intel SDM Vol. 3 §20.1 / §20.1.1; Vol. 2 "PUSH"/"POP" (Sreg);
//! Vol. 3 §3.4.2 (real-address base).

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
const VM86_DS: u16 = 0x4000;
const NEW_DS: u16 = 0x3000;
const NEW_ES: u16 = 0x3500;
const NEW_SS: u16 = 0x2200;

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
    // Tiny GDT: only kernel code/data. VM86 selectors like 0x3000 are *not*
    // present — a PE=1 GDT path would #GP; real-mode-like must succeed.
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
    bus.poke_u32(frame + 20, 0x5000); // ES
    bus.poke_u32(frame + 24, u32::from(VM86_DS));
    bus.poke_u32(frame + 28, 0x6000); // FS
    bus.poke_u32(frame + 32, 0x7000); // GS

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

/// `PUSH DS` writes the selector and stays in VM86.
#[test]
fn vm86_push_ds_writes_selector_stays_vm() {
    let (mut cpu, mut bus) = enter_vm86(&[0x1E, 0xF4]); // PUSH DS; HLT

    step(&mut cpu, &mut bus).unwrap();

    assert_ne!(cpu.rflags & (1 << 17), 0);
    assert_eq!(cpu.gpr_u16(CpuState::RSP), VM86_SP - 2);
    let sp_base = (u32::from(VM86_SS) << 4) as usize;
    assert_eq!(
        bus.peek_u16(sp_base + (VM86_SP as usize - 2)),
        VM86_DS
    );
}

/// `PUSH ES` / `PUSH SS` / `PUSH CS` likewise stay in VM86.
#[test]
fn vm86_push_es_ss_cs_stay_vm() {
    let (mut cpu, mut bus) = enter_vm86(&[0x06, 0x16, 0x0E, 0xF4]);
    step(&mut cpu, &mut bus).unwrap(); // PUSH ES
    step(&mut cpu, &mut bus).unwrap(); // PUSH SS
    step(&mut cpu, &mut bus).unwrap(); // PUSH CS
    assert_ne!(cpu.rflags & (1 << 17), 0);
    assert_eq!(cpu.gpr_u16(CpuState::RSP), VM86_SP - 6);
    let base = (u32::from(VM86_SS) << 4) as usize;
    assert_eq!(bus.peek_u16(base + VM86_SP as usize - 2), 0x5000); // ES
    assert_eq!(bus.peek_u16(base + VM86_SP as usize - 4), VM86_SS);
    assert_eq!(bus.peek_u16(base + VM86_SP as usize - 6), VM86_CS);
}

/// `POP DS` reloads `selector<<4` without GDT lookup; stays in VM86.
#[test]
fn vm86_pop_ds_real_mode_like_stays_vm() {
    let (mut cpu, mut bus) = enter_vm86(&[0x1F, 0xF4]); // POP DS
    let sp = VM86_SP - 2;
    let base = (u32::from(VM86_SS) << 4) as usize;
    bus.poke_u16(base + sp as usize, NEW_DS);
    cpu.set_gpr_u16(CpuState::RSP, sp);

    step(&mut cpu, &mut bus).unwrap();

    assert_ne!(cpu.rflags & (1 << 17), 0, "must stay in VM86");
    assert_eq!(cpu.ds.selector, NEW_DS);
    assert_eq!(cpu.ds.base, u64::from(NEW_DS) << 4);
    assert_eq!(cpu.gpr_u16(CpuState::RSP), VM86_SP);
}

/// `POP ES` same real-mode-like path.
#[test]
fn vm86_pop_es_real_mode_like_stays_vm() {
    let (mut cpu, mut bus) = enter_vm86(&[0x07, 0xF4]);
    let sp = VM86_SP - 2;
    let base = (u32::from(VM86_SS) << 4) as usize;
    bus.poke_u16(base + sp as usize, NEW_ES);
    cpu.set_gpr_u16(CpuState::RSP, sp);

    step(&mut cpu, &mut bus).unwrap();

    assert_ne!(cpu.rflags & (1 << 17), 0);
    assert_eq!(cpu.es.selector, NEW_ES);
    assert_eq!(cpu.es.base, u64::from(NEW_ES) << 4);
}

/// `POP SS` reloads SS real-mode-like (no GDT writable-data check).
#[test]
fn vm86_pop_ss_real_mode_like_stays_vm() {
    let (mut cpu, mut bus) = enter_vm86(&[0x17, 0xF4]);
    let sp = VM86_SP - 2;
    let base = (u32::from(VM86_SS) << 4) as usize;
    bus.poke_u16(base + sp as usize, NEW_SS);
    cpu.set_gpr_u16(CpuState::RSP, sp);

    step(&mut cpu, &mut bus).unwrap();

    assert_ne!(cpu.rflags & (1 << 17), 0);
    assert_eq!(cpu.ss.selector, NEW_SS);
    assert_eq!(cpu.ss.base, u64::from(NEW_SS) << 4);
    // SP was committed on the *old* SS before the reload.
    assert_eq!(cpu.gpr_u16(CpuState::RSP), VM86_SP);
}

/// `PUSH DS` / `POP DS` round-trip while VM=1.
#[test]
fn vm86_push_pop_ds_round_trip() {
    let (mut cpu, mut bus) = enter_vm86(&[0x1E, 0x1F, 0xF4]);
    let before = cpu.ds.selector;
    step(&mut cpu, &mut bus).unwrap();
    cpu.ds.load_real_mode_selector(0x1111); // clobber
    step(&mut cpu, &mut bus).unwrap();
    assert_ne!(cpu.rflags & (1 << 17), 0);
    assert_eq!(cpu.ds.selector, before);
    assert_eq!(cpu.ds.base, u64::from(before) << 4);
}

/// Two-byte `PUSH FS` / `POP FS` also real-mode-like in VM86.
#[test]
fn vm86_push_pop_fs_real_mode_like() {
    let (mut cpu, mut bus) = enter_vm86(&[0x0F, 0xA0, 0x0F, 0xA1, 0xF4]);
    let before = cpu.fs.selector;
    step(&mut cpu, &mut bus).unwrap(); // PUSH FS
    assert_eq!(cpu.gpr_u16(CpuState::RSP), VM86_SP - 2);
    cpu.fs.load_real_mode_selector(0x2222);
    step(&mut cpu, &mut bus).unwrap(); // POP FS
    assert_ne!(cpu.rflags & (1 << 17), 0);
    assert_eq!(cpu.fs.selector, before);
    assert_eq!(cpu.fs.base, u64::from(before) << 4);
}
