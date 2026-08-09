//! Round-5 slice 1: 32-bit available TSS descriptor load via `LTR`, and `STR`.
//!
//! Guest-built GDT + TSS + IDT fixtures exercise present/type/privilege checks
//! and the busy-bit update. No task switch is performed.
//!
//! Spec: Intel SDM Vol. 2 "LTR"/"STR"; Vol. 3 §§7.2–7.3 (Table 3-2).

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
const HANDLER: u16 = 0x1800;
const SEL_CODE: u16 = 0x0008;
const SEL_DATA: u16 = 0x0010;
const SEL_TSS: u16 = 0x0018;

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
    let offset = offset.to_le_bytes();
    let selector = selector.to_le_bytes();
    [
        offset[0],
        offset[1],
        selector[0],
        selector[1],
        0,
        access,
        0,
        0,
    ]
}

fn install_gdt(bus: &mut RamBus, tss_access: u8, tss_limit: u32) {
    bus.write_bytes(GDT, &[0u8; 8]);
    bus.write_bytes(GDT + 8, &encode_seg_desc(0, 0xFFFF, 0x9A, 0));
    bus.write_bytes(GDT + 16, &encode_seg_desc(0, 0xFFFF, 0x93, 0));
    bus.write_bytes(
        GDT + 24,
        &encode_seg_desc(TSS as u32, tss_limit, tss_access, 0),
    );
}

fn install_exception_gates(bus: &mut RamBus, cpu: &mut CpuState) {
    for vector in [6u8, 11, 13] {
        let entry = IDT + usize::from(vector) * 8;
        bus.write_bytes(entry, &encode_idt_gate16(HANDLER, SEL_CODE, 0x86));
    }
    cpu.idtr.base = IDT as u64;
    cpu.idtr.limit = 13 * 8 + 7;
}

fn protected_fixture(code: &[u8]) -> (CpuState, RamBus) {
    let mut bus = RamBus::new(0x10000);
    install_gdt(&mut bus, 0x89, 0x67);
    bus.write_bytes(CODE, code);
    bus.mem[usize::from(HANDLER)] = 0xF4; // HLT in handler

    let mut cpu = CpuState::reset();
    cpu.cr0 |= 1;
    cpu.cs = x86_core::SegmentReg {
        selector: SEL_CODE,
        base: 0,
        limit: 0xFFFF,
        flags: 0x009A,
    };
    cpu.ss = x86_core::SegmentReg {
        selector: SEL_DATA,
        base: 0,
        limit: 0xFFFF,
        flags: 0x0093,
    };
    cpu.ds = cpu.ss.clone();
    cpu.gdtr.base = GDT as u64;
    cpu.gdtr.limit = 31;
    install_exception_gates(&mut bus, &mut cpu);
    cpu.rip = CODE as u64;
    cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
    (cpu, bus)
}

/// `LTR` loads TR from a present available 32-bit TSS (`type=9`) and marks it
/// busy in the GDT; `STR` then stores the visible selector.
/// Spec: SDM Vol. 2 LTR/STR; Vol. 3 §§7.2–7.3.
#[test]
fn ltr_loads_available_tss32_marks_busy_and_str_stores_selector() {
    // B8 18 00      MOV AX, 0x0018
    // 0F 00 D8      LTR AX
    // 0F 00 CB      STR BX
    // F4            HLT
    let (mut cpu, mut bus) =
        protected_fixture(&[0xB8, 0x18, 0x00, 0x0F, 0x00, 0xD8, 0x0F, 0x00, 0xCB, 0xF4]);

    step(&mut cpu, &mut bus).unwrap();
    step(&mut cpu, &mut bus).unwrap();
    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.tr.selector, SEL_TSS);
    assert_eq!(cpu.tr.base, TSS as u64);
    assert_eq!(cpu.tr.limit, 0x67);
    assert_eq!(cpu.tr.flags & 0x0F, 0x0B, "cached type becomes busy");
    assert_eq!(bus.mem[GDT + 24 + 5] & 0x0F, 0x0B, "GDT descriptor busy");
    assert_eq!(cpu.gpr_u16(CpuState::RBX), SEL_TSS);
}

