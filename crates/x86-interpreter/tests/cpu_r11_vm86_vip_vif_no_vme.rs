//! Round-11 slice 2: VIP/VIF honesty **without** `CR4.VME`.
//!
//! Without VME, `POPF`/`POPFD` and `IRET`/`IRETD` in VM86 must **not** load
//! `VIP`/`VIF` from the stack image; those bits stay sticky. There is also no
//! VME redirect: `CLI`/`STI` still affect `IF` (IOPL=3), and enabling `IF`
//! while `VIP=1` does **not** raise `#GP` (that `#GP` is a VME feature).
//!
//! Spec: Intel SDM Vol. 2 "POPF/POPFD", "IRET/IRETD" (RETURN-FROM-VM86);
//! Vol. 3 §20.2 / Table 20-2 (VME=0); Vol. 3 §2.5 (CR4.VME); CPUID.1:EDX.VME
//! remains clear (honest).

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
const VM86_SS: u16 = 0x0100;
const VM86_SP: u16 = 0x8000;

const EFLAGS_IF: u64 = 1 << 9;
const EFLAGS_VM: u64 = 1 << 17;
const EFLAGS_VIF: u64 = 1 << 19;
const EFLAGS_VIP: u64 = 1 << 20;
const VIP_VIF: u64 = EFLAGS_VIP | EFLAGS_VIF;

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

fn enter_vm86(guest: &[u8], iopl: u8) -> (CpuState, RamBus) {
    let mut bus = RamBus::new(0x20000);
    bus.write_bytes(GDT, &[0u8; 8]);
    bus.write_bytes(GDT + 8, &encode_seg_desc(0, 0xF_FFFF, 0x9A, 0xC0));
    bus.write_bytes(GDT + 16, &encode_seg_desc(0, 0xF_FFFF, 0x93, 0xC0));
    bus.mem[MONITOR_CODE] = 0xCF;

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
    // CR4.VME intentionally clear; CPUID does not advertise VME.
    cpu.cr4 = 0;
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
    assert_ne!(cpu.rflags & EFLAGS_VM, 0);
    assert_eq!(cpu.cr4 & 1, 0, "CR4.VME must stay clear");
    (cpu, bus)
}

/// `POPFD` image trying to clear VIP/VIF leaves them sticky (no VME).
#[test]
fn vm86_popfd_keeps_vip_vif_sticky() {
    let (mut cpu, mut bus) = enter_vm86(&[0x66, 0x9D, 0xF4], 3); // POPFD
    cpu.rflags |= VIP_VIF;
    let sp = VM86_SP - 4;
    let base = (u32::from(VM86_SS) << 4) as usize;
    // Image: IOPL=3, IF=0, VIP=VIF=0 (attempt to clear sticky bits).
    bus.poke_u32(base + sp as usize, 0x0000_3002);
    cpu.set_gpr_u16(CpuState::RSP, sp);

    step(&mut cpu, &mut bus).unwrap();

    assert_ne!(cpu.rflags & EFLAGS_VM, 0);
    assert_eq!(cpu.rflags & VIP_VIF, VIP_VIF, "VIP|VIF sticky on POPFD");
    assert_eq!((cpu.rflags >> 12) & 3, 3, "IOPL sticky");
    assert_eq!(cpu.rflags & EFLAGS_IF, 0, "IF may change under IOPL=3");
}

/// `POPFD` image trying to *set* VIP/VIF also fails — bits never load.
#[test]
fn vm86_popfd_cannot_set_vip_vif_from_image() {
    let (mut cpu, mut bus) = enter_vm86(&[0x66, 0x9D, 0xF4], 3);
    assert_eq!(cpu.rflags & VIP_VIF, 0);
    let sp = VM86_SP - 4;
    let base = (u32::from(VM86_SS) << 4) as usize;
    // Image attempts VIP|VIF=1 plus IF=1, IOPL=3.
    bus.poke_u32(base + sp as usize, 0x0018_3202);
    cpu.set_gpr_u16(CpuState::RSP, sp);

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.rflags & VIP_VIF, 0, "VIP|VIF never load from POPFD");
    assert_ne!(cpu.rflags & EFLAGS_IF, 0);
}

