//! Round-12 slice 4: `INTO` / `#OF` from VM86 vs `INT n` under VME/IOPL.
//!
//! `INTO` (`0xCE`) is **not** IOPL-sensitive and is **not** governed by the
//! software-interrupt redirection bitmap (those apply only to `INT n`). With
//! `OF=1` it delivers vector 4 through the protected-mode IDT (9-dword VM86
//! frame). With `OF=0` it falls through. `CR4.VME=1` must not redirect `#OF`
//! to the IVT even when the bitmap bit for vector 4 is clear.
//!
//! Spec: Intel SDM Vol. 2 "INT n/INTO/INT3/INT1" (Virtual-8086 Mode Exceptions);
//! Vol. 3 §20.2.2 Table 20-2; Vol. 3 §6.15 (#OF trap).

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
const HANDLER_OF: u32 = 0x0000_1800;
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

fn enter_vm86(guest: &[u8], iopl: u8, cr4_vme: bool) -> (CpuState, RamBus) {
    let mut bus = RamBus::new(0x40000);
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

    // Vector 4: #OF / INT 4 gate (DPL=3 for software INTO and INT 4).
    bus.write_bytes(IDT + 4 * 8, &encode_idt_gate32(HANDLER_OF, SEL_KCODE, 0xEE));
    bus.write_bytes(
        IDT + 13 * 8,
        &encode_idt_gate32(HANDLER_GP, SEL_KCODE, 0x8E),
    );

    bus.mem[MONITOR_CODE] = 0xCF;
    bus.mem[HANDLER_OF as usize] = 0xF4;
    bus.mem[HANDLER_GP as usize] = 0xF4;

    let linear = (u32::from(VM86_CS) << 4) + u32::from(VM86_IP);
    bus.write_bytes(linear as usize, guest);
    let ivt_lin = (u32::from(VM86_CS) << 4) + u32::from(IVT_HANDLER_IP);
    bus.mem[ivt_lin as usize] = 0xF4;
    // IVT vector 4 → VM86 handler (used only if INT 4 redirects).
    bus.poke_u16(4 * 4, IVT_HANDLER_IP);
    bus.poke_u16(4 * 4 + 2, VM86_CS);

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

/// VME=1, IOPL=0, redirect bit 4 clear: `INTO` still uses the IDT (not IVT).
#[test]
fn vme_into_of_ignores_redirect_bitmap_and_iopl() {
    let (mut cpu, mut bus) = enter_vm86(&[0xCE], 0, true);
    set_redirect_bit(&mut bus, 4, false);
    cpu.rflags |= 1 << 11; // OF=1
    let saved = cpu.rflags as u32;

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.rip, u64::from(HANDLER_OF), "IDT #OF handler");
    assert_eq!(cpu.rflags & (1 << 17), 0, "left VM86");
    let esp = cpu.gpr_u32(CpuState::RSP);
    assert_eq!(esp, KERNEL_ESP0 - 36);
    assert_eq!(bus.peek_u32(esp as usize), u32::from(VM86_IP) + 1);
    assert_eq!(bus.peek_u32((esp + 8) as usize), saved);
}

/// Contrast: `INT 4` with the same VME/IOPL/bitmap **does** IVT-redirect.
#[test]
fn vme_int4_with_clear_bitmap_redirects_unlike_into() {
    let (mut cpu, mut bus) = enter_vm86(&[0xCD, 0x04], 0, true);
    set_redirect_bit(&mut bus, 4, false);

    step(&mut cpu, &mut bus).unwrap();

    assert_ne!(cpu.rflags & (1 << 17), 0, "stays in VM86");
    assert_eq!(cpu.rip, u64::from(IVT_HANDLER_IP));
}

/// Untaken `INTO` with VME+IOPL0 does not consult IDT or IVT.
#[test]
fn vme_into_no_of_falls_through() {
    let (mut cpu, mut bus) = enter_vm86(&[0xCE, 0xF4], 0, true);
    set_redirect_bit(&mut bus, 4, false);
    cpu.rflags &= !(1 << 11);

    step(&mut cpu, &mut bus).unwrap();

    assert_ne!(cpu.rflags & (1 << 17), 0);
    assert_eq!(cpu.rip, u64::from(VM86_IP) + 1);
}