/// CPL > 0 raises `#GP(0)` through the IDT and leaves TR/GDT untouched.
#[test]
fn ltr_at_cpl3_raises_gp0_atomically() {
    let (mut cpu, mut bus) = protected_fixture(&[0xB8, 0x18, 0x00, 0x0F, 0x00, 0xD8, 0xF4]);
    // Same-CPL ring-3 code+data so delivery stays same-privilege.
    bus.write_bytes(GDT + 8, &encode_seg_desc(0, 0xFFFF, 0xFA, 0));
    bus.write_bytes(GDT + 16, &encode_seg_desc(0, 0xFFFF, 0xF3, 0));
    // Retarget exception gates at the ring-3 code selector.
    for vector in [6u8, 11, 13] {
        let entry = IDT + usize::from(vector) * 8;
        bus.write_bytes(entry, &encode_idt_gate16(HANDLER, SEL_CODE | 3, 0xE6));
    }
    cpu.cs.selector = SEL_CODE | 3;
    cpu.cs.flags = 0x00FA;
    cpu.ss.selector = SEL_DATA | 3;
    cpu.ss.flags = 0x00F3;

    step(&mut cpu, &mut bus).unwrap(); // MOV AX
    let before = cpu.tr.clone();
    let access_before = bus.mem[GDT + 24 + 5];
    step(&mut cpu, &mut bus).unwrap(); // LTR → #GP(0)

    assert_eq!(cpu.rip, u64::from(HANDLER));
    assert_eq!(cpu.tr, before);
    assert_eq!(bus.mem[GDT + 24 + 5], access_before);
    // 16-bit gate frame: [SP]=error, [SP+2]=IP, [SP+4]=CS, [SP+6]=FLAGS
    let sp = usize::from(cpu.gpr_u16(CpuState::RSP));
    assert_eq!(bus.peek_u16(sp), 0, "#GP(0) error code");
}

/// Null, LDT, busy, not-present, wrong-type, and short-limit selectors fault
/// with the selector error code; TR is unchanged.
#[test]
fn ltr_rejects_invalid_tss_descriptors() {
    let cases: &[(u16, u8, u32, u16)] = &[
        (0x0000, 0x89, 0x67, 0),               // null → #GP(0)
        (0x001C, 0x89, 0x67, 0x001C & 0xFFFC), // TI=1 → #GP
        (SEL_TSS, 0x8B, 0x67, SEL_TSS),        // busy → #GP
        (SEL_TSS, 0x09, 0x67, SEL_TSS),        // not present → #NP
        (SEL_TSS, 0x92, 0x67, SEL_TSS),        // data → #GP
        (SEL_TSS, 0x89, 0x66, SEL_TSS),        // limit < 67H → #GP
    ];

    for &(selector, access, limit, error_code) in cases {
        let lo = selector.to_le_bytes();
        let (mut cpu, mut bus) = protected_fixture(&[0xB8, lo[0], lo[1], 0x0F, 0x00, 0xD8, 0xF4]);
        install_gdt(&mut bus, access, limit);
        step(&mut cpu, &mut bus).unwrap();
        let before = cpu.tr.clone();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(
            cpu.rip,
            u64::from(HANDLER),
            "selector={selector:#06x} access={access:#04x}"
        );
        assert_eq!(cpu.tr, before);
        let sp = usize::from(cpu.gpr_u16(CpuState::RSP));
        assert_eq!(
            bus.peek_u16(sp),
            error_code,
            "selector={selector:#06x} access={access:#04x}"
        );
        if selector == SEL_TSS && access == 0x89 && limit < 0x67 {
            assert_eq!(bus.mem[GDT + 24 + 5], 0x89, "short limit must not busy");
        }
    }
}

/// Real-address mode `LTR` is `#UD` via the IVT.
#[test]
fn ltr_in_real_mode_is_ud() {
    let mut bus = RamBus::new(0x10000);
    // IVT vector 6 → 0000:1800
    bus.write_bytes(6 * 4, &[0x00, 0x18, 0x00, 0x00]);
    bus.write_bytes(0x7C00, &[0x0F, 0x00, 0xD8, 0xF4]);
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

/// `STR` stores the current TR selector without requiring CPL 0.
#[test]
fn str_stores_selector_at_cpl3() {
    let (mut cpu, mut bus) = protected_fixture(&[0x0F, 0x00, 0xC8, 0xF4]); // STR AX
    cpu.tr = x86_core::SegmentReg {
        selector: SEL_TSS,
        base: TSS as u64,
        limit: 0x67,
        flags: 0x008B,
    };
    cpu.cs.selector = SEL_CODE | 3;
    cpu.cs.flags = 0x00FA;
    cpu.ss.selector = SEL_DATA | 3;
    cpu.ss.flags = 0x00F3;

    step(&mut cpu, &mut bus).unwrap();
    assert_eq!(cpu.gpr_u16(CpuState::RAX), SEL_TSS);
}
