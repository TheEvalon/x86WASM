//! Round-13 slice 3: deepen VME soft-int redirect bitmap (more vectors /
//! edges) and method-6 FLAGS rewrite.
//!
//! Extends the R12 stub: `INT n` with redirect bit clear and `IOPL < 3`
//! pushes FLAGS with `IOPL=3` and `IF ← VIF`, then clears `VIF`/`IF`/`TF`.
//! Edge coverage: vectors `0x00` / `0xFF`, byte-boundary bit, incomplete map,
//! and method-5 (`IOPL=3`) live FLAGS push.
//!
//! Spec: Intel SDM Vol. 3 §§20.2–20.3 Table 20-2 / Figure 20-5; Vol. 2 INT n;
//! Vol. 3 §7.2.1.

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
const TSS_LIMIT: u32 = 0x87;
const IO_MAP_BASE: u16 = 0x88;
const REDIRECT_MAP: usize = TSS + (IO_MAP_BASE as usize) - 32;
const MONITOR_CODE: usize = 0x1000;
const HANDLER_IDT: u32 = 0x0000_1800;
const HANDLER_GP: u32 = 0x0000_1900;
const KERNEL_ESP0: u32 = 0x0000_9000;
const SEL_KCODE: u16 = 0x0008;
const SEL_KDATA: u16 = 0x0010;
const SEL_TSS: u16 = 0x0018;

const VM86_CS: u16 = 0x1000;
const VM86_IP: u16 = 0x0100;
const VM86_SS: u16 = 0x1000;
const VM86_SP: u16 = 0xFFFE;
const IVT_HANDLER_IP: u16 = 0x0400;

const CR4_VME: u64 = 1;
const EFLAGS_VIF: u64 = 1 << 19;

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

fn set_redirect_bit(bus: &mut RamBus, vector: u8, set: bool) {
    let byte_index = REDIRECT_MAP + usize::from(vector) / 8;
    let bit = 1u8 << (vector % 8);
    if set {
        bus.mem[byte_index] |= bit;
    } else {
        bus.mem[byte_index] &= !bit;
    }
}

fn install_tables(bus: &mut RamBus) {
    bus.write_bytes(GDT, &[0u8; 8]);
    bus.write_bytes(GDT + 8, &encode_seg_desc(0, 0xF_FFFF, 0x9A, 0xC0));
    bus.write_bytes(GDT + 16, &encode_seg_desc(0, 0xF_FFFF, 0x93, 0xC0));
    bus.write_bytes(GDT + 24, &encode_seg_desc(TSS as u32, TSS_LIMIT, 0x8B, 0));
    bus.poke_u32(TSS + 4, KERNEL_ESP0);
    bus.poke_u16(TSS + 8, SEL_KDATA);
    bus.poke_u16(TSS + 0x66, IO_MAP_BASE);
    for b in 0..32 {
        bus.mem[REDIRECT_MAP + b] = 0xFF;
    }
    bus.write_bytes(
        IDT + 0x00 * 8,
        &encode_idt_gate32(HANDLER_IDT, SEL_KCODE, 0xEE),
    );
    bus.write_bytes(
        IDT + 0x07 * 8,
        &encode_idt_gate32(HANDLER_IDT, SEL_KCODE, 0xEE),
    );
    bus.write_bytes(
        IDT + 0xFF * 8,
        &encode_idt_gate32(HANDLER_IDT, SEL_KCODE, 0xEE),
    );
    bus.write_bytes(
        IDT + 13 * 8,
        &encode_idt_gate32(HANDLER_GP, SEL_KCODE, 0x8E),
    );
}

