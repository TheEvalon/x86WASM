//! Round-7 slice 3: `VERR` / `VERW` soft segment checks (`0F 00 /4` / `/5`).
//!
//! Spec: Intel SDM Vol. 2 "VERR"/"VERW".

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
const SEL_KCODE: u16 = 0x0008;
const SEL_KDATA: u16 = 0x0010;
const SEL_RODATA: u16 = 0x0018; // type data R=1 W=0
const SEL_XCODE: u16 = 0x0020; // execute-only code
const SEL_RCODE: u16 = 0x0028; // readable code
const SEL_UDATA: u16 = 0x0033; // DPL3 data, RPL3

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

fn fixture(code: &[u8]) -> (CpuState, RamBus) {
    let mut bus = RamBus::new(0x10000);
    bus.write_bytes(GDT, &[0u8; 8]);
    bus.write_bytes(GDT + 8, &encode_seg_desc(0, 0xF_FFFF, 0x9A, 0xC0)); // code
    bus.write_bytes(GDT + 16, &encode_seg_desc(0, 0xF_FFFF, 0x93, 0xC0)); // data RW
    bus.write_bytes(GDT + 24, &encode_seg_desc(0, 0xF_FFFF, 0x91, 0xC0)); // data RO
    bus.write_bytes(GDT + 32, &encode_seg_desc(0, 0xF_FFFF, 0x98, 0xC0)); // exec-only
    bus.write_bytes(GDT + 40, &encode_seg_desc(0, 0xF_FFFF, 0x9A, 0xC0)); // readable code
    bus.write_bytes(GDT + 48, &encode_seg_desc(0, 0xF_FFFF, 0xF3, 0xC0)); // DPL3 data
    bus.write_bytes(IDT + 6 * 8, &encode_idt_gate32(0x1900, SEL_KCODE, 0x8E));
    bus.mem[0x1900] = 0xF4;
    bus.write_bytes(CODE, code);

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
    cpu.gdtr.limit = 55;
    cpu.idtr.base = IDT as u64;
    cpu.idtr.limit = 0x7FF;
    cpu.rip = CODE as u64;
    cpu.rflags = 0x2; // ZF clear
    (cpu, bus)
}

#[test]
fn verr_sets_zf_for_readable_data_and_code() {
    // 0F 00 E0  VERR AX
    let (mut cpu, mut bus) = fixture(&[0x0F, 0x00, 0xE0]);
    cpu.set_gpr_u16(CpuState::RAX, SEL_KDATA);
    step(&mut cpu, &mut bus).unwrap();
    assert_ne!(cpu.rflags & (1 << 6), 0, "VERR data ZF=1");

    let (mut cpu, mut bus) = fixture(&[0x0F, 0x00, 0xE0]);
    cpu.set_gpr_u16(CpuState::RAX, SEL_RCODE);
    step(&mut cpu, &mut bus).unwrap();
    assert_ne!(cpu.rflags & (1 << 6), 0, "VERR readable code ZF=1");

    let (mut cpu, mut bus) = fixture(&[0x0F, 0x00, 0xE0]);
    cpu.set_gpr_u16(CpuState::RAX, SEL_XCODE);
    step(&mut cpu, &mut bus).unwrap();
    assert_eq!(cpu.rflags & (1 << 6), 0, "VERR exec-only ZF=0");
}

#[test]
fn verw_sets_zf_only_for_writable_data() {
    let (mut cpu, mut bus) = fixture(&[0x0F, 0x00, 0xE8]); // VERW AX
    cpu.set_gpr_u16(CpuState::RAX, SEL_KDATA);
    step(&mut cpu, &mut bus).unwrap();
    assert_ne!(cpu.rflags & (1 << 6), 0, "VERW RW data");

    let (mut cpu, mut bus) = fixture(&[0x0F, 0x00, 0xE8]);
    cpu.set_gpr_u16(CpuState::RAX, SEL_RODATA);
    step(&mut cpu, &mut bus).unwrap();
    assert_eq!(cpu.rflags & (1 << 6), 0, "VERW RO data");

    let (mut cpu, mut bus) = fixture(&[0x0F, 0x00, 0xE8]);
    cpu.set_gpr_u16(CpuState::RAX, SEL_RCODE);
    step(&mut cpu, &mut bus).unwrap();
    assert_eq!(cpu.rflags & (1 << 6), 0, "VERW code");
}

#[test]
fn verr_verw_null_and_privilege_clear_zf() {
    let (mut cpu, mut bus) = fixture(&[0x0F, 0x00, 0xE0]);
    cpu.set_gpr_u16(CpuState::RAX, 0);
    step(&mut cpu, &mut bus).unwrap();
    assert_eq!(cpu.rflags & (1 << 6), 0, "null");

    // CPL0 checking a DPL3 selector with RPL0: max(CPL,RPL)=0 ≤ 3 → OK for VERR.
    let (mut cpu, mut bus) = fixture(&[0x0F, 0x00, 0xE0]);
    cpu.set_gpr_u16(CpuState::RAX, SEL_UDATA & !3); // RPL0, DPL3 data
    step(&mut cpu, &mut bus).unwrap();
    assert_ne!(cpu.rflags & (1 << 6), 0);

    // Ring-3 checking DPL0 data with RPL3: CPL=3 > DPL=0 → ZF=0.
    let (mut cpu, mut bus) = fixture(&[0x0F, 0x00, 0xE0]);
    cpu.cs.selector = 0x000B; // RPL3, still uses DPL0 code cache for CPL
    cpu.cs.flags = 0xC0FA;
    cpu.set_gpr_u16(CpuState::RAX, SEL_KDATA | 3);
    step(&mut cpu, &mut bus).unwrap();
    assert_eq!(cpu.rflags & (1 << 6), 0, "privilege fail");
}

#[test]
fn verr_in_real_mode_is_ud() {
    let mut bus = RamBus::new(0x10000);
    // IVT vector 6 → 0000:1800
    bus.write_bytes(6 * 4, &[0x00, 0x18, 0x00, 0x00]);
    bus.write_bytes(0x7C00, &[0x0F, 0x00, 0xE0, 0xF4]);
    bus.mem[0x1800] = 0xF4;
    let mut cpu = CpuState::reset();
    cpu.cs = x86_core::SegmentReg::real_mode_code(0);
    cpu.ss = x86_core::SegmentReg::real_mode(0);
    cpu.rip = 0x7C00;
    cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
    step(&mut cpu, &mut bus).unwrap();
    assert_eq!(cpu.ip16(), 0x1800);
    assert_eq!(cpu.cs.selector, 0);
}
