//! Round-9 slice 1: enter virtual-8086 mode via `IRETD` from CPL 0.
//!
//! Spec: Intel SDM Vol. 2 "IRET/IRETD" (RETURN-TO-VIRTUAL-8086-MODE);
//! Vol. 3 §§20.2–20.3 / Figure 20-4 (9-dword PL0 stack); Vol. 3 §5.5 (CPL=3
//! while `EFLAGS.VM=1`).

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

const VM86_CS: u16 = 0x1000;
const VM86_IP: u16 = 0x0100;
const VM86_SS: u16 = 0x2000;
const VM86_SP: u16 = 0xFFFE;
const VM86_ES: u16 = 0x3000;
const VM86_DS: u16 = 0x4000;
const VM86_FS: u16 = 0x5000;
const VM86_GS: u16 = 0x6000;

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

/// CPL-0 `CS.D=1` fixture with a 9-dword VM86 return frame and `IRETD` at `CODE`.
fn vm86_enter_fixture(eflags_image: u32) -> (CpuState, RamBus) {
    let mut bus = RamBus::new(0x20000);
    bus.write_bytes(GDT, &[0u8; 8]);
    bus.write_bytes(GDT + 8, &encode_seg_desc(0, 0xF_FFFF, 0x9A, 0xC0));
    bus.write_bytes(GDT + 16, &encode_seg_desc(0, 0xF_FFFF, 0x93, 0xC0));
    bus.mem[CODE] = 0xCF; // IRETD under CS.D=1

    // Place a HLT at the VM86 resume point so a later step can observe it.
    let linear = (u32::from(VM86_CS) << 4) + u32::from(VM86_IP);
    bus.mem[linear as usize] = 0xF4;

    let frame = (KERNEL_ESP - 36) as usize;
    // Figure 20-4 / Vol. 2 IRET: EIP, CS, EFLAGS, ESP, SS, ES, DS, FS, GS.
    bus.poke_u32(frame, u32::from(VM86_IP));
    bus.poke_u32(frame + 4, u32::from(VM86_CS));
    bus.poke_u32(frame + 8, eflags_image);
    bus.poke_u32(frame + 12, u32::from(VM86_SP));
    bus.poke_u32(frame + 16, u32::from(VM86_SS));
    bus.poke_u32(frame + 20, u32::from(VM86_ES));
    bus.poke_u32(frame + 24, u32::from(VM86_DS));
    bus.poke_u32(frame + 28, u32::from(VM86_FS));
    bus.poke_u32(frame + 32, u32::from(VM86_GS));

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
    cpu.es = cpu.ss.clone();
    cpu.gdtr.base = GDT as u64;
    cpu.gdtr.limit = 23;
    cpu.rip = CODE as u64;
    cpu.set_gpr_u32(CpuState::RSP, KERNEL_ESP - 36);
    cpu.rflags = 0x2;
    (cpu, bus)
}

/// `IRETD` from CPL 0 with `EFLAGS.VM=1` loads the 9-dword frame and forces CPL 3.
#[test]
fn iretd_enters_vm86_with_nine_dword_frame_and_cpl3() {
    // IF=1, IOPL=3, bit1 reserved-1, VM=1.
    let eflags = 0x0002 | (3 << 12) | (1 << 9) | (1 << 17);
    let (mut cpu, mut bus) = vm86_enter_fixture(eflags);

    step(&mut cpu, &mut bus).unwrap();

    assert_ne!(cpu.rflags & (1 << 17), 0, "VM must be set");
    assert_eq!(cpu.cs.selector, VM86_CS);
    assert_eq!(cpu.cs.base, u64::from(VM86_CS) << 4);
    assert_eq!(cpu.rip, u64::from(VM86_IP));
    assert_eq!(cpu.ss.selector, VM86_SS);
    assert_eq!(cpu.ss.base, u64::from(VM86_SS) << 4);
    assert_eq!(cpu.gpr_u16(CpuState::RSP), VM86_SP);
    assert_eq!(cpu.es.selector, VM86_ES);
    assert_eq!(cpu.ds.selector, VM86_DS);
    assert_eq!(cpu.fs.selector, VM86_FS);
    assert_eq!(cpu.gs.selector, VM86_GS);
    assert_eq!(cpu.es.base, u64::from(VM86_ES) << 4);
    assert_eq!(cpu.ds.base, u64::from(VM86_DS) << 4);
    assert_eq!(cpu.fs.base, u64::from(VM86_FS) << 4);
    assert_eq!(cpu.gs.base, u64::from(VM86_GS) << 4);
    // Architectural CPL is 3 while VM=1 even when CS[1:0] is not 3.
    assert_eq!(VM86_CS & 3, 0, "fixture CS low bits are not RPL");
    assert!(
        cpu.interrupt_flag(),
        "EFLAGS.IF from the VM86 image must load"
    );
    assert_eq!((cpu.rflags >> 12) & 3, 3);

    // Resume one instruction in VM86 (HLT) to prove fetch uses real-mode CS base.
    step(&mut cpu, &mut bus).unwrap();
    assert!(cpu.halted);
}

/// Stack-limit fault while reading the 9-dword frame must not enter VM86.
#[test]
fn iretd_vm86_frame_reads_are_atomic() {
    let eflags = 0x0002 | (1 << 17);
    let (mut cpu, mut bus) = vm86_enter_fixture(eflags);
    // Only the first five dwords fit; sixth pop must `#SS(0)`.
    cpu.ss.limit = (KERNEL_ESP - 36 + 19) as u32;

    let err = step(&mut cpu, &mut bus).expect_err("truncated VM86 frame");
    assert!(
        matches!(
            err,
            ExecError::ArchFault {
                vector: 12,
                error_code: Some(0),
            } | ExecError::TripleFault { .. }
        ),
        "expected #SS(0) or delivery escalation, got {err:?}"
    );
    assert_eq!(cpu.rflags & (1 << 17), 0, "must not enter VM86 after fault");
    assert_eq!(cpu.cs.selector, SEL_KCODE);
}

/// EIP above the real-mode CS limit raises `#GP(0)` without entering VM86.
#[test]
fn iretd_vm86_rejects_eip_past_cs_limit() {
    let eflags = 0x0002 | (1 << 17);
    let (mut cpu, mut bus) = vm86_enter_fixture(eflags);
    let frame = (KERNEL_ESP - 36) as usize;
    bus.poke_u32(frame, 0x1_0000); // > 0xFFFF

    let err = step(&mut cpu, &mut bus).expect_err("EIP past limit");
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
    assert_eq!(cpu.rflags & (1 << 17), 0);
    assert_eq!(cpu.cs.selector, SEL_KCODE);
}
