//! Round-13 slice 1: VME `CLI`/`STI` operate on `VIF` when `CR4.VME=1` and
//! `IOPL < 3` (SDM Vol. 3 Virtual-8086 Mode Extensions / Table 20-2).
//!
//! With `IOPL = 3`, `CLI`/`STI` still touch `IF`. Enabling interrupts while
//! `VIP=1` raises `#GP(0)` under VME (whether via `IF` or `VIF`).
//! `CPUID.01H:EDX.VME` remains clear.
//!
//! Spec: Intel SDM Vol. 2 "CLI"/"STI"; Vol. 3 §§20.2–20.3 Table 20-2;
//! Vol. 3 §2.5 (`CR4.VME`).

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
const MONITOR_CODE: usize = 0x1000;
const HANDLER_GP: u32 = 0x0000_1900;
const KERNEL_ESP0: u32 = 0x0000_9000;
const SEL_KCODE: u16 = 0x0008;
const SEL_KDATA: u16 = 0x0010;
const SEL_TSS: u16 = 0x0018;

const VM86_CS: u16 = 0x1000;
const VM86_IP: u16 = 0x0100;
const VM86_SS: u16 = 0x1000;
const VM86_SP: u16 = 0xFFFE;

const CR4_VME: u64 = 1;
const EFLAGS_VM: u64 = 1 << 17;
const EFLAGS_VIF: u64 = 1 << 19;
const EFLAGS_VIP: u64 = 1 << 20;

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
    [off[0], off[1], sel[0], sel[1], 0, access, off[2], off[3]]
}

fn enter_vm86(guest: &[u8], iopl: u8, cr4_vme: bool) -> (CpuState, RamBus) {
    let mut bus = RamBus::new(0x40000);
    bus.write_bytes(GDT, &[0u8; 8]);
    bus.write_bytes(GDT + 8, &encode_seg_desc(0, 0xF_FFFF, 0x9A, 0xC0));
    bus.write_bytes(GDT + 16, &encode_seg_desc(0, 0xF_FFFF, 0x93, 0xC0));
    bus.write_bytes(GDT + 24, &encode_seg_desc(TSS as u32, 0x67, 0x8B, 0));
    bus.poke_u32(TSS + 4, KERNEL_ESP0);
    bus.poke_u16(TSS + 8, SEL_KDATA);
    bus.write_bytes(
        IDT + 13 * 8,
        &encode_idt_gate32(HANDLER_GP, SEL_KCODE, 0x8E),
    );
    bus.mem[MONITOR_CODE] = 0xCF;
    bus.mem[HANDLER_GP as usize] = 0xF4;

    let linear = (u32::from(VM86_CS) << 4) + u32::from(VM86_IP);
    bus.write_bytes(linear as usize, guest);

    let eflags = 0x0002 | (u32::from(iopl) << 12) | (1 << 9) | (1 << 17);
    let frame = (KERNEL_ESP0 - 36) as usize;
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
    if cr4_vme {
        cpu.cr4 |= CR4_VME;
    }
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
    cpu.gdtr.limit = 31;
    cpu.idtr.base = IDT as u64;
    cpu.idtr.limit = 0x7FF;
    cpu.rip = MONITOR_CODE as u64;
    cpu.set_gpr_u32(CpuState::RSP, KERNEL_ESP0 - 36);
    cpu.rflags = 0x2;
    step(&mut cpu, &mut bus).unwrap();
    assert_ne!(cpu.rflags & EFLAGS_VM, 0);
    (cpu, bus)
}

/// VME + IOPL=0: `CLI` clears `VIF`, leaves `IF` alone.
#[test]
fn vme_cli_iopl0_clears_vif_not_if() {
    let (mut cpu, mut bus) = enter_vm86(&[0xFA, 0xF4], 0, true);
    cpu.rflags |= EFLAGS_VIF;
    assert!(cpu.interrupt_flag());

    step(&mut cpu, &mut bus).unwrap();

    assert_ne!(cpu.rflags & EFLAGS_VM, 0);
    assert!(cpu.interrupt_flag(), "IF sticky under VME CLI IOPL<3");
    assert_eq!(cpu.rflags & EFLAGS_VIF, 0, "CLI must clear VIF");
}

