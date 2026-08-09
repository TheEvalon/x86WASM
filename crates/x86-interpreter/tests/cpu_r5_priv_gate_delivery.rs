//! Round-5 slice 2: privilege-changing interrupt/trap gate delivery via TSS.
//!
//! A ring-3 guest taking a ring-0 386 interrupt gate loads `SS0:ESP0` from the
//! TSS, pushes the outer `SS:ESP`, then the ordinary frame. Same-CPL delivery
//! remains intact.
//!
//! Spec: Intel SDM Vol. 3 §6.12.1 Figure 6-5; §7.2.1 (TSS stack fields).

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

    fn peek_u32(&self, addr: usize) -> u32 {
        u32::from_le_bytes([
            self.mem[addr],
            self.mem[addr + 1],
            self.mem[addr + 2],
            self.mem[addr + 3],
        ])
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
const CODE: usize = 0x1000;
const HANDLER: u32 = 0x0000_1800;
const KERNEL_ESP0: u32 = 0x0000_9000;
const SEL_KCODE: u16 = 0x0008;
const SEL_KDATA: u16 = 0x0010;
const SEL_TSS: u16 = 0x0018;
const SEL_UCODE: u16 = 0x0023; // index 4, RPL=3
const SEL_UDATA: u16 = 0x002B; // index 5, RPL=3

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

fn install_tables(bus: &mut RamBus) {
    bus.write_bytes(GDT, &[0u8; 8]);
    // 0x08 kernel code D=1
    bus.write_bytes(GDT + 8, &encode_seg_desc(0, 0xF_FFFF, 0x9A, 0xC0));
    // 0x10 kernel data B=1
    bus.write_bytes(GDT + 16, &encode_seg_desc(0, 0xF_FFFF, 0x93, 0xC0));
    // 0x18 busy 32-bit TSS (as after LTR)
    bus.write_bytes(GDT + 24, &encode_seg_desc(TSS as u32, 0x67, 0x8B, 0));
    // 0x20 user code D=1 DPL=3
    bus.write_bytes(GDT + 32, &encode_seg_desc(0, 0xF_FFFF, 0xFA, 0xC0));
    // 0x28 user data B=1 DPL=3
    bus.write_bytes(GDT + 40, &encode_seg_desc(0, 0xF_FFFF, 0xF3, 0xC0));

    bus.poke_u32(TSS + 4, KERNEL_ESP0);
    bus.poke_u16(TSS + 8, SEL_KDATA);

    // Vector 0x80: 386 interrupt gate to kernel code, DPL=3 (software INT).
    bus.write_bytes(IDT + 0x80 * 8, &encode_idt_gate32(HANDLER, SEL_KCODE, 0xEE));
    // Vector 14: #PF trap gate (error code), DPL=0.
    bus.write_bytes(IDT + 14 * 8, &encode_idt_gate32(HANDLER, SEL_KCODE, 0x8F));
    // Vector 6: same-CPL #UD for the ring-0 regression.
    bus.write_bytes(IDT + 6 * 8, &encode_idt_gate32(HANDLER, SEL_KCODE, 0x8E));
}

fn user_fixture(code: &[u8]) -> (CpuState, RamBus) {
    let mut bus = RamBus::new(0x10000);
    install_tables(&mut bus);
    bus.write_bytes(CODE, code);
    bus.mem[HANDLER as usize] = 0xF4;

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
    cpu.tr = x86_core::SegmentReg {
        selector: SEL_TSS,
        base: TSS as u64,
        limit: 0x67,
        flags: 0x008B,
    };
    cpu.gdtr.base = GDT as u64;
    cpu.gdtr.limit = 47;
    cpu.idtr.base = IDT as u64;
    cpu.idtr.limit = 0x7FF;
    cpu.rip = CODE as u64;
    cpu.set_gpr_u32(CpuState::RSP, 0x0000_8000);
    cpu.rflags = 0x202; // IF=1 reserved bit1
    (cpu, bus)
}

/// Ring-3 `INT 0x80` through a DPL=3 386 interrupt gate switches to
/// `SS0:ESP0`, pushes the outer stack, then the 32-bit frame, and enters CPL 0.
#[test]
fn privilege_changing_int80_switches_stack_and_pushes_outer_ss_esp() {
    // CD 80   INT 0x80
    let (mut cpu, mut bus) = user_fixture(&[0xCD, 0x80]);
    let old_esp = cpu.gpr_u32(CpuState::RSP);
    let old_ss = cpu.ss.selector;
    let old_flags = cpu.rflags as u32;

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.cs.selector & 3, 0, "CPL becomes 0");
    assert_eq!(cpu.cs.selector & !3, SEL_KCODE);
    assert_eq!(cpu.ss.selector, SEL_KDATA);
    assert_eq!(cpu.rip, u64::from(HANDLER));
    assert!(!cpu.interrupt_flag(), "interrupt gate clears IF");

    // Frame on kernel stack (high→low): SS, ESP, EFLAGS, CS, EIP
    let esp = cpu.gpr_u32(CpuState::RSP);
    assert_eq!(esp, KERNEL_ESP0 - 20);
    assert_eq!(bus.peek_u32((esp + 16) as usize), u32::from(old_ss));
    assert_eq!(bus.peek_u32((esp + 12) as usize), old_esp);
    assert_eq!(bus.peek_u32((esp + 8) as usize), old_flags);
    assert_eq!(bus.peek_u32((esp + 4) as usize), u32::from(SEL_UCODE));
    assert_eq!(bus.peek_u32(esp as usize), CODE as u32 + 2);
}

