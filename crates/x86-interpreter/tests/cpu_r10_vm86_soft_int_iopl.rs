//! Round-10 slice 4: software `INT n` / `INT3` / `INTO` from VM86 + IOPL.
//!
//! Without VME: `INT n` and `INTO` require `IOPL = 3` else `#GP(0)`.
//! `INT3` is not IOPL-sensitive. Successful forms use the VM86→CPL0 9-dword
//! frame from slice 1.
//!
//! Spec: Intel SDM Vol. 3 §20.2.2 / Table 20-1; Vol. 2 INT n/INT3/INTO;
//! Vol. 3 §§20.2–20.3 (delivery frame).

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
const MONITOR_CODE: usize = 0x1000;
const HANDLER_INT: u32 = 0x0000_1800;
const HANDLER_GP: u32 = 0x0000_1900;
const KERNEL_ESP0: u32 = 0x0000_9000;
const SEL_KCODE: u16 = 0x0008;
const SEL_KDATA: u16 = 0x0010;
const SEL_TSS: u16 = 0x0018;

const VM86_CS: u16 = 0x1000;
const VM86_IP: u16 = 0x0100;
const VM86_SS: u16 = 0x2000;
const VM86_SP: u16 = 0xFFFE;

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
    bus.write_bytes(GDT + 8, &encode_seg_desc(0, 0xF_FFFF, 0x9A, 0xC0));
    bus.write_bytes(GDT + 16, &encode_seg_desc(0, 0xF_FFFF, 0x93, 0xC0));
    bus.write_bytes(GDT + 24, &encode_seg_desc(TSS as u32, 0x67, 0x8B, 0));
    bus.poke_u32(TSS + 4, KERNEL_ESP0);
    bus.poke_u16(TSS + 8, SEL_KDATA);

    // INT 0x21 software gate DPL=3.
    bus.write_bytes(
        IDT + 0x21 * 8,
        &encode_idt_gate32(HANDLER_INT, SEL_KCODE, 0xEE),
    );
    // INT3 / INTO / #GP
    bus.write_bytes(IDT + 3 * 8, &encode_idt_gate32(HANDLER_INT, SEL_KCODE, 0xEE));
    bus.write_bytes(IDT + 4 * 8, &encode_idt_gate32(HANDLER_INT, SEL_KCODE, 0xEE));
    bus.write_bytes(IDT + 13 * 8, &encode_idt_gate32(HANDLER_GP, SEL_KCODE, 0x8E));
}

fn enter_vm86(guest: &[u8], iopl: u8) -> (CpuState, RamBus) {
    let mut bus = RamBus::new(0x20000);
    install_tables(&mut bus);
    bus.mem[MONITOR_CODE] = 0xCF;
    bus.mem[HANDLER_INT as usize] = 0xF4;
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
    assert_ne!(cpu.rflags & (1 << 17), 0);
    (cpu, bus)
}

/// VM86 + IOPL=0: `INT n` → `#GP(0)` delivered via the VM86→CPL0 frame.
#[test]
fn vm86_int_n_with_iopl_below_3_raises_gp0() {
    let (mut cpu, mut bus) = enter_vm86(&[0xCD, 0x21], 0);

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.rip, u64::from(HANDLER_GP), "delivered #GP");
    assert_eq!(cpu.rflags & (1 << 17), 0);
    assert_eq!(cpu.cs.selector & 3, 0);
    let esp = cpu.gpr_u32(CpuState::RSP);
    // #GP error code dword + 9-dword VM86 frame = 10 dwords.
    assert_eq!(esp, KERNEL_ESP0 - 40);
    assert_eq!(bus.peek_u32(esp as usize), 0, "error code 0");
    assert_eq!(
        bus.peek_u32((esp + 4) as usize),
        u32::from(VM86_IP),
        "faulting EIP is INT n"
    );
}

/// VM86 + IOPL=3: `INT n` uses the 9-dword VM86→CPL0 frame to the INT gate.
#[test]
fn vm86_int_n_with_iopl3_delivers_nine_dword_frame() {
    let (mut cpu, mut bus) = enter_vm86(&[0xCD, 0x21], 3);
    let saved = cpu.rflags as u32;

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.rip, u64::from(HANDLER_INT));
    assert_eq!(cpu.rflags & (1 << 17), 0);
    let esp = cpu.gpr_u32(CpuState::RSP);
    assert_eq!(esp, KERNEL_ESP0 - 36);
    assert_eq!(bus.peek_u32(esp as usize), u32::from(VM86_IP) + 2);
    assert_eq!(bus.peek_u32((esp + 8) as usize), saved);
    assert_eq!(bus.peek_u32((esp + 32) as usize), 0x6000); // GS
}

/// `INT3` from VM86 ignores IOPL and still delivers (slice 1 path).
#[test]
fn vm86_int3_ignores_iopl_and_delivers() {
    let (mut cpu, mut bus) = enter_vm86(&[0xCC], 0);

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.rip, u64::from(HANDLER_INT));
    assert_eq!(cpu.gpr_u32(CpuState::RSP), KERNEL_ESP0 - 36);
}

/// VM86 + IOPL=0 + OF=1: `INTO` → `#GP(0)`.
#[test]
fn vm86_into_with_iopl_below_3_raises_gp0() {
    let (mut cpu, mut bus) = enter_vm86(&[0xCE], 0);
    cpu.rflags |= 1 << 11; // OF

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.rip, u64::from(HANDLER_GP));
    let esp = cpu.gpr_u32(CpuState::RSP);
    assert_eq!(bus.peek_u32(esp as usize), 0);
    assert_eq!(bus.peek_u32((esp + 4) as usize), u32::from(VM86_IP));
}

/// VM86 + IOPL=3 + OF=1: `INTO` delivers #OF through the VM86 frame.
#[test]
fn vm86_into_with_iopl3_delivers_of() {
    let (mut cpu, mut bus) = enter_vm86(&[0xCE], 3);
    cpu.rflags |= 1 << 11;

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.rip, u64::from(HANDLER_INT));
    assert_eq!(cpu.gpr_u32(CpuState::RSP), KERNEL_ESP0 - 36);
    assert_eq!(
        bus.peek_u32(cpu.gpr_u32(CpuState::RSP) as usize),
        u32::from(VM86_IP) + 1,
        "trap return IP after INTO"
    );
}
