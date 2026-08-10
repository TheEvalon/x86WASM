//! Round-10 slice 2: far `JMP` while `EFLAGS.VM=1`.
//!
//! In virtual-8086 mode, far JMP is real-address-like: reload CS:IP with
//! `base = selector << 4` and remain in VM86 (`VM` stays set).
//!
//! Spec: Intel SDM Vol. 3 §20.1 / §20.1.3; Vol. 2 "JMP" (far); Vol. 3 §3.4.2.

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
const MONITOR_CODE: usize = 0x1000;
const KERNEL_ESP: u32 = 0x0000_9000;
const SEL_KCODE: u16 = 0x0008;
const SEL_KDATA: u16 = 0x0010;

const VM86_CS: u16 = 0x1000;
const VM86_IP: u16 = 0x0100;
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
    let mut bus = RamBus::new(0x80000);
    bus.write_bytes(GDT, &[0u8; 8]);
    bus.write_bytes(GDT + 8, &encode_seg_desc(0, 0xF_FFFF, 0x9A, 0xC0));
    bus.write_bytes(GDT + 16, &encode_seg_desc(0, 0xF_FFFF, 0x93, 0xC0));
    bus.mem[MONITOR_CODE] = 0xCF;

    let linear = (u32::from(VM86_CS) << 4) + u32::from(VM86_IP);
    bus.write_bytes(linear as usize, guest);

    let target = (u32::from(TARGET_CS) << 4) + u32::from(TARGET_IP);
    bus.mem[target as usize] = 0xF4; // HLT landing

    let eflags = 0x0002 | (3 << 12) | (1 << 9) | (1 << 17);
    let frame = (KERNEL_ESP - 36) as usize;
    bus.poke_u32(frame, u32::from(VM86_IP));
    bus.poke_u32(frame + 4, u32::from(VM86_CS));
    bus.poke_u32(frame + 8, eflags);
    bus.poke_u32(frame + 12, 0xFFFE);
    bus.poke_u32(frame + 16, 0x2000);
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
    cpu.rip = MONITOR_CODE as u64;
    cpu.set_gpr_u32(CpuState::RSP, KERNEL_ESP - 36);
    cpu.rflags = 0x2;
    step(&mut cpu, &mut bus).unwrap();
    assert_ne!(cpu.rflags & (1 << 17), 0);
    (cpu, bus)
}

/// Direct far `JMP ptr16:16` (`EA`) in VM86 reloads CS:IP and stays in VM86.
#[test]
fn vm86_far_jmp_direct_reloads_cs_ip_stays_vm() {
    // EA 00 02 00 18  — JMP 1800:0200
    let (mut cpu, mut bus) = enter_vm86(&[0xEA, 0x00, 0x02, 0x00, 0x18]);

    step(&mut cpu, &mut bus).unwrap();

    assert_ne!(cpu.rflags & (1 << 17), 0, "must stay in VM86");
    assert_eq!(cpu.cs.selector, TARGET_CS);
    assert_eq!(cpu.cs.base, u64::from(TARGET_CS) << 4);
    assert_eq!(cpu.rip, u64::from(TARGET_IP));

    step(&mut cpu, &mut bus).unwrap();
    assert!(cpu.halted, "fetch uses new CS base");
}

/// Indirect far `JMP m16:16` (Group 5 `/5`) in VM86.
#[test]
fn vm86_far_jmp_indirect_m16_16_stays_vm() {
    // Place pointer at DS:0x0100 with DS=0x4000 → linear 0x40100.
    // FF 2E 00 01  — JMP FAR [0x0100] (mod=00 r/m=110 disp16 under adsz16)
    let (mut cpu, mut bus) = enter_vm86(&[0xFF, 0x2E, 0x00, 0x01]);
    let ptr = (0x4000u32 << 4) + 0x0100;
    bus.poke_u16(ptr as usize, TARGET_IP);
    bus.poke_u16(ptr as usize + 2, TARGET_CS);

    step(&mut cpu, &mut bus).unwrap();

    assert_ne!(cpu.rflags & (1 << 17), 0);
    assert_eq!(cpu.cs.selector, TARGET_CS);
    assert_eq!(cpu.rip, u64::from(TARGET_IP));
}
