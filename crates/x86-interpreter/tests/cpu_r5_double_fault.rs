//! Round-5 slice 4: `#DF` escalation and triple-fault host error.
//!
//! Spec: Intel SDM Vol. 3 §6.15 (Interrupt 8—Double Fault Exception).

use x86_core::CpuState;
use x86_interpreter::{step, Bus, ExecError, ProtectedModeDeliveryError};

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
const CODE: usize = 0x1000;
const DF_HANDLER: u32 = 0x0000_1A00;
const SEL_KCODE: u16 = 0x0008;
const SEL_KDATA: u16 = 0x0010;

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

fn ring0_fixture(code: &[u8]) -> (CpuState, RamBus) {
    let mut bus = RamBus::new(0x10000);
    bus.write_bytes(GDT, &[0u8; 8]);
    bus.write_bytes(GDT + 8, &encode_seg_desc(0, 0xF_FFFF, 0x9A, 0xC0));
    bus.write_bytes(GDT + 16, &encode_seg_desc(0, 0xF_FFFF, 0x93, 0xC0));
    bus.write_bytes(CODE, code);
    bus.mem[DF_HANDLER as usize] = 0xF4;

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
    cpu.gdtr.base = GDT as u64;
    cpu.gdtr.limit = 23;
    cpu.idtr.base = IDT as u64;
    cpu.idtr.limit = 0x7FF;
    cpu.rip = CODE as u64;
    cpu.set_gpr_u32(CpuState::RSP, 0x0000_8000);
    (cpu, bus)
}

/// An invalid `#UD` gate escalates into `#DF` with error code 0.
#[test]
fn fault_during_exception_delivery_escalates_to_double_fault() {
    let (mut cpu, mut bus) = ring0_fixture(&[0x0F, 0x0B]); // UD2
                                                           // Vector 6: task gate — first delivery fails without touching the stack.
    bus.write_bytes(IDT + 6 * 8, &encode_idt_gate32(0x1900, SEL_KCODE, 0x85));
    // Vector 8: valid 386 interrupt gate for #DF.
    bus.write_bytes(IDT + 8 * 8, &encode_idt_gate32(DF_HANDLER, SEL_KCODE, 0x8E));

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.rip, u64::from(DF_HANDLER), "#DF handler entered");
    let esp = cpu.gpr_u32(CpuState::RSP);
    // #DF pushes dword error code 0 under FLAGS/CS/EIP.
    assert_eq!(bus.peek_u32(esp as usize), 0, "#DF error code is 0");
}

/// When `#DF` delivery also fails, the host sees `TripleFault` and state is
/// unchanged from the instruction boundary.
#[test]
fn fault_during_double_fault_delivery_is_triple_fault() {
    let (mut cpu, mut bus) = ring0_fixture(&[0x0F, 0x0B]); // UD2
    bus.write_bytes(IDT + 6 * 8, &encode_idt_gate32(0x1900, SEL_KCODE, 0x85));
    // No usable #DF gate (task-gate type 0x85).
    bus.write_bytes(IDT + 8 * 8, &encode_idt_gate32(DF_HANDLER, SEL_KCODE, 0x85));
    let before = cpu.clone();
    let mem_before = bus.mem.clone();

    let err = step(&mut cpu, &mut bus).unwrap_err();
    assert!(matches!(
        err,
        ExecError::TripleFault {
            reason: ProtectedModeDeliveryError::GateType(0x85)
        }
    ));
    assert_eq!(cpu, before);
    assert_eq!(bus.mem, mem_before);
}
