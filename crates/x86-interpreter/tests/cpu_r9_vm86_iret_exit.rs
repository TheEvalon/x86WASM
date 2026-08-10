//! Round-9 slice 4: VM86 `IRET` `#GP` path, stay-in-VM86 return, monitor exit.
//!
//! Spec: Intel SDM Vol. 2 "IRET/IRETD" RETURN-FROM-VIRTUAL-8086-MODE /
//! RETURN-TO-VIRTUAL-8086-MODE; Vol. 3 §20.2.3.

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
const VM86_CS: u16 = 0x1000;
const VM86_IP: u16 = 0x0100;
const VM86_SS: u16 = 0x0100;
const VM86_SP: u16 = 0x8000;
const RESUME_IP: u16 = 0x0200;

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
    assert_ne!(cpu.rflags & (1 << 17), 0);
    (cpu, bus)
}

/// VM86 + IOPL=0: `IRET` → `#GP(0)` (trap to monitor).
#[test]
fn vm86_iret_with_iopl_below_3_raises_gp0() {
    let (mut cpu, mut bus) = enter_vm86(0, &[0xCF]);
    let err = step(&mut cpu, &mut bus).expect_err("IRET");
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
    assert_ne!(cpu.rflags & (1 << 17), 0, "still in VM86");
}

/// VM86 + IOPL=3: `IRET` stays in VM86 and resumes at the popped CS:IP.
#[test]
fn vm86_iret_with_iopl3_stays_in_vm86() {
    let (mut cpu, mut bus) = enter_vm86(3, &[0xCF]);
    // Build a 16-bit IRET frame on the VM86 stack: IP, CS, FLAGS.
    let sp = VM86_SP - 6;
    let base = (u32::from(VM86_SS) << 4) as usize;
    bus.poke_u16(base + sp as usize, RESUME_IP);
    bus.poke_u16(base + sp as usize + 2, VM86_CS);
    bus.poke_u16(base + sp as usize + 4, 0x3202); // try IOPL=3 in image (sticky anyway)
    cpu.set_gpr_u16(CpuState::RSP, sp);
    // Resume target: HLT
    let resume_linear = (u32::from(VM86_CS) << 4) + u32::from(RESUME_IP);
    bus.mem[resume_linear as usize] = 0xF4;

    step(&mut cpu, &mut bus).unwrap();
    assert_ne!(cpu.rflags & (1 << 17), 0, "VM sticky");
    assert_eq!((cpu.rflags >> 12) & 3, 3, "IOPL sticky");
    assert_eq!(cpu.cs.selector, VM86_CS);
    assert_eq!(cpu.rip, u64::from(RESUME_IP));

    step(&mut cpu, &mut bus).unwrap();
    assert!(cpu.halted);
}

/// Successful exit: after VM86 enter, a CPL-0 monitor `IRETD` with `VM=0`
/// returns to protected mode (interrupt push of the 9-dword frame is still out).
#[test]
fn monitor_iretd_exits_vm86_to_protected_mode() {
    let (mut cpu, mut bus) = enter_vm86(3, &[0xF4]);
    assert_ne!(cpu.rflags & (1 << 17), 0);

    // Simulate monitor: CPL 0, clear VM, prepare a same-privilege protected
    // IRETD frame back to ring-0 code that HLTs.
    const EXIT_EIP: u32 = 0x1800;
    bus.mem[EXIT_EIP as usize] = 0xF4;
    bus.mem[CODE] = 0xCF; // IRETD at ring 0

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
    cpu.rflags = 0x2; // VM cleared — already in the monitor
    cpu.rip = CODE as u64;

    let frame = (KERNEL_ESP - 12) as usize;
    bus.poke_u32(frame, EXIT_EIP);
    bus.poke_u32(frame + 4, u32::from(SEL_KCODE));
    bus.poke_u32(frame + 8, 0x0002); // VM=0
    cpu.set_gpr_u32(CpuState::RSP, KERNEL_ESP - 12);

    step(&mut cpu, &mut bus).unwrap(); // IRETD → protected
    assert_eq!(cpu.rflags & (1 << 17), 0, "VM cleared on exit");
    assert_eq!(cpu.cs.selector, SEL_KCODE);
    assert_eq!(cpu.rip, u64::from(EXIT_EIP));
    assert_eq!(cpu.cs.base, 0, "protected CS cache, not real-mode shift");

    step(&mut cpu, &mut bus).unwrap();
    assert!(cpu.halted);
}