/// VME + IOPL=0: `STI` sets `VIF`, leaves `IF` alone.
#[test]
fn vme_sti_iopl0_sets_vif_not_if() {
    let (mut cpu, mut bus) = enter_vm86(&[0xFB, 0xF4], 0, true);
    cpu.set_interrupt_flag(false);
    assert_eq!(cpu.rflags & EFLAGS_VIF, 0);

    step(&mut cpu, &mut bus).unwrap();

    assert!(!cpu.interrupt_flag(), "IF must stay clear");
    assert_ne!(cpu.rflags & EFLAGS_VIF, 0, "STI must set VIF");
}

/// VME + IOPL=0: `STI` with `VIP=1` → `#GP(0)` (VIP∧VIF).
#[test]
fn vme_sti_iopl0_with_vip_raises_gp() {
    let (mut cpu, mut bus) = enter_vm86(&[0xFB], 0, true);
    cpu.set_interrupt_flag(false);
    cpu.rflags |= EFLAGS_VIP;

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.rip, u64::from(HANDLER_GP));
    assert_eq!(cpu.rflags & EFLAGS_VM, 0);
    assert_eq!(cpu.rflags & EFLAGS_VIF, 0, "failed STI must not set VIF");
}

/// VME + IOPL=3: `CLI`/`STI` still update `IF` (not VIF).
#[test]
fn vme_cli_sti_iopl3_update_if() {
    let (mut cpu, mut bus) = enter_vm86(&[0xFA, 0xFB, 0xF4], 3, true);
    assert!(cpu.interrupt_flag());
    assert_eq!(cpu.rflags & EFLAGS_VIF, 0);

    step(&mut cpu, &mut bus).unwrap(); // CLI
    assert!(!cpu.interrupt_flag());
    assert_eq!(cpu.rflags & EFLAGS_VIF, 0);

    step(&mut cpu, &mut bus).unwrap(); // STI
    assert!(cpu.interrupt_flag());
    assert_eq!(cpu.rflags & EFLAGS_VIF, 0);
}

/// VME + IOPL=3: `STI` with `VIP=1` → `#GP(0)` (VIP∧IF).
#[test]
fn vme_sti_iopl3_with_vip_raises_gp() {
    let (mut cpu, mut bus) = enter_vm86(&[0xFB], 3, true);
    cpu.set_interrupt_flag(false);
    cpu.rflags |= EFLAGS_VIP;

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.rip, u64::from(HANDLER_GP));
    assert!(!cpu.interrupt_flag(), "failed STI must not set IF");
}

/// Without `CR4.VME`, IOPL=0 `CLI` still `#GP` (R9 contract preserved).
#[test]
fn without_vme_cli_iopl0_still_gp() {
    let (mut cpu, mut bus) = enter_vm86(&[0xFA], 0, false);
    assert!(cpu.interrupt_flag());

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.rip, u64::from(HANDLER_GP));
    assert_eq!(cpu.rflags & EFLAGS_VM, 0, "monitor #GP leaves VM86");
}

/// CPUID.1:EDX.VME stays clear even after exercising VME `CLI`/`STI`.
#[test]
fn cpuid_vme_remains_clear_after_vme_cli() {
    let (mut cpu, mut bus) = enter_vm86(
        &[
            0xFA, // CLI (VIF path)
            0x66, 0xB8, 0x01, 0x00, 0x00, 0x00, // MOV EAX,1
            0x0F, 0xA2, // CPUID
            0xF4,
        ],
        0,
        true,
    );
    cpu.rflags |= EFLAGS_VIF;
    step(&mut cpu, &mut bus).unwrap(); // CLI
    step(&mut cpu, &mut bus).unwrap(); // MOV EAX,1
    step(&mut cpu, &mut bus).unwrap(); // CPUID
    assert_eq!(
        cpu.gpr_u32(CpuState::RDX) & (1 << 1),
        0,
        "CPUID.VME must stay clear"
    );
}