/// `POPF` (16-bit) cannot touch VIP/VIF (high word); they remain.
#[test]
fn vm86_popf16_leaves_vip_vif_untouched() {
    let (mut cpu, mut bus) = enter_vm86(&[0x9D, 0xF4], 3);
    cpu.rflags |= VIP_VIF;
    let sp = VM86_SP - 2;
    let base = (u32::from(VM86_SS) << 4) as usize;
    bus.poke_u16(base + sp as usize, 0x3002); // IF=0, IOPL=3 in image
    cpu.set_gpr_u16(CpuState::RSP, sp);

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.rflags & VIP_VIF, VIP_VIF);
    assert_eq!(cpu.rflags & EFLAGS_IF, 0);
}

/// `IRETD` while VM=1 keeps VIP/VIF sticky (image VIP/VIF ignored).
#[test]
fn vm86_iretd_keeps_vip_vif_sticky() {
    let (mut cpu, mut bus) = enter_vm86(&[0x66, 0xCF], 3); // IRETD
    cpu.rflags |= VIP_VIF;
    let sp = VM86_SP - 12;
    let base = (u32::from(VM86_SS) << 4) as usize;
    let resume_ip = 0x0200u16;
    bus.poke_u32(base + sp as usize, u32::from(resume_ip));
    bus.poke_u32(base + sp as usize + 4, u32::from(VM86_CS));
    // Flags image: try clear VIP/VIF, clear IF, keep IOPL=3 / VM.
    bus.poke_u32(base + sp as usize + 8, 0x0002_3002 | (1 << 17));
    cpu.set_gpr_u16(CpuState::RSP, sp);
    let resume = (u32::from(VM86_CS) << 4) + u32::from(resume_ip);
    bus.mem[resume as usize] = 0xF4;

    step(&mut cpu, &mut bus).unwrap();

    assert_ne!(cpu.rflags & EFLAGS_VM, 0);
    assert_eq!(cpu.rflags & VIP_VIF, VIP_VIF, "VIP|VIF sticky on IRETD");
    assert_eq!((cpu.rflags >> 12) & 3, 3);
    assert_eq!(cpu.rip, u64::from(resume_ip));
}

/// Without VME: `STI` with `VIP=1` updates `IF` and does **not** `#GP`.
#[test]
fn vm86_sti_with_vip_set_no_vme_updates_if() {
    let (mut cpu, mut bus) = enter_vm86(&[0xFB, 0xF4], 3); // STI
    cpu.rflags |= EFLAGS_VIP;
    cpu.set_interrupt_flag(false);

    step(&mut cpu, &mut bus).unwrap();

    assert!(cpu.interrupt_flag(), "STI sets IF (no VIF path)");
    assert_ne!(cpu.rflags & EFLAGS_VIP, 0, "VIP remains");
    assert_eq!(cpu.rflags & EFLAGS_VIF, 0, "VIF not invented by STI");
    assert_ne!(cpu.rflags & EFLAGS_VM, 0);
}

/// Without VME: `CLI` with VIP/VIF set clears `IF`, not `VIF`.
#[test]
fn vm86_cli_with_vip_vif_clears_if_not_vif() {
    let (mut cpu, mut bus) = enter_vm86(&[0xFA, 0xF4], 3);
    cpu.rflags |= VIP_VIF;
    assert!(cpu.interrupt_flag());

    step(&mut cpu, &mut bus).unwrap();

    assert!(!cpu.interrupt_flag());
    assert_eq!(
        cpu.rflags & VIP_VIF,
        VIP_VIF,
        "CLI must not clear VIF without VME"
    );
}