fn enter_vm86(guest: &[u8], iopl: u8) -> (CpuState, RamBus) {
    let mut bus = RamBus::new(0x40000);
    install_tables(&mut bus);
    bus.mem[MONITOR_CODE] = 0xCF;
    bus.mem[HANDLER_IDT as usize] = 0xF4;
    bus.mem[HANDLER_GP as usize] = 0xF4;

    let linear = (u32::from(VM86_CS) << 4) + u32::from(VM86_IP);
    bus.write_bytes(linear as usize, guest);
    let ivt_lin = (u32::from(VM86_CS) << 4) + u32::from(IVT_HANDLER_IP);
    bus.mem[ivt_lin as usize] = 0xF4;
    for vector in [0x00u8, 0x07, 0xFF] {
        bus.poke_u16(usize::from(vector) * 4, IVT_HANDLER_IP);
        bus.poke_u16(usize::from(vector) * 4 + 2, VM86_CS);
    }

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
    cpu.cr4 |= CR4_VME;
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
        limit: TSS_LIMIT,
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

fn stack_base() -> usize {
    (u32::from(VM86_SS) << 4) as usize
}

/// Method 6: vector 0, IOPL=0, VIF=1 → IVT + FLAGS rewrite.
#[test]
fn vme_redirect_vector0_method6_flags() {
    let (mut cpu, mut bus) = enter_vm86(&[0xCD, 0x00], 0);
    set_redirect_bit(&mut bus, 0x00, false);
    cpu.rflags |= EFLAGS_VIF;

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.rip, u64::from(IVT_HANDLER_IP));
    let sp = cpu.gpr_u16(CpuState::RSP);
    let pushed = bus.peek_u16(stack_base() + sp as usize + 4);
    assert_eq!((pushed >> 12) & 3, 3);
    assert_ne!(pushed & (1 << 9), 0);
    assert_eq!(cpu.rflags & EFLAGS_VIF, 0);
}

/// Vector 0xFF (last bitmap bit) redirects when clear.
#[test]
fn vme_redirect_vector_ff_bit_clear() {
    let (mut cpu, mut bus) = enter_vm86(&[0xCD, 0xFF], 0);
    set_redirect_bit(&mut bus, 0xFF, false);

    step(&mut cpu, &mut bus).unwrap();

    assert_ne!(cpu.rflags & (1 << 17), 0);
    assert_eq!(cpu.rip, u64::from(IVT_HANDLER_IP));
}

/// Vector 7 (bit 7 of byte 0) bit-set still takes `#GP` at IOPL=0.
#[test]
fn vme_redirect_vector7_bit_set_iopl0_gp() {
    let (mut cpu, mut bus) = enter_vm86(&[0xCD, 0x07], 0);
    set_redirect_bit(&mut bus, 0x07, true);

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.rip, u64::from(HANDLER_GP));
}

/// Method 5 (`IOPL=3`): push live FLAGS (no IF←VIF rewrite); VIF unchanged.
#[test]
fn vme_redirect_iopl3_method5_live_flags() {
    let (mut cpu, mut bus) = enter_vm86(&[0xCD, 0x00], 3);
    set_redirect_bit(&mut bus, 0x00, false);
    cpu.rflags |= EFLAGS_VIF;
    cpu.set_interrupt_flag(false);
    let flags_before = cpu.rflags as u16;

    step(&mut cpu, &mut bus).unwrap();

    let sp = cpu.gpr_u16(CpuState::RSP);
    let pushed = bus.peek_u16(stack_base() + sp as usize + 4);
    assert_eq!(pushed, flags_before, "method 5 pushes live FLAGS");
    assert_ne!(cpu.rflags & EFLAGS_VIF, 0, "VIF not cleared on method 5");
    assert_eq!(cpu.rflags & (1 << 9), 0, "IF cleared");
}

/// Incomplete redirect map (`I/O base` past `TR.limit`) → treat bits as set.
#[test]
fn vme_incomplete_redirect_map_treated_as_set() {
    let (mut cpu, mut bus) = enter_vm86(&[0xCD, 0x00], 0);
    // I/O map base 0x88 needs map through 0x87; shrink TR.limit below that.
    cpu.tr.limit = 0x70;
    set_redirect_bit(&mut bus, 0x00, false); // would redirect if map were valid

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.rip, u64::from(HANDLER_GP));
}

/// `I/O map base < 32` → no map → bits treated as set → `#GP` at IOPL=0.
#[test]
fn vme_io_map_base_below_32_no_silent_redirect() {
    let (mut cpu, mut bus) = enter_vm86(&[0xCD, 0x00], 0);
    bus.poke_u16(TSS + 0x66, 16); // too small for a 32-byte redirect map
    set_redirect_bit(&mut bus, 0x00, false);

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.rip, u64::from(HANDLER_GP));
}