/// Privilege-changing `#GP` through a 386 interrupt gate pushes a doubleword
/// error code under the outer-stack frame.
#[test]
fn privilege_changing_gp_pushes_dword_error_code() {
    // 66 B8 18 00   MOV AX, 0x0018   (opsize override under CS.D=1)
    // 0F 00 D8      LTR AX           → #GP(0) at CPL 3
    let (mut cpu, mut bus) = user_fixture(&[0x66, 0xB8, 0x18, 0x00, 0x0F, 0x00, 0xD8]);
    bus.write_bytes(IDT + 13 * 8, &encode_idt_gate32(HANDLER, SEL_KCODE, 0x8E));

    step(&mut cpu, &mut bus).unwrap(); // MOV AX
    step(&mut cpu, &mut bus).unwrap(); // LTR → #GP(0) with stack switch

    let esp = cpu.gpr_u32(CpuState::RSP);
    assert_eq!(cpu.cs.selector & 3, 0);
    assert_eq!(esp, KERNEL_ESP0 - 24, "5 dwords + error");
    assert_eq!(bus.peek_u32(esp as usize), 0, "dword error code 0");
    assert_eq!(
        bus.peek_u32((esp + 4) as usize),
        CODE as u32 + 4,
        "faulting EIP points at LTR"
    );
    assert_eq!(bus.peek_u32((esp + 20) as usize), u32::from(SEL_UDATA));
}

/// Same-CPL ring-0 delivery still pushes only FLAGS/CS/EIP (no outer SS:ESP).
#[test]
fn same_cpl_ring0_delivery_unchanged() {
    let (mut cpu, mut bus) = user_fixture(&[0x0F, 0x0B]); // UD2
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
    cpu.set_gpr_u32(CpuState::RSP, 0x0000_8000);

    step(&mut cpu, &mut bus).unwrap();
    let esp = cpu.gpr_u32(CpuState::RSP);
    assert_eq!(esp, 0x8000 - 12, "three dwords only");
    assert_eq!(bus.peek_u32(esp as usize), CODE as u32);
    assert_eq!(cpu.cs.selector, SEL_KCODE);
    assert_eq!(cpu.ss.selector, SEL_KDATA);
}

/// Missing TSS / bad SS0 leaves architectural state untouched.
#[test]
fn privilege_change_without_tss_is_atomic() {
    let (mut cpu, mut bus) = user_fixture(&[0xCD, 0x80]);
    cpu.tr.limit = 0x10; // excludes SS0
    let before = cpu.clone();
    let err = step(&mut cpu, &mut bus).unwrap_err();
    assert!(matches!(
        err,
        ExecError::ProtectedModeExceptionDelivery { vector: 0x80, .. }
    ));
    assert_eq!(cpu.cs, before.cs);
    assert_eq!(cpu.ss, before.ss);
    assert_eq!(cpu.gpr_u32(CpuState::RSP), before.gpr_u32(CpuState::RSP));
    assert_eq!(cpu.rip, before.rip);
}
