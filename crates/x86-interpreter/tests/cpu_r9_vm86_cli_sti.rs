//! Round-9 slice 2: VM86-sensitive `CLI`/`STI` (IOPL).
//!
//! Without VME: in virtual-8086 mode, `IOPL < 3` → `#GP(0)`; `IOPL = 3` clears
//! or sets `IF`. Protected mode (not PVI): `CPL > IOPL` → `#GP(0)`.
//!
//! Spec: Intel SDM Vol. 2 "CLI"/"STI" Table 3-7; Vol. 3 ch.20 / §20.2.1.

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
    let mut bus = RamBus::new(0x20000);
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
    cpu.rip = CODE as u64;
    cpu.set_gpr_u32(CpuState::RSP, KERNEL_ESP - 36);
    cpu.rflags = 0x2;
    step(&mut cpu, &mut bus).unwrap();
    assert_ne!(cpu.rflags & (1 << 17), 0);
    (cpu, bus)
}

/// VM86 + IOPL=0: `CLI` raises `#GP(0)` and leaves IF set.
#[test]
fn vm86_cli_with_iopl_below_3_raises_gp0() {
    let (mut cpu, mut bus) = enter_vm86(0, &[0xFA]); // CLI
    assert!(cpu.interrupt_flag());

    let err = step(&mut cpu, &mut bus).expect_err("CLI must #GP");
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
    assert!(cpu.interrupt_flag(), "IF must remain set after failed CLI");
    assert_ne!(cpu.rflags & (1 << 17), 0);
}

/// VM86 + IOPL=0: `STI` raises `#GP(0)`.
#[test]
fn vm86_sti_with_iopl_below_3_raises_gp0() {
    let (mut cpu, mut bus) = enter_vm86(0, &[0xFB]);
    cpu.set_interrupt_flag(false);

    let err = step(&mut cpu, &mut bus).expect_err("STI must #GP");
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
    assert!(!cpu.interrupt_flag());
}

/// VM86 + IOPL=3: `CLI`/`STI` update IF.
#[test]
fn vm86_cli_sti_with_iopl3_update_if() {
    let (mut cpu, mut bus) = enter_vm86(3, &[0xFA, 0xFB, 0xF4]);
    assert!(cpu.interrupt_flag());
    step(&mut cpu, &mut bus).unwrap(); // CLI
    assert!(!cpu.interrupt_flag());
    step(&mut cpu, &mut bus).unwrap(); // STI
    assert!(cpu.interrupt_flag());
}

/// Protected mode ring 3 with IOPL=0: `CLI` → `#GP(0)` (no PVI).
#[test]
fn protected_cli_when_cpl_gt_iopl_raises_gp0() {
    let mut bus = RamBus::new(0x10000);
    bus.write_bytes(GDT, &[0u8; 8]);
    bus.write_bytes(GDT + 8, &encode_seg_desc(0, 0xF_FFFF, 0x9A, 0xC0));
    bus.write_bytes(GDT + 16, &encode_seg_desc(0, 0xF_FFFF, 0x93, 0xC0));
    bus.write_bytes(GDT + 24, &encode_seg_desc(0, 0xF_FFFF, 0xFA, 0xC0));
    bus.write_bytes(GDT + 32, &encode_seg_desc(0, 0xF_FFFF, 0xF3, 0xC0));
    bus.mem[CODE] = 0xFA; // CLI

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
    cpu.gdtr.base = GDT as u64;
    cpu.gdtr.limit = 39;
    cpu.rip = CODE as u64;
    cpu.set_gpr_u32(CpuState::RSP, 0x8000);
    cpu.rflags = 0x202; // IF=1, IOPL=0
    assert!(cpu.interrupt_flag());

    let err = step(&mut cpu, &mut bus).expect_err("ring3 CLI");
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
    assert!(cpu.interrupt_flag());
}
