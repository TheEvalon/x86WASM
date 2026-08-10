//! Round-12 slice 3: 16-bit IDT interrupt/trap gate delivery from VM86.
//!
//! Privilege-changing 286 gates (`type 6/7`) from virtual-8086 mode push the
//! **9-word** VM86 frame (GS/FS/DS/ES + SS:SP + FLAGS + CS:IP) on the inner
//! stack, then nullify DS/ES/FS/GS. Frame element width follows the gate;
//! stack-pointer width follows destination `SS.B`.
//!
//! Spec: Intel SDM Vol. 3 §§20.2–20.3 / Figure 20-2 (word form); §6.11–§6.12.1;
//! Vol. 2 INT n.

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

    fn peek_u16(&self, addr: usize) -> u16 {
        u16::from_le_bytes([self.mem[addr], self.mem[addr + 1]])
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
const IDT: usize = 0x2800;
const TSS: usize = 0x3000;
const MONITOR_CODE: usize = 0x1000;
const HANDLER16: u32 = 0x0000_1800;
const KERNEL_ESP0: u32 = 0x0000_9000;
const SEL_KCODE32: u16 = 0x0008;
const SEL_KDATA: u16 = 0x0010;
const SEL_TSS: u16 = 0x0018;
const SEL_KCODE16: u16 = 0x0020;

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

fn encode_idt_gate16(offset: u16, selector: u16, access: u8) -> [u8; 8] {
    let off = offset.to_le_bytes();
    let sel = selector.to_le_bytes();
    [off[0], off[1], sel[0], sel[1], 0, access, 0, 0]
}

fn enter_vm86(guest: &[u8]) -> (CpuState, RamBus) {
    let mut bus = RamBus::new(0x20000);
    bus.write_bytes(GDT, &[0u8; 8]);
    // null, 32-bit monitor code, data, TSS, 16-bit handler code (D=0, G=1).
    bus.write_bytes(GDT + 8, &encode_seg_desc(0, 0xF_FFFF, 0x9A, 0xC0));
    bus.write_bytes(GDT + 16, &encode_seg_desc(0, 0xF_FFFF, 0x93, 0xC0));
    bus.write_bytes(GDT + 24, &encode_seg_desc(TSS as u32, 0x67, 0x8B, 0));
    bus.write_bytes(GDT + 32, &encode_seg_desc(0, 0xF_FFFF, 0x9A, 0x80));
    bus.poke_u32(TSS + 4, KERNEL_ESP0);
    bus.poke_u16(TSS + 8, SEL_KDATA);
    // INT 0x21 — 286 interrupt gate P=1 DPL=3 type=6.
    bus.write_bytes(
        IDT + 0x21 * 8,
        &encode_idt_gate16(HANDLER16 as u16, SEL_KCODE16, 0xE6),
    );

    bus.mem[MONITOR_CODE] = 0xCF;
    bus.mem[HANDLER16 as usize] = 0xF4;
    let linear = (u32::from(VM86_CS) << 4) + u32::from(VM86_IP);
    bus.write_bytes(linear as usize, guest);

    let eflags = 0x0002 | (3u32 << 12) | (1 << 9) | (1 << 17);
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
        selector: SEL_KCODE32,
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
    cpu.gdtr.limit = 39;
    cpu.idtr.base = IDT as u64;
    cpu.idtr.limit = 0x7FF;
    cpu.rip = MONITOR_CODE as u64;
    cpu.set_gpr_u32(CpuState::RSP, KERNEL_ESP0 - 36);
    cpu.rflags = 0x2;
    step(&mut cpu, &mut bus).unwrap();
    assert_ne!(cpu.rflags & (1 << 17), 0);
    (cpu, bus)
}

fn assert_null_data(seg: &x86_core::SegmentReg, name: &str) {
    assert_eq!(seg.base, 0, "{name} base");
    assert_eq!(seg.limit, 0, "{name} limit");
    assert_eq!(seg.flags, 0, "{name} flags");
}

/// VM86 `INT 21h` through a 286 gate: 9-word frame + null data segments.
#[test]
fn vm86_int_n_16bit_gate_nine_word_frame() {
    let (mut cpu, mut bus) = enter_vm86(&[0xCD, 0x21]);
    let saved_flags = cpu.rflags as u16;

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.rip, u64::from(HANDLER16));
    assert_eq!(cpu.cs.selector & !3, SEL_KCODE16);
    assert_eq!(cpu.rflags & (1 << 17), 0, "VM cleared");
    assert_eq!(cpu.rflags & (1 << 9), 0, "IF cleared by interrupt gate");

    // 9 words × 2 = 18 bytes from KERNEL_ESP0 on a B=1 stack.
    let esp = cpu.gpr_u32(CpuState::RSP);
    assert_eq!(esp, KERNEL_ESP0 - 18);
    let base = esp as usize;
    assert_eq!(bus.peek_u16(base), VM86_IP + 2);
    assert_eq!(bus.peek_u16(base + 2), VM86_CS);
    assert_eq!(bus.peek_u16(base + 4), saved_flags);
    assert_eq!(bus.peek_u16(base + 6), VM86_SP);
    assert_eq!(bus.peek_u16(base + 8), VM86_SS);
    assert_eq!(bus.peek_u16(base + 10), VM86_ES);
    assert_eq!(bus.peek_u16(base + 12), VM86_DS);
    assert_eq!(bus.peek_u16(base + 14), VM86_FS);
    assert_eq!(bus.peek_u16(base + 16), VM86_GS);

    assert_null_data(&cpu.es, "ES");
    assert_null_data(&cpu.ds, "DS");
    assert_null_data(&cpu.fs, "FS");
    assert_null_data(&cpu.gs, "GS");
}
