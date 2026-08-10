//! Round-10 slice 1: VM86 → CPL0 interrupt/exception delivery frame.
//!
//! When `EFLAGS.VM=1` and a privilege-changing 386 IDT gate delivers to CPL 0,
//! the processor pushes the **9-dword** VM86 stack frame (GS/FS/DS/ES + SS:ESP
//! + EFLAGS with VM + CS:EIP), then nullifies DS/ES/FS/GS.
//!
//! Spec: Intel SDM Vol. 3 §§20.2–20.3 / Figure 20-2; Vol. 3 §6.12.1 Figure 6-5;
//! Vol. 2 INT n/INT3/INTO; Vol. 1 §3.4.3 (`EFLAGS.VM`).

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
const HANDLER: u32 = 0x0000_1800;
const KERNEL_ESP0: u32 = 0x0000_9000;
const SEL_KCODE: u16 = 0x0008;
const SEL_KDATA: u16 = 0x0010;
const SEL_TSS: u16 = 0x0018;

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

    // Vector 3 (INT3): 386 interrupt gate, DPL=3 so software INT from CPL 3 is legal.
    bus.write_bytes(IDT + 3 * 8, &encode_idt_gate32(HANDLER, SEL_KCODE, 0xEE));
    // Vector 6 (#UD): 386 interrupt gate for exception delivery.
    bus.write_bytes(IDT + 6 * 8, &encode_idt_gate32(HANDLER, SEL_KCODE, 0x8E));
    // Vector 13 (#GP): for IOPL-sensitive paths in later slices.
    bus.write_bytes(IDT + 13 * 8, &encode_idt_gate32(HANDLER, SEL_KCODE, 0x8E));
}

/// Enter VM86 via monitor `IRETD`, then leave guest code at `VM86_CS:VM86_IP`.
fn enter_vm86(guest: &[u8], iopl: u8) -> (CpuState, RamBus) {
    let mut bus = RamBus::new(0x20000);
    install_tables(&mut bus);
    bus.mem[MONITOR_CODE] = 0xCF; // IRETD
    bus.mem[HANDLER as usize] = 0xF4;

    let linear = (u32::from(VM86_CS) << 4) + u32::from(VM86_IP);
    bus.write_bytes(linear as usize, guest);

    let eflags = 0x0002 | (u32::from(iopl) << 12) | (1 << 9) | (1 << 17);
    let frame = (KERNEL_ESP0 - 36) as usize;
    bus.poke_u32(frame, u32::from(VM86_IP));
    bus.poke_u32(frame + 4, u32::from(VM86_CS));
    bus.poke_u32(frame + 8, eflags);
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
    assert_ne!(cpu.rflags & (1 << 17), 0, "fixture must enter VM86");
    assert_eq!(cpu.cs.selector, VM86_CS);
    (cpu, bus)
}

fn assert_null_data_sreg(s: &x86_core::SegmentReg, name: &str) {
    assert_eq!(s.selector & 0xFFFC, 0, "{name} must be null selector");
    assert_eq!(s.base, 0, "{name} base cleared");
    assert_eq!(s.limit, 0, "{name} limit cleared");
    assert_eq!(s.flags, 0, "{name} flags cleared");
}

/// `INT3` from VM86 through a DPL=3 386 interrupt gate builds the 9-dword frame.
#[test]
fn vm86_int3_pushes_nine_dword_frame_and_nullifies_data_sregs() {
    // INT3 is not IOPL-sensitive (Vol. 3 §20.2.2); IOPL=0 still delivers.
    let (mut cpu, mut bus) = enter_vm86(&[0xCC], 0);
    let saved_flags = cpu.rflags as u32;
    assert_ne!(saved_flags & (1 << 17), 0);

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.rflags & (1 << 17), 0, "VM cleared on delivery");
    assert_eq!(cpu.cs.selector & 3, 0, "CPL 0");
    assert_eq!(cpu.cs.selector & !3, SEL_KCODE);
    assert_eq!(cpu.ss.selector, SEL_KDATA);
    assert_eq!(cpu.rip, u64::from(HANDLER));
    assert!(!cpu.interrupt_flag(), "interrupt gate clears IF");

    assert_null_data_sreg(&cpu.es, "ES");
    assert_null_data_sreg(&cpu.ds, "DS");
    assert_null_data_sreg(&cpu.fs, "FS");
    assert_null_data_sreg(&cpu.gs, "GS");

    // Low→high: EIP, CS, EFLAGS, ESP, SS, ES, DS, FS, GS (Vol. 3 Figure 20-2).
    let esp = cpu.gpr_u32(CpuState::RSP);
    assert_eq!(esp, KERNEL_ESP0 - 36, "9 dwords");
    assert_eq!(
        bus.peek_u32(esp as usize),
        u32::from(VM86_IP) + 1,
        "return EIP"
    );
    assert_eq!(bus.peek_u32((esp + 4) as usize), u32::from(VM86_CS));
    assert_eq!(
        bus.peek_u32((esp + 8) as usize),
        saved_flags,
        "EFLAGS with VM"
    );
    assert_eq!(bus.peek_u32((esp + 12) as usize), u32::from(VM86_SP));
    assert_eq!(bus.peek_u32((esp + 16) as usize), u32::from(VM86_SS));
    assert_eq!(bus.peek_u32((esp + 20) as usize), u32::from(VM86_ES));
    assert_eq!(bus.peek_u32((esp + 24) as usize), u32::from(VM86_DS));
    assert_eq!(bus.peek_u32((esp + 28) as usize), u32::from(VM86_FS));
    assert_eq!(bus.peek_u32((esp + 32) as usize), u32::from(VM86_GS));
}

/// Trap gate from VM86 preserves IF while still clearing VM and building the frame.
#[test]
fn vm86_trap_gate_preserves_if_and_pushes_vm86_frame() {
    let (mut cpu, mut bus) = enter_vm86(&[0xCC], 3);
    // Replace vector 3 with a trap gate (type 0xF), DPL=3.
    bus.write_bytes(IDT + 3 * 8, &encode_idt_gate32(HANDLER, SEL_KCODE, 0xEF));
    let saved_flags = cpu.rflags as u32;
    assert_ne!(saved_flags & (1 << 9), 0);

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.rflags & (1 << 17), 0);
    assert!(cpu.interrupt_flag(), "trap gate preserves IF");
    let esp = cpu.gpr_u32(CpuState::RSP);
    assert_eq!(esp, KERNEL_ESP0 - 36);
    assert_eq!(bus.peek_u32((esp + 8) as usize), saved_flags);
}

/// `#UD` from VM86 also uses the privilege-changing VM86 frame (no error code).
#[test]
fn vm86_ud_exception_uses_nine_dword_frame() {
    let (mut cpu, mut bus) = enter_vm86(&[0x0F, 0x0B], 3); // UD2
    let saved_flags = cpu.rflags as u32;

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.cs.selector & 3, 0);
    assert_eq!(cpu.rflags & (1 << 17), 0);
    let esp = cpu.gpr_u32(CpuState::RSP);
    assert_eq!(esp, KERNEL_ESP0 - 36);
    assert_eq!(
        bus.peek_u32(esp as usize),
        u32::from(VM86_IP),
        "faulting EIP is the UD2"
    );
    assert_eq!(bus.peek_u32((esp + 8) as usize), saved_flags);
    assert_eq!(bus.peek_u32((esp + 32) as usize), u32::from(VM86_GS));
    assert_null_data_sreg(&cpu.ds, "DS");
}
