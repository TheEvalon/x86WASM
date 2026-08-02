//! Reference interpreter for the lab opcode subset (M1 + early M2).
//!
//! Semantics follow Intel SDM Vol. 2 / Vol. 3 for the implemented forms only.

#![forbid(unsafe_code)]

use thiserror::Error;
use x86_core::CpuState;
use x86_decode::{decode, DecodeError, DecodedInsn};
use x86_mmu::linear_addr;

/// Memory + port callbacks supplied by `machine-pc`.
pub trait Bus {
    fn read_u8(&mut self, addr: u64) -> Result<u8, ExecError>;
    fn write_u8(&mut self, addr: u64, val: u8) -> Result<(), ExecError>;
    fn read_u16(&mut self, addr: u64) -> Result<u16, ExecError> {
        let lo = self.read_u8(addr)?;
        let hi = self.read_u8(addr.wrapping_add(1))?;
        Ok(u16::from_le_bytes([lo, hi]))
    }
    fn write_u16(&mut self, addr: u64, val: u16) -> Result<(), ExecError> {
        let bytes = val.to_le_bytes();
        self.write_u8(addr, bytes[0])?;
        self.write_u8(addr.wrapping_add(1), bytes[1])
    }
    fn port_in_u8(&mut self, port: u16) -> Result<u8, ExecError>;
    fn port_out_u8(&mut self, port: u16, val: u8) -> Result<(), ExecError>;
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ExecError {
    #[error(transparent)]
    Decode(#[from] DecodeError),
    #[error("memory fault at {0:#x}")]
    MemoryFault(u64),
    #[error("unsupported encoding for opcode 0x{0:02X}")]
    Unsupported(u8),
}

fn parity_even(v: u8) -> bool {
    v.count_ones().is_multiple_of(2)
}

fn set_logic_flags_u8(cpu: &mut CpuState, result: u8) {
    cpu.set_cf(false);
    cpu.set_of(false);
    cpu.set_af(false);
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x80 != 0);
    cpu.set_pf(parity_even(result));
}

fn set_logic_flags_u16(cpu: &mut CpuState, result: u16) {
    cpu.set_cf(false);
    cpu.set_of(false);
    cpu.set_af(false);
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x8000 != 0);
    cpu.set_pf(parity_even(result as u8));
}

fn set_add_flags_u16(cpu: &mut CpuState, a: u16, b: u16, result: u16) {
    cpu.set_cf((a as u32) + (b as u32) > 0xFFFF);
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x8000 != 0);
    cpu.set_pf(parity_even(result as u8));
    cpu.set_af(((a ^ b ^ result) & 0x10) != 0);
    let of = (!(a ^ b) & (a ^ result) & 0x8000) != 0;
    cpu.set_of(of);
}

fn set_sub_flags_u16(cpu: &mut CpuState, a: u16, b: u16, result: u16) {
    cpu.set_cf(a < b);
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x8000 != 0);
    cpu.set_pf(parity_even(result as u8));
    cpu.set_af(((a ^ b ^ result) & 0x10) != 0);
    let of = ((a ^ b) & (a ^ result) & 0x8000) != 0;
    cpu.set_of(of);
}

fn set_sub_flags_u8(cpu: &mut CpuState, a: u8, b: u8, result: u8) {
    cpu.set_cf(a < b);
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x80 != 0);
    cpu.set_pf(parity_even(result));
    cpu.set_af(((a ^ b ^ result) & 0x10) != 0);
    let of = ((a ^ b) & (a ^ result) & 0x80) != 0;
    cpu.set_of(of);
}

/// 16-bit effective address from ModRM (real-mode / 16-bit address size).
fn ea_16(cpu: &CpuState, insn: &DecodedInsn) -> Result<(u64, bool), ExecError> {
    let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
    if m.mod_ == 3 {
        return Ok((0, true));
    }
    let off = calc_ea16(cpu, m.mod_, m.rm, insn.displacement)?;
    let seg = match insn.prefixes.segment_override {
        Some(0x2E) => &cpu.cs,
        Some(0x36) => &cpu.ss,
        Some(0x26) => &cpu.es,
        Some(0x64) => &cpu.fs,
        Some(0x65) => &cpu.gs,
        Some(0x3E) | None => {
            // Default DS, except BP-based uses SS.
            if m.rm == 2 || m.rm == 3 || (m.rm == 6 && m.mod_ != 0) {
                &cpu.ss
            } else {
                &cpu.ds
            }
        }
        _ => &cpu.ds,
    };
    Ok((linear_addr(seg, u64::from(off)), false))
}

fn calc_ea16(cpu: &CpuState, mod_: u8, rm: u8, displacement: i32) -> Result<u16, ExecError> {
    let disp = displacement as i16 as u16;
    let base = match rm {
        0 => cpu
            .gpr_u16(CpuState::RBX)
            .wrapping_add(cpu.gpr_u16(CpuState::RSI)),
        1 => cpu
            .gpr_u16(CpuState::RBX)
            .wrapping_add(cpu.gpr_u16(CpuState::RDI)),
        2 => cpu
            .gpr_u16(CpuState::RBP)
            .wrapping_add(cpu.gpr_u16(CpuState::RSI)),
        3 => cpu
            .gpr_u16(CpuState::RBP)
            .wrapping_add(cpu.gpr_u16(CpuState::RDI)),
        4 => cpu.gpr_u16(CpuState::RSI),
        5 => cpu.gpr_u16(CpuState::RDI),
        6 if mod_ == 0 => return Ok(disp),
        6 => cpu.gpr_u16(CpuState::RBP),
        7 => cpu.gpr_u16(CpuState::RBX),
        _ => return Err(ExecError::Unsupported(0)),
    };
    Ok(match mod_ {
        0 => base,
        1 | 2 => base.wrapping_add(disp),
        _ => return Err(ExecError::Unsupported(0)),
    })
}

fn read_rm_u8(cpu: &CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<u8, ExecError> {
    let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
    if m.mod_ == 3 {
        Ok(cpu.gpr_u8_low(m.rm as usize))
    } else {
        let (addr, _) = ea_16(cpu, insn)?;
        bus.read_u8(addr)
    }
}

fn write_rm_u8(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    insn: &DecodedInsn,
    val: u8,
) -> Result<(), ExecError> {
    let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
    if m.mod_ == 3 {
        cpu.set_gpr_u8_low(m.rm as usize, val);
        Ok(())
    } else {
        let (addr, _) = ea_16(cpu, insn)?;
        bus.write_u8(addr, val)
    }
}

fn read_rm_u16(cpu: &CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<u16, ExecError> {
    let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
    if m.mod_ == 3 {
        Ok(cpu.gpr_u16(m.rm as usize))
    } else {
        let (addr, _) = ea_16(cpu, insn)?;
        bus.read_u16(addr)
    }
}

fn write_rm_u16(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    insn: &DecodedInsn,
    val: u16,
) -> Result<(), ExecError> {
    let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
    if m.mod_ == 3 {
        cpu.set_gpr_u16(m.rm as usize, val);
        Ok(())
    } else {
        let (addr, _) = ea_16(cpu, insn)?;
        bus.write_u16(addr, val)
    }
}

fn push16(cpu: &mut CpuState, bus: &mut dyn Bus, val: u16) -> Result<(), ExecError> {
    let sp = cpu.gpr_u16(CpuState::RSP).wrapping_sub(2);
    cpu.set_gpr_u16(CpuState::RSP, sp);
    let addr = linear_addr(&cpu.ss, u64::from(sp));
    bus.write_u16(addr, val)
}

fn pop16(cpu: &mut CpuState, bus: &mut dyn Bus) -> Result<u16, ExecError> {
    let sp = cpu.gpr_u16(CpuState::RSP);
    let addr = linear_addr(&cpu.ss, u64::from(sp));
    let v = bus.read_u16(addr)?;
    cpu.set_gpr_u16(CpuState::RSP, sp.wrapping_add(2));
    Ok(v)
}

/// ModRM.reg → segment register index for MOV Sreg forms (SDM Vol. 2, MOV).
/// Returns None for reserved encodings (6, 7) which cause #UD.
fn sreg_from_modrm_reg(reg: u8) -> Option<u8> {
    match reg {
        0..=5 => Some(reg),
        _ => None,
    }
}

fn read_sreg_selector(cpu: &CpuState, sreg: u8) -> u16 {
    match sreg {
        0 => cpu.es.selector,
        1 => cpu.cs.selector,
        2 => cpu.ss.selector,
        3 => cpu.ds.selector,
        4 => cpu.fs.selector,
        5 => cpu.gs.selector,
        _ => unreachable!("sreg filtered by sreg_from_modrm_reg"),
    }
}

fn write_sreg_real_mode(cpu: &mut CpuState, sreg: u8, selector: u16) -> Result<(), ExecError> {
    // MOV to CS is invalid (#UD). Spec: Intel SDM Vol. 2 "MOV" — MOV to CS.
    if sreg == 1 {
        return Err(ExecError::Unsupported(0x8E));
    }
    let seg = x86_core::SegmentReg::real_mode(selector);
    match sreg {
        0 => cpu.es = seg,
        2 => cpu.ss = seg,
        3 => cpu.ds = seg,
        4 => cpu.fs = seg,
        5 => cpu.gs = seg,
        _ => return Err(ExecError::Unsupported(0x8E)),
    }
    Ok(())
}

/// SI/DI step for string ops: +size if DF=0, −size if DF=1 (SDM Vol. 1 §3.4.3).
fn string_index_delta(cpu: &CpuState, size: u16) -> u16 {
    if cpu.direction_flag() {
        size.wrapping_neg()
    } else {
        size
    }
}

fn data_seg_for_string_src<'a>(cpu: &'a CpuState, insn: &DecodedInsn) -> &'a x86_core::SegmentReg {
    match insn.prefixes.segment_override {
        Some(0x26) => &cpu.es,
        Some(0x2E) => &cpu.cs,
        Some(0x36) => &cpu.ss,
        Some(0x64) => &cpu.fs,
        Some(0x65) => &cpu.gs,
        Some(0x3E) | None => &cpu.ds,
        _ => &cpu.ds,
    }
}

/// SF/ZF/PF for shift results (SHL/SHR/SAR). AF undefined — left unchanged.
/// Spec: Intel SDM Vol. 2 SAL/SAR/SHL/SHR — Flags Affected.
fn set_shift_result_flags_u8(cpu: &mut CpuState, result: u8) {
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x80 != 0);
    cpu.set_pf(parity_even(result));
}

fn set_shift_result_flags_u16(cpu: &mut CpuState, result: u16) {
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x8000 != 0);
    cpu.set_pf(parity_even(result as u8));
}

/// Group 2 byte ops (D0/C0/D2). Spec: SDM Vol. 2 ROL/ROR/RCL/RCR/SHL/SHR/SAR.
/// `raw_count` is masked to 5 bits; count 0 leaves dest and flags unchanged.
fn grp2_u8(cpu: &mut CpuState, reg: u8, mut val: u8, raw_count: u8) -> Result<u8, ExecError> {
    let count = raw_count & 0x1F;
    if count == 0 {
        return Ok(val);
    }
    match reg {
        0 => {
            // ROL — tempCOUNT = COUNT mod 8; CF = LSB(result) when COUNT>0.
            let n = count % 8;
            if n != 0 {
                val = val.rotate_left(u32::from(n));
            }
            let new_cf = (val & 1) != 0;
            cpu.set_cf(new_cf);
            if count == 1 {
                cpu.set_of(((val & 0x80) != 0) ^ new_cf);
            }
            Ok(val)
        }
        1 => {
            let n = count % 8;
            if n != 0 {
                val = val.rotate_right(u32::from(n));
            }
            let new_cf = (val & 0x80) != 0;
            cpu.set_cf(new_cf);
            if count == 1 {
                cpu.set_of(((val ^ (val << 1)) & 0x80) != 0);
            }
            Ok(val)
        }
        2 => {
            // RCL — rotate through CF; tempCOUNT = COUNT mod 9.
            let n = count % 9;
            for _ in 0..n {
                let new_cf = (val & 0x80) != 0;
                val = (val << 1) | u8::from(cpu.rflags & 1 != 0);
                cpu.set_cf(new_cf);
            }
            if count == 1 {
                let cf = cpu.rflags & 1 != 0;
                cpu.set_of(((val & 0x80) != 0) ^ cf);
            }
            Ok(val)
        }
        3 => {
            let n = count % 9;
            for _ in 0..n {
                let new_cf = (val & 1) != 0;
                val = (val >> 1) | (u8::from(cpu.rflags & 1 != 0) << 7);
                cpu.set_cf(new_cf);
            }
            if count == 1 {
                cpu.set_of(((val ^ (val << 1)) & 0x80) != 0);
            }
            Ok(val)
        }
        4 => {
            // SHL/SAL
            for _ in 0..count {
                cpu.set_cf((val & 0x80) != 0);
                val <<= 1;
            }
            if count == 1 {
                let cf = cpu.rflags & 1 != 0;
                cpu.set_of(((val & 0x80) != 0) ^ cf);
            }
            set_shift_result_flags_u8(cpu, val);
            Ok(val)
        }
        5 => {
            let orig = val;
            for _ in 0..count {
                cpu.set_cf((val & 1) != 0);
                val >>= 1;
            }
            if count == 1 {
                cpu.set_of((orig & 0x80) != 0);
            }
            set_shift_result_flags_u8(cpu, val);
            Ok(val)
        }
        6 => Err(ExecError::Unsupported(0xD0)), // reserved encoding
        7 => {
            for _ in 0..count {
                cpu.set_cf((val & 1) != 0);
                val = ((val as i8) >> 1) as u8;
            }
            if count == 1 {
                cpu.set_of(false);
            }
            set_shift_result_flags_u8(cpu, val);
            Ok(val)
        }
        _ => Err(ExecError::Unsupported(0xD0)),
    }
}

/// Group 2 word ops (D1/C1/D3). Spec: SDM Vol. 2 ROL/ROR/RCL/RCR/SHL/SHR/SAR.
fn grp2_u16(cpu: &mut CpuState, reg: u8, mut val: u16, raw_count: u8) -> Result<u16, ExecError> {
    let count = raw_count & 0x1F;
    if count == 0 {
        return Ok(val);
    }
    match reg {
        0 => {
            let n = count % 16;
            if n != 0 {
                val = val.rotate_left(u32::from(n));
            }
            let new_cf = (val & 1) != 0;
            cpu.set_cf(new_cf);
            if count == 1 {
                cpu.set_of(((val & 0x8000) != 0) ^ new_cf);
            }
            Ok(val)
        }
        1 => {
            let n = count % 16;
            if n != 0 {
                val = val.rotate_right(u32::from(n));
            }
            let new_cf = (val & 0x8000) != 0;
            cpu.set_cf(new_cf);
            if count == 1 {
                cpu.set_of(((val ^ (val << 1)) & 0x8000) != 0);
            }
            Ok(val)
        }
        2 => {
            let n = count % 17;
            for _ in 0..n {
                let new_cf = (val & 0x8000) != 0;
                val = (val << 1) | u16::from(cpu.rflags & 1 != 0);
                cpu.set_cf(new_cf);
            }
            if count == 1 {
                let cf = cpu.rflags & 1 != 0;
                cpu.set_of(((val & 0x8000) != 0) ^ cf);
            }
            Ok(val)
        }
        3 => {
            let n = count % 17;
            for _ in 0..n {
                let new_cf = (val & 1) != 0;
                val = (val >> 1) | (u16::from(cpu.rflags & 1 != 0) << 15);
                cpu.set_cf(new_cf);
            }
            if count == 1 {
                cpu.set_of(((val ^ (val << 1)) & 0x8000) != 0);
            }
            Ok(val)
        }
        4 => {
            for _ in 0..count {
                cpu.set_cf((val & 0x8000) != 0);
                val <<= 1;
            }
            if count == 1 {
                let cf = cpu.rflags & 1 != 0;
                cpu.set_of(((val & 0x8000) != 0) ^ cf);
            }
            set_shift_result_flags_u16(cpu, val);
            Ok(val)
        }
        5 => {
            let orig = val;
            for _ in 0..count {
                cpu.set_cf((val & 1) != 0);
                val >>= 1;
            }
            if count == 1 {
                cpu.set_of((orig & 0x8000) != 0);
            }
            set_shift_result_flags_u16(cpu, val);
            Ok(val)
        }
        6 => Err(ExecError::Unsupported(0xD1)),
        7 => {
            for _ in 0..count {
                cpu.set_cf((val & 1) != 0);
                val = ((val as i16) >> 1) as u16;
            }
            if count == 1 {
                cpu.set_of(false);
            }
            set_shift_result_flags_u16(cpu, val);
            Ok(val)
        }
        _ => Err(ExecError::Unsupported(0xD1)),
    }
}

/// Short Jcc condition for opcodes 0x70–0x7F (Intel SDM Vol. 2, Jcc).
fn jcc_condition(cpu: &CpuState, opcode: u8) -> bool {
    let cf = cpu.rflags & 1 != 0;
    let pf = cpu.rflags & (1 << 2) != 0;
    let zf = cpu.rflags & (1 << 6) != 0;
    let sf = cpu.rflags & (1 << 7) != 0;
    let of = cpu.rflags & (1 << 11) != 0;
    match opcode {
        0x70 => of,                // JO
        0x71 => !of,               // JNO
        0x72 => cf,                // JB / JC / JNAE
        0x73 => !cf,               // JAE / JNB / JNC
        0x74 => zf,                // JE / JZ
        0x75 => !zf,               // JNE / JNZ
        0x76 => cf || zf,          // JBE / JNA
        0x77 => !cf && !zf,        // JA / JNBE
        0x78 => sf,                // JS
        0x79 => !sf,               // JNS
        0x7A => pf,                // JP / JPE
        0x7B => !pf,               // JNP / JPO
        0x7C => sf != of,          // JL / JNGE
        0x7D => sf == of,          // JGE / JNL
        0x7E => zf || (sf != of),  // JLE / JNG
        0x7F => !zf && (sf == of), // JG / JNLE
        _ => false,
    }
}

fn fetch_decode(cpu: &CpuState, bus: &mut dyn Bus) -> Result<x86_decode::DecodedInsn, ExecError> {
    // Grow the window until decode succeeds or we hit the 15-byte SDM limit.
    let mut buf = Vec::with_capacity(15);
    loop {
        if buf.len() >= 15 {
            return Err(ExecError::Decode(DecodeError::TooLong));
        }
        let ip = u64::from(cpu.ip16()).wrapping_add(buf.len() as u64) & 0xFFFF;
        let addr = linear_addr(&cpu.cs, ip);
        buf.push(bus.read_u8(addr)?);
        match decode(&buf) {
            Ok(insn) => return Ok(insn),
            Err(DecodeError::Truncated) => continue,
            Err(e) => return Err(ExecError::Decode(e)),
        }
    }
}

/// Real-mode software interrupt delivery through the IVT at `IDTR.base`.
///
/// Spec: Intel SDM Vol. 2 "INT n/INTO/INT3/INT1"; Vol. 3 §6.4.
fn real_mode_software_interrupt(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    vector: u8,
    return_ip: u16,
) -> Result<(), ExecError> {
    let flags16 = cpu.rflags as u16;
    push16(cpu, bus, flags16)?;
    push16(cpu, bus, cpu.cs.selector)?;
    push16(cpu, bus, return_ip)?;
    // Clear IF and TF (Vol. 2 INT n Operation, real-address mode).
    cpu.rflags &= !((1 << 9) | (1 << 8));
    let entry = cpu.idtr.base.wrapping_add(u64::from(vector) * 4);
    let offset = bus.read_u16(entry)?;
    let selector = bus.read_u16(entry.wrapping_add(2))?;
    cpu.cs = x86_core::SegmentReg::real_mode_code(selector);
    cpu.set_ip16(offset);
    Ok(())
}

/// Execute a single instruction at CS:IP.
pub fn step(cpu: &mut CpuState, bus: &mut dyn Bus) -> Result<(), ExecError> {
    if cpu.halted {
        return Ok(());
    }
    let insn = fetch_decode(cpu, bus)?;
    let next_ip = cpu.ip16().wrapping_add(insn.length as u16);
    let op = insn.opcode;

    match op {
        0x06 => {
            // PUSH ES — Spec: Intel SDM Vol. 2 "PUSH".
            push16(cpu, bus, cpu.es.selector)?;
            cpu.set_ip16(next_ip);
        }
        0x07 => {
            // POP ES — Spec: Intel SDM Vol. 2 "POP".
            let sel = pop16(cpu, bus)?;
            cpu.es = x86_core::SegmentReg::real_mode(sel);
            cpu.set_ip16(next_ip);
        }
        0x0E => {
            // PUSH CS — Spec: Intel SDM Vol. 2 "PUSH".
            push16(cpu, bus, cpu.cs.selector)?;
            cpu.set_ip16(next_ip);
        }
        0x16 => {
            // PUSH SS — Spec: Intel SDM Vol. 2 "PUSH".
            push16(cpu, bus, cpu.ss.selector)?;
            cpu.set_ip16(next_ip);
        }
        0x17 => {
            // POP SS — Spec: Intel SDM Vol. 2 "POP".
            // Unsupported here: one-instruction interrupt inhibit after POP SS (Vol. 2).
            let sel = pop16(cpu, bus)?;
            cpu.ss = x86_core::SegmentReg::real_mode(sel);
            cpu.set_ip16(next_ip);
        }
        0x1E => {
            // PUSH DS — Spec: Intel SDM Vol. 2 "PUSH".
            push16(cpu, bus, cpu.ds.selector)?;
            cpu.set_ip16(next_ip);
        }
        0x1F => {
            // POP DS — Spec: Intel SDM Vol. 2 "POP".
            let sel = pop16(cpu, bus)?;
            cpu.ds = x86_core::SegmentReg::real_mode(sel);
            cpu.set_ip16(next_ip);
        }
        0xF4 => {
            cpu.halted = true;
            cpu.set_ip16(next_ip);
        }
        0xFA => {
            cpu.set_interrupt_flag(false);
            cpu.set_ip16(next_ip);
        }
        0xFB => {
            cpu.set_interrupt_flag(true);
            cpu.set_ip16(next_ip);
        }
        0x90 => cpu.set_ip16(next_ip),
        0x98 => {
            // CBW — sign-extend AL into AX. Spec: Intel SDM Vol. 2 "CBW/CWDE/CDQE".
            // Unsupported here: CWDE (opsize 32), CDQE (REX.W).
            let al = cpu.al() as i8 as i16 as u16;
            cpu.set_ax(al);
            cpu.set_ip16(next_ip);
        }
        0x99 => {
            // CWD — sign-extend AX into DX:AX. Spec: Intel SDM Vol. 2 "CWD/CDQ/CQO".
            // Unsupported here: CDQ (opsize 32), CQO (REX.W).
            let dx = if cpu.ax() & 0x8000 != 0 { 0xFFFFu16 } else { 0 };
            cpu.set_gpr_u16(CpuState::RDX, dx);
            cpu.set_ip16(next_ip);
        }
        0xF5 => {
            let cf = cpu.rflags & 1 != 0;
            cpu.set_cf(!cf);
            cpu.set_ip16(next_ip);
        }
        0xF8 => {
            // CLC — Spec: Intel SDM Vol. 2 "CLC".
            cpu.set_cf(false);
            cpu.set_ip16(next_ip);
        }
        0xF9 => {
            // STC — Spec: Intel SDM Vol. 2 "STC".
            cpu.set_cf(true);
            cpu.set_ip16(next_ip);
        }
        0xFC => {
            // CLD — Spec: Intel SDM Vol. 2 "CLD".
            cpu.set_direction_flag(false);
            cpu.set_ip16(next_ip);
        }
        0xFD => {
            // STD — Spec: Intel SDM Vol. 2 "STD".
            cpu.set_direction_flag(true);
            cpu.set_ip16(next_ip);
        }
        0xEC => {
            let port = cpu.gpr_u16(CpuState::RDX);
            let v = bus.port_in_u8(port)?;
            cpu.set_al(v);
            cpu.set_ip16(next_ip);
        }
        0xEE => {
            let port = cpu.gpr_u16(CpuState::RDX);
            bus.port_out_u8(port, cpu.al())?;
            cpu.set_ip16(next_ip);
        }
        0xE4 => {
            let port = insn.immediate as u16;
            let v = bus.port_in_u8(port)?;
            cpu.set_al(v);
            cpu.set_ip16(next_ip);
        }
        0xE6 => {
            let port = insn.immediate as u16;
            bus.port_out_u8(port, cpu.al())?;
            cpu.set_ip16(next_ip);
        }
        0xEB => {
            let target = next_ip.wrapping_add(insn.immediate as i16 as u16);
            cpu.set_ip16(target);
        }
        0xE9 => {
            let target = next_ip.wrapping_add(insn.immediate as i16 as u16);
            cpu.set_ip16(target);
        }
        0xEA => {
            // JMP far ptr16:16 — real-address mode.
            // Spec: Intel SDM Vol. 2 "JMP" (ptr16:16).
            // Unsupported here: protected-mode / task-gate forms; opsize 32 (ptr16:32).
            let offset = insn.immediate as u16;
            let selector = insn.displacement as u16;
            cpu.cs = x86_core::SegmentReg::real_mode_code(selector);
            cpu.set_ip16(offset);
        }
        0xE8 => {
            push16(cpu, bus, next_ip)?;
            let target = next_ip.wrapping_add(insn.immediate as i16 as u16);
            cpu.set_ip16(target);
        }
        0xC3 => {
            let ip = pop16(cpu, bus)?;
            cpu.set_ip16(ip);
        }
        0x9A => {
            // CALL far ptr16:16 — real-address mode.
            // Spec: Intel SDM Vol. 2 "CALL" (ptr16:16). Push CS then return IP; load CS:IP.
            // Unsupported here: protected-mode privilege / gate transfer; opsize 32 (ptr16:32).
            let offset = insn.immediate as u16;
            let selector = insn.displacement as u16;
            push16(cpu, bus, cpu.cs.selector)?;
            push16(cpu, bus, next_ip)?;
            cpu.cs = x86_core::SegmentReg::real_mode_code(selector);
            cpu.set_ip16(offset);
        }
        0xCB => {
            // RETF — far return, 16-bit stack frame (pop IP then CS).
            // Spec: Intel SDM Vol. 2 "RET" (far).
            // Unsupported here: RETF imm16 stack-release form (CA iw); opsize 32.
            let ip = pop16(cpu, bus)?;
            let cs_sel = pop16(cpu, bus)?;
            cpu.cs = x86_core::SegmentReg::real_mode_code(cs_sel);
            cpu.set_ip16(ip);
        }
        0x9C => {
            // PUSHF — 16-bit FLAGS in real-address mode (default opsize).
            // Spec: Intel SDM Vol. 2 "PUSHF/PUSHFD/PUSHFQ".
            push16(cpu, bus, cpu.rflags as u16)?;
            cpu.set_ip16(next_ip);
        }
        0x9D => {
            // POPF — 16-bit FLAGS in real-address mode (default opsize).
            // Spec: Intel SDM Vol. 2 "POPF/POPFD/POPFQ".
            // Unsupported here: IOPL/VIP/VIF privilege masking (protected / V86).
            let flags = pop16(cpu, bus)?;
            cpu.rflags = (cpu.rflags & !0xFFFF) | u64::from(flags) | 2;
            cpu.set_ip16(next_ip);
        }
        0xCC => {
            // INT3 — one-byte breakpoint; vector 3 via IVT (real-address mode).
            // Spec: Intel SDM Vol. 2 "INT3"; Vol. 3 §6.4.
            // Unsupported here: ICEBP/INT1 (F1); protected-mode privilege checks.
            real_mode_software_interrupt(cpu, bus, 3, next_ip)?;
        }
        0xCD => {
            // INT imm8 — real-address mode via IVT / IDTR base.
            // Spec: Intel SDM Vol. 2 "INT n", Vol. 3 §6.4 (real-address mode).
            real_mode_software_interrupt(cpu, bus, insn.immediate as u8, next_ip)?;
        }
        0xCF => {
            // IRET — real-address mode (16-bit stack frame).
            // Spec: Intel SDM Vol. 2 "IRET/IRETD/IRETQ".
            let ip = pop16(cpu, bus)?;
            let cs_sel = pop16(cpu, bus)?;
            let flags = pop16(cpu, bus)?;
            cpu.cs = x86_core::SegmentReg::real_mode_code(cs_sel);
            cpu.set_ip16(ip);
            // Preserve high RFLAGS; bit 1 of FLAGS is reserved-1.
            cpu.rflags = (cpu.rflags & !0xFFFF) | u64::from(flags) | 2;
        }
        0xD0 => {
            // Group 2 r/m8, 1 — Spec: Intel SDM Vol. 2 ROL/ROR/RCL/RCR/SHL/SHR/SAR.
            // Unsupported here: /6 reserved; AH/CH/DH/BH via high-byte rm.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let v = read_rm_u8(cpu, bus, &insn)?;
            let r = grp2_u8(cpu, m.reg, v, 1)?;
            write_rm_u8(cpu, bus, &insn, r)?;
            cpu.set_ip16(next_ip);
        }
        0xD1 => {
            // Group 2 r/m16, 1 — Spec: Intel SDM Vol. 2 ROL/ROR/RCL/RCR/SHL/SHR/SAR.
            // Unsupported here: opsize 32; /6 reserved.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let v = read_rm_u16(cpu, bus, &insn)?;
            let r = grp2_u16(cpu, m.reg, v, 1)?;
            write_rm_u16(cpu, bus, &insn, r)?;
            cpu.set_ip16(next_ip);
        }
        0xC0 => {
            // Group 2 r/m8, imm8 — Spec: Intel SDM Vol. 2 (COUNT masked to 5 bits).
            // Unsupported here: /6 reserved; AH/CH/DH/BH high-byte rm.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let v = read_rm_u8(cpu, bus, &insn)?;
            let r = grp2_u8(cpu, m.reg, v, insn.immediate as u8)?;
            write_rm_u8(cpu, bus, &insn, r)?;
            cpu.set_ip16(next_ip);
        }
        0xC1 => {
            // Group 2 r/m16, imm8 — Spec: Intel SDM Vol. 2 (COUNT masked to 5 bits).
            // Unsupported here: opsize 32; /6 reserved.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let v = read_rm_u16(cpu, bus, &insn)?;
            let r = grp2_u16(cpu, m.reg, v, insn.immediate as u8)?;
            write_rm_u16(cpu, bus, &insn, r)?;
            cpu.set_ip16(next_ip);
        }
        0xD2 => {
            // Group 2 r/m8, CL — Spec: Intel SDM Vol. 2 (COUNT = CL, masked to 5 bits).
            // Unsupported here: /6 reserved; AH/CH/DH/BH high-byte rm.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let v = read_rm_u8(cpu, bus, &insn)?;
            let r = grp2_u8(cpu, m.reg, v, cpu.gpr_u8_low(CpuState::RCX))?;
            write_rm_u8(cpu, bus, &insn, r)?;
            cpu.set_ip16(next_ip);
        }
        0xD3 => {
            // Group 2 r/m16, CL — Spec: Intel SDM Vol. 2 (COUNT = CL, masked to 5 bits).
            // Unsupported here: opsize 32; /6 reserved.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let v = read_rm_u16(cpu, bus, &insn)?;
            let r = grp2_u16(cpu, m.reg, v, cpu.gpr_u8_low(CpuState::RCX))?;
            write_rm_u16(cpu, bus, &insn, r)?;
            cpu.set_ip16(next_ip);
        }
        0xF6 => {
            // Group 3 r/m8 — NOT (/2) / NEG (/3). Spec: Intel SDM Vol. 2 "NOT"/"NEG".
            // Unsupported here: TEST (/0), MUL/IMUL/DIV/IDIV (/4–/7); AH/CH/DH/BH.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let v = read_rm_u8(cpu, bus, &insn)?;
            match m.reg {
                2 => {
                    // NOT — one's complement; flags unaffected.
                    write_rm_u8(cpu, bus, &insn, !v)?;
                }
                3 => {
                    // NEG — two's complement; flags as SUB from 0 (CF cleared iff operand was 0).
                    let r = v.wrapping_neg();
                    write_rm_u8(cpu, bus, &insn, r)?;
                    set_sub_flags_u8(cpu, 0, v, r);
                }
                _ => return Err(ExecError::Unsupported(op)),
            }
            cpu.set_ip16(next_ip);
        }
        0xF7 => {
            // Group 3 r/m16 — NOT (/2) / NEG (/3). Spec: Intel SDM Vol. 2 "NOT"/"NEG".
            // Unsupported here: TEST (/0), MUL/IMUL/DIV/IDIV (/4–/7); opsize 32.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let v = read_rm_u16(cpu, bus, &insn)?;
            match m.reg {
                2 => {
                    write_rm_u16(cpu, bus, &insn, !v)?;
                }
                3 => {
                    let r = v.wrapping_neg();
                    write_rm_u16(cpu, bus, &insn, r)?;
                    set_sub_flags_u16(cpu, 0, v, r);
                }
                _ => return Err(ExecError::Unsupported(op)),
            }
            cpu.set_ip16(next_ip);
        }
        0x70..=0x7F => {
            // Jcc rel8 — Spec: Intel SDM Vol. 2 "Jcc".
            // Unsupported here: near rel16/rel32 forms (0F 8x); JCXZ/JECXZ (E3).
            if jcc_condition(cpu, op) {
                cpu.set_ip16(next_ip.wrapping_add(insn.immediate as i16 as u16));
            } else {
                cpu.set_ip16(next_ip);
            }
        }
        0x40..=0x47 => {
            let idx = (op - 0x40) as usize;
            let old = cpu.gpr_u16(idx);
            let v = old.wrapping_add(1);
            let saved_cf = cpu.rflags & 1 != 0;
            cpu.set_gpr_u16(idx, v);
            set_add_flags_u16(cpu, old, 1, v);
            // INC does not modify CF (Intel SDM Vol. 2, INC).
            cpu.set_cf(saved_cf);
            cpu.set_ip16(next_ip);
        }
        0x50..=0x57 => {
            let idx = (op - 0x50) as usize;
            push16(cpu, bus, cpu.gpr_u16(idx))?;
            cpu.set_ip16(next_ip);
        }
        0x58..=0x5F => {
            let idx = (op - 0x58) as usize;
            let v = pop16(cpu, bus)?;
            cpu.set_gpr_u16(idx, v);
            cpu.set_ip16(next_ip);
        }
        0xA4 => {
            // MOVSB — Spec: Intel SDM Vol. 2 "MOVS/MOVSB/MOVSW/MOVSD/MOVSQ".
            // Unsupported here: MOVSW/D/Q; REP/REPE/REPNE prefixes.
            let si = cpu.gpr_u16(CpuState::RSI);
            let di = cpu.gpr_u16(CpuState::RDI);
            let src = linear_addr(data_seg_for_string_src(cpu, &insn), u64::from(si));
            let dst = linear_addr(&cpu.es, u64::from(di));
            let v = bus.read_u8(src)?;
            bus.write_u8(dst, v)?;
            let d = string_index_delta(cpu, 1);
            cpu.set_gpr_u16(CpuState::RSI, si.wrapping_add(d));
            cpu.set_gpr_u16(CpuState::RDI, di.wrapping_add(d));
            cpu.set_ip16(next_ip);
        }
        0xAA => {
            // STOSB — Spec: Intel SDM Vol. 2 "STOS/STOSB/STOSW/STOSD/STOSQ".
            // Unsupported here: STOSW/D/Q; REP prefix.
            let di = cpu.gpr_u16(CpuState::RDI);
            let dst = linear_addr(&cpu.es, u64::from(di));
            bus.write_u8(dst, cpu.al())?;
            let d = string_index_delta(cpu, 1);
            cpu.set_gpr_u16(CpuState::RDI, di.wrapping_add(d));
            cpu.set_ip16(next_ip);
        }
        0xAC => {
            // LODSB — Spec: Intel SDM Vol. 2 "LODS/LODSB/LODSW/LODSD/LODSQ".
            // Unsupported here: LODSW/D/Q; REP prefix.
            let si = cpu.gpr_u16(CpuState::RSI);
            let src = linear_addr(data_seg_for_string_src(cpu, &insn), u64::from(si));
            let v = bus.read_u8(src)?;
            cpu.set_al(v);
            let d = string_index_delta(cpu, 1);
            cpu.set_gpr_u16(CpuState::RSI, si.wrapping_add(d));
            cpu.set_ip16(next_ip);
        }
        0xB0..=0xB7 => {
            let idx = (op - 0xB0) as usize;
            cpu.set_gpr_u8_low(idx, insn.immediate as u8);
            cpu.set_ip16(next_ip);
        }
        0xB8..=0xBF => {
            let idx = (op - 0xB8) as usize;
            cpu.set_gpr_u16(idx, insn.immediate as u16);
            cpu.set_ip16(next_ip);
        }
        0x8A => {
            // MOV r8, r/m8
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let v = read_rm_u8(cpu, bus, &insn)?;
            cpu.set_gpr_u8_low(m.reg as usize, v);
            cpu.set_ip16(next_ip);
        }
        0x88 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let v = cpu.gpr_u8_low(m.reg as usize);
            write_rm_u8(cpu, bus, &insn, v)?;
            cpu.set_ip16(next_ip);
        }
        0x8B => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let v = read_rm_u16(cpu, bus, &insn)?;
            cpu.set_gpr_u16(m.reg as usize, v);
            cpu.set_ip16(next_ip);
        }
        0x89 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let v = cpu.gpr_u16(m.reg as usize);
            write_rm_u16(cpu, bus, &insn, v)?;
            cpu.set_ip16(next_ip);
        }
        0x8C => {
            // MOV r/m16, Sreg — real-address mode, 16-bit opsize.
            // Spec: Intel SDM Vol. 2 "MOV" (r/m16, Sreg).
            // Unsupported here: opsize 32 (zero-extend to r32); protected-mode side effects.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let sreg = sreg_from_modrm_reg(m.reg).ok_or(ExecError::Unsupported(op))?;
            let v = read_sreg_selector(cpu, sreg);
            write_rm_u16(cpu, bus, &insn, v)?;
            cpu.set_ip16(next_ip);
        }
        0x8D => {
            // LEA r16, m — load 16-bit effective address (offset only; no memory read).
            // Spec: Intel SDM Vol. 2 "LEA".
            // Unsupported here: opsize 32; address-size 32; register source (#UD).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if m.mod_ == 3 {
                return Err(ExecError::Unsupported(op));
            }
            let off = calc_ea16(cpu, m.mod_, m.rm, insn.displacement)?;
            cpu.set_gpr_u16(m.reg as usize, off);
            cpu.set_ip16(next_ip);
        }
        0x8E => {
            // MOV Sreg, r/m16 — real-address mode load (base = selector << 4).
            // Spec: Intel SDM Vol. 2 "MOV" (Sreg, r/m16); Vol. 3 §3.4.2.
            // Unsupported here: MOV to CS (#UD); reserved Sreg encodings (#UD as Unsupported);
            // protected-mode descriptor checks; one-instruction IRQ inhibit after MOV SS.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let sreg = sreg_from_modrm_reg(m.reg).ok_or(ExecError::Unsupported(op))?;
            let v = read_rm_u16(cpu, bus, &insn)?;
            write_sreg_real_mode(cpu, sreg, v)?;
            cpu.set_ip16(next_ip);
        }
        0x84 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_rm_u8(cpu, bus, &insn)?;
            let b = cpu.gpr_u8_low(m.reg as usize);
            set_logic_flags_u8(cpu, a & b);
            cpu.set_ip16(next_ip);
        }
        0x85 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_rm_u16(cpu, bus, &insn)?;
            let b = cpu.gpr_u16(m.reg as usize);
            set_logic_flags_u16(cpu, a & b);
            cpu.set_ip16(next_ip);
        }
        0x31 | 0x33 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if op == 0x31 {
                let a = read_rm_u16(cpu, bus, &insn)?;
                let b = cpu.gpr_u16(m.reg as usize);
                let r = a ^ b;
                write_rm_u16(cpu, bus, &insn, r)?;
                set_logic_flags_u16(cpu, r);
            } else {
                let a = cpu.gpr_u16(m.reg as usize);
                let b = read_rm_u16(cpu, bus, &insn)?;
                let r = a ^ b;
                cpu.set_gpr_u16(m.reg as usize, r);
                set_logic_flags_u16(cpu, r);
            }
            cpu.set_ip16(next_ip);
        }
        0x01 | 0x03 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if op == 0x01 {
                let a = read_rm_u16(cpu, bus, &insn)?;
                let b = cpu.gpr_u16(m.reg as usize);
                let r = a.wrapping_add(b);
                write_rm_u16(cpu, bus, &insn, r)?;
                set_add_flags_u16(cpu, a, b, r);
            } else {
                let a = cpu.gpr_u16(m.reg as usize);
                let b = read_rm_u16(cpu, bus, &insn)?;
                let r = a.wrapping_add(b);
                cpu.set_gpr_u16(m.reg as usize, r);
                set_add_flags_u16(cpu, a, b, r);
            }
            cpu.set_ip16(next_ip);
        }
        0x29 | 0x2B => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if op == 0x29 {
                let a = read_rm_u16(cpu, bus, &insn)?;
                let b = cpu.gpr_u16(m.reg as usize);
                let r = a.wrapping_sub(b);
                write_rm_u16(cpu, bus, &insn, r)?;
                set_sub_flags_u16(cpu, a, b, r);
            } else {
                let a = cpu.gpr_u16(m.reg as usize);
                let b = read_rm_u16(cpu, bus, &insn)?;
                let r = a.wrapping_sub(b);
                cpu.set_gpr_u16(m.reg as usize, r);
                set_sub_flags_u16(cpu, a, b, r);
            }
            cpu.set_ip16(next_ip);
        }
        0x39 | 0x3B => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if op == 0x39 {
                let a = read_rm_u16(cpu, bus, &insn)?;
                let b = cpu.gpr_u16(m.reg as usize);
                set_sub_flags_u16(cpu, a, b, a.wrapping_sub(b));
            } else {
                let a = cpu.gpr_u16(m.reg as usize);
                let b = read_rm_u16(cpu, bus, &insn)?;
                set_sub_flags_u16(cpu, a, b, a.wrapping_sub(b));
            }
            cpu.set_ip16(next_ip);
        }
        0x04 => {
            let a = cpu.al();
            let b = insn.immediate as u8;
            let r = a.wrapping_add(b);
            cpu.set_al(r);
            // minimal flags for 8-bit add
            cpu.set_cf((a as u16) + (b as u16) > 0xFF);
            cpu.set_zf(r == 0);
            cpu.set_sf(r & 0x80 != 0);
            cpu.set_pf(parity_even(r));
            cpu.set_ip16(next_ip);
        }
        0x09 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_rm_u16(cpu, bus, &insn)?;
            let b = cpu.gpr_u16(m.reg as usize);
            let r = a | b;
            write_rm_u16(cpu, bus, &insn, r)?;
            set_logic_flags_u16(cpu, r);
            cpu.set_ip16(next_ip);
        }
        _ => return Err(ExecError::Unsupported(op)),
    }

    Ok(())
}

/// Run until HLT or `max_steps`.
pub fn run(cpu: &mut CpuState, bus: &mut dyn Bus, max_steps: u64) -> Result<u64, ExecError> {
    let mut n = 0u64;
    while n < max_steps && !cpu.halted {
        step(cpu, bus)?;
        n += 1;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct VecBus {
        mem: Vec<u8>,
        ports: Vec<u8>,
    }

    impl Bus for VecBus {
        fn read_u8(&mut self, addr: u64) -> Result<u8, ExecError> {
            let i = addr as usize;
            if i >= self.mem.len() {
                return Err(ExecError::MemoryFault(addr));
            }
            Ok(self.mem[i])
        }
        fn write_u8(&mut self, addr: u64, val: u8) -> Result<(), ExecError> {
            let i = addr as usize;
            if i >= self.mem.len() {
                return Err(ExecError::MemoryFault(addr));
            }
            self.mem[i] = val;
            Ok(())
        }
        fn port_in_u8(&mut self, _port: u16) -> Result<u8, ExecError> {
            Ok(0xFF)
        }
        fn port_out_u8(&mut self, _port: u16, val: u8) -> Result<(), ExecError> {
            self.ports.push(val);
            Ok(())
        }
    }

    #[test]
    fn xor_reg_clears_and_sets_zf() {
        let mut cpu = CpuState::reset();
        cpu.cs.base = 0;
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RAX, 0x1234);
        // 31 C0  xor ax, ax
        let mut bus = VecBus {
            mem: vec![0x31, 0xC0, 0xF4],
            ports: vec![],
        };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0);
        assert_ne!(cpu.rflags & (1 << 6), 0);
    }

    #[test]
    fn out_dx_al_writes_port() {
        let mut cpu = CpuState::reset();
        cpu.cs.base = 0;
        cpu.rip = 0;
        cpu.set_al(b'Z');
        cpu.set_gpr_u16(CpuState::RDX, 0x3F8);
        let mut bus = VecBus {
            mem: vec![0xEE, 0xF4],
            ports: vec![],
        };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.ports, b"Z");
    }

    /// INT n: push FLAGS/CS/IP, clear IF+TF, load IVT[vector] (SDM Vol. 2 / Vol. 3 §6.4).
    #[test]
    fn int_imm8_real_mode_via_ivt() {
        let mut mem = vec![0u8; 0x10000];
        // IVT[0x21] at 0x84: offset 0x2000, segment 0x1000 → linear 0x12000 (out of this bus).
        // Use segment 0x0000 offset 0x0800 so handler is in the same 64 KiB image.
        mem[0x84] = 0x00;
        mem[0x85] = 0x08; // offset 0x0800
        mem[0x86] = 0x00;
        mem[0x87] = 0x00; // segment 0x0000
                          // Code at CS:IP = 0:0 — INT 21h
        mem[0] = 0xCD;
        mem[1] = 0x21;
        // Handler at 0x800: HLT
        mem[0x800] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        cpu.rflags |= 1 << 8; // TF set so we can observe clear
        cpu.rflags |= 1; // CF sticky so FLAGS round-trip is visible
        let saved_flags = cpu.rflags as u16;

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.cs.selector, 0);
        assert_eq!(cpu.ip16(), 0x0800);
        assert!(!cpu.interrupt_flag());
        assert_eq!(cpu.rflags & (1 << 8), 0);
        // Stack: FLAGS, CS, IP (top)
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF8);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 2); // return IP after INT
        assert_eq!(bus.read_u16(0xFFFA).unwrap(), 0); // CS
        assert_eq!(bus.read_u16(0xFFFC).unwrap(), saved_flags);
    }

    /// IRET restores IP/CS/FLAGS from the 16-bit real-mode interrupt frame.
    #[test]
    fn iret_restores_real_mode_frame() {
        let mut mem = vec![0u8; 0x10000];
        mem[0x100] = 0xCF; // IRET
                           // Pre-built frame at SS:SP = 0:0xFFF8 — IP, CS, FLAGS
        mem[0xFFF8] = 0x34;
        mem[0xFFF9] = 0x12; // IP 0x1234
        mem[0xFFFA] = 0x00;
        mem[0xFFFB] = 0x20; // CS 0x2000
        mem[0xFFFC] = 0x03; // FLAGS: CF+reserved1 (IF clear)
        mem[0xFFFD] = 0x00;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0x100;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFF8);
        cpu.set_interrupt_flag(true);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.ip16(), 0x1234);
        assert_eq!(cpu.cs.selector, 0x2000);
        assert_eq!(cpu.cs.base, 0x2000u64 << 4);
        assert!(!cpu.interrupt_flag());
        assert_ne!(cpu.rflags & 1, 0); // CF restored
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE);
    }

    #[test]
    fn int_then_iret_round_trip() {
        let mut mem = vec![0u8; 0x10000];
        // IVT[0x10] → handler at 0000:0900
        mem[0x40] = 0x00;
        mem[0x41] = 0x09;
        mem[0x42] = 0x00;
        mem[0x43] = 0x00;
        // 0: INT 10h; HLT (return target)
        mem[0] = 0xCD;
        mem[1] = 0x10;
        mem[2] = 0xF4;
        // Handler: IRET
        mem[0x900] = 0xCF;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        let flags_before = cpu.rflags;

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap(); // INT
        step(&mut cpu, &mut bus).unwrap(); // IRET

        assert_eq!(cpu.ip16(), 2);
        assert_eq!(cpu.cs.selector, 0);
        assert_eq!(cpu.rflags & 0xFFFF, flags_before & 0xFFFF);
        assert!(cpu.interrupt_flag());
    }

    /// PUSHF pushes 16-bit FLAGS (SDM Vol. 2 PUSHF/PUSHFD/PUSHFQ, real-address mode).
    #[test]
    fn pushf_pushes_flags16() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x9C; // PUSHF
        mem[1] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        cpu.rflags |= 1; // CF
        let flags16 = cpu.rflags as u16;

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFC);
        assert_eq!(bus.read_u16(0xFFFC).unwrap(), flags16);
        assert_eq!(cpu.ip16(), 1);
    }

    /// POPF restores 16-bit FLAGS; reserved bit 1 stays set (SDM Vol. 2 POPF).
    #[test]
    fn popf_restores_flags16() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x9D; // POPF
        mem[1] = 0xF4;
        mem[0xFFFC] = 0x03; // CF + reserved1; IF clear
        mem[0xFFFD] = 0x00;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFC);
        cpu.set_interrupt_flag(true);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();

        assert!(!cpu.interrupt_flag());
        assert_ne!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & 2, 2);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE);
        assert_eq!(cpu.ip16(), 1);
    }

    #[test]
    fn pushf_popf_round_trip() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x9C; // PUSHF
        mem[1] = 0xFA; // CLI (clear IF in live flags)
        mem[2] = 0x9D; // POPF (restore)
        mem[3] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        cpu.rflags |= 1;
        let flags_before = cpu.rflags & 0xFFFF;

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap(); // PUSHF
        step(&mut cpu, &mut bus).unwrap(); // CLI
        assert!(!cpu.interrupt_flag());
        step(&mut cpu, &mut bus).unwrap(); // POPF

        assert_eq!(cpu.rflags & 0xFFFF, flags_before);
        assert!(cpu.interrupt_flag());
    }

    /// CALL far: push CS/IP, load ptr16:16 (SDM Vol. 2 CALL).
    #[test]
    fn call_far_pushes_cs_ip_and_loads_target() {
        let mut mem = vec![0u8; 0x10000];
        // CALL 0000:0800 — encoding 9A 00 08 00 00
        mem[0] = 0x9A;
        mem[1] = 0x00;
        mem[2] = 0x08;
        mem[3] = 0x00;
        mem[4] = 0x00;
        mem[0x800] = 0xF4; // HLT at target

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.cs.selector, 0);
        assert_eq!(cpu.ip16(), 0x0800);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFA);
        assert_eq!(bus.read_u16(0xFFFA).unwrap(), 5); // return IP
        assert_eq!(bus.read_u16(0xFFFC).unwrap(), 0); // return CS
    }

    /// RETF restores IP/CS from the far-call frame.
    #[test]
    fn retf_restores_cs_ip() {
        let mut mem = vec![0u8; 0x10000];
        mem[0x100] = 0xCB; // RETF
        mem[0xFFFA] = 0x34;
        mem[0xFFFB] = 0x12; // IP
        mem[0xFFFC] = 0x00;
        mem[0xFFFD] = 0x20; // CS 0x2000

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0x100;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFA);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.ip16(), 0x1234);
        assert_eq!(cpu.cs.selector, 0x2000);
        assert_eq!(cpu.cs.base, 0x2000u64 << 4);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE);
    }

    #[test]
    fn call_far_then_retf_round_trip() {
        let mut mem = vec![0u8; 0x10000];
        // 0: CALL 0000:0900; HLT
        mem[0] = 0x9A;
        mem[1] = 0x00;
        mem[2] = 0x09;
        mem[3] = 0x00;
        mem[4] = 0x00;
        mem[5] = 0xF4;
        // Handler: RETF
        mem[0x900] = 0xCB;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap(); // CALL far
        step(&mut cpu, &mut bus).unwrap(); // RETF

        assert_eq!(cpu.ip16(), 5);
        assert_eq!(cpu.cs.selector, 0);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE);
    }

    /// PUSH/POP DS updates selector and real-mode base (SDM Vol. 2 PUSH/POP; Vol. 3 §3.4.2).
    #[test]
    fn push_pop_ds_round_trip() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x1E; // PUSH DS
        mem[1] = 0x1F; // POP DS
        mem[2] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0x1234);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap(); // PUSH DS
        assert_eq!(bus.read_u16(0xFFFC).unwrap(), 0x1234);
        cpu.ds = x86_core::SegmentReg::real_mode(0); // clobber
        step(&mut cpu, &mut bus).unwrap(); // POP DS
        assert_eq!(cpu.ds.selector, 0x1234);
        assert_eq!(cpu.ds.base, 0x1234u64 << 4);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE);
    }

    #[test]
    fn push_cs_and_pop_es() {
        // Code lives at F000:0000 (linear 0xF0000); stack still uses SS=0.
        let mut mem = vec![0u8; 0x100000];
        mem[0xF0000] = 0x0E; // PUSH CS
        mem[0xF0001] = 0x07; // POP ES
        mem[0xF0002] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0xF000);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.es.selector, 0xF000);
        assert_eq!(cpu.es.base, 0xF000u64 << 4);
    }

    /// JMP far loads CS:IP from ptr16:16 without touching the stack (SDM Vol. 2 JMP).
    #[test]
    fn jmp_far_loads_cs_ip() {
        let mut mem = vec![0u8; 0x20000];
        // At 0000:0000 — JMP 1000:0200
        mem[0] = 0xEA;
        mem[1] = 0x00;
        mem[2] = 0x02;
        mem[3] = 0x00;
        mem[4] = 0x10;
        // Target linear = 0x1000<<4 + 0x200 = 0x10200
        mem[0x10200] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.cs.selector, 0x1000);
        assert_eq!(cpu.cs.base, 0x1000u64 << 4);
        assert_eq!(cpu.ip16(), 0x0200);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE); // stack unchanged
    }

    /// MOV AX, DS / MOV ES, AX — reg forms (SDM Vol. 2 MOV r/m16,Sreg / Sreg,r/m16).
    #[test]
    fn mov_sreg_reg_forms() {
        let mut mem = vec![0u8; 0x10000];
        // 8C D8 = MOV AX, DS; 8E C0 = MOV ES, AX
        mem[0] = 0x8C;
        mem[1] = 0xD8;
        mem[2] = 0x8E;
        mem[3] = 0xC0;
        mem[4] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0x1234);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RAX, 0);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x1234);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.es.selector, 0x1234);
        assert_eq!(cpu.es.base, 0x1234u64 << 4);
    }

    /// MOV r/m16, Sreg and MOV Sreg, r/m16 memory forms (SDM Vol. 2 MOV).
    #[test]
    fn mov_sreg_mem_forms() {
        let mut mem = vec![0u8; 0x10000];
        // Use ES as Sreg so DS remains 0 for the EA default segment.
        // 8C 06 00 20 = MOV [0x2000], ES
        // 8E 06 00 20 = MOV ES, [0x2000]
        mem[0] = 0x8C;
        mem[1] = 0x06;
        mem[2] = 0x00;
        mem[3] = 0x20;
        mem[4] = 0x8E;
        mem[5] = 0x06;
        mem[6] = 0x00;
        mem[7] = 0x20;
        mem[8] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0xABCD);
        cpu.rip = 0;

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x2000).unwrap(), 0xABCD);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.es.selector, 0xABCD);
        assert_eq!(cpu.es.base, 0xABCDu64 << 4);
    }

    /// MOV CS, r/m16 is invalid (#UD) — reported as Unsupported (SDM Vol. 2 MOV).
    #[test]
    fn mov_to_cs_unsupported() {
        let mut mem = vec![0u8; 0x10000];
        // 8E C8 = MOV CS, AX
        mem[0] = 0x8E;
        mem[1] = 0xC8;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RAX, 0x1000);

        let mut bus = VecBus { mem, ports: vec![] };
        assert_eq!(step(&mut cpu, &mut bus), Err(ExecError::Unsupported(0x8E)));
        assert_eq!(cpu.cs.selector, 0); // unchanged
    }

    /// LODSB/STOSB/MOVSB advance SI/DI by DF (SDM Vol. 2 LODS/STOS/MOVS).
    #[test]
    fn string_byte_ops_df_forward() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xAC; // LODSB
        mem[1] = 0xAA; // STOSB
        mem[2] = 0xA4; // MOVSB
        mem[3] = 0xF4;
        mem[0x1000] = b'X';
        mem[0x1001] = b'Y';

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_direction_flag(false);
        cpu.set_gpr_u16(CpuState::RSI, 0x1000);
        cpu.set_gpr_u16(CpuState::RDI, 0x2000);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap(); // LODSB
        assert_eq!(cpu.al(), b'X');
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x1001);

        step(&mut cpu, &mut bus).unwrap(); // STOSB
        assert_eq!(bus.read_u8(0x2000).unwrap(), b'X');
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x2001);

        // MOVSB: DS:[SI]=Y → ES:[DI]
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x2001).unwrap(), b'Y');
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x1002);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x2002);
    }

    #[test]
    fn lodsb_df_backward() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xAC;
        mem[1] = 0xF4;
        mem[0x1000] = 0xAB;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_direction_flag(true);
        cpu.set_gpr_u16(CpuState::RSI, 0x1000);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xAB);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x0FFF);
    }

    /// Short Jcc take/not-take for unsigned and signed conditions (SDM Vol. 2 Jcc).
    #[test]
    fn jcc_short_conditions() {
        // Layout: JA +2 → target HLT at ip=4; fall-through HLT at ip=2.
        // 77 02 = JA +2; F4; F4
        let run = |opcode: u8, flags: u64, expect_taken: bool| {
            let mut mem = vec![0u8; 0x10000];
            mem[0] = opcode;
            mem[1] = 0x02; // rel8 = +2 → land on second HLT
            mem[2] = 0xF4;
            mem[3] = 0x90;
            mem[4] = 0xF4;

            let mut cpu = CpuState::reset();
            cpu.cs = x86_core::SegmentReg::real_mode_code(0);
            cpu.rip = 0;
            cpu.rflags = 0x2 | flags;
            let mut bus = VecBus { mem, ports: vec![] };
            step(&mut cpu, &mut bus).unwrap();
            if expect_taken {
                assert_eq!(cpu.ip16(), 4, "op {opcode:#x} flags {flags:#x} should take");
            } else {
                assert_eq!(
                    cpu.ip16(),
                    2,
                    "op {opcode:#x} flags {flags:#x} should fall through"
                );
            }
        };

        // JA (77): CF=0 and ZF=0
        run(0x77, 0, true);
        run(0x77, 1, false); // CF
        run(0x77, 1 << 6, false); // ZF
                                  // JAE (73): CF=0
        run(0x73, 0, true);
        run(0x73, 1, false);
        // JBE (76): CF|ZF
        run(0x76, 0, false);
        run(0x76, 1, true);
        run(0x76, 1 << 6, true);
        // JL (7C): SF != OF
        run(0x7C, 0, false);
        run(0x7C, 1 << 7, true); // SF
        run(0x7C, (1 << 7) | (1 << 11), false); // SF+OF
                                                // JG (7F): ZF=0 and SF==OF
        run(0x7F, 0, true);
        run(0x7F, 1 << 6, false);
        run(0x7F, 1 << 7, false);
        // JO (70) / JS (78) / JP (7A)
        run(0x70, 1 << 11, true);
        run(0x70, 0, false);
        run(0x78, 1 << 7, true);
        run(0x7A, 1 << 2, true);
        // JGE (7D) / JLE (7E) / JNO (71) / JNS (79) / JNP (7B)
        run(0x7D, 0, true);
        run(0x7E, 1 << 6, true);
        run(0x71, 0, true);
        run(0x79, 0, true);
        run(0x7B, 0, true);
    }

    /// INT3 delivers vector 3 through the IVT like INT 3 (SDM Vol. 2 INT3; Vol. 3 §6.4).
    #[test]
    fn int3_real_mode_via_ivt() {
        let mut mem = vec![0u8; 0x10000];
        // IVT[3] at linear 0x0C: offset 0x0900, segment 0x0000
        mem[0x0C] = 0x00;
        mem[0x0D] = 0x09;
        mem[0x0E] = 0x00;
        mem[0x0F] = 0x00;
        mem[0] = 0xCC; // INT3
        mem[0x900] = 0xF4; // handler HLT

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        let saved_flags = cpu.rflags as u16;

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.cs.selector, 0);
        assert_eq!(cpu.ip16(), 0x0900);
        assert!(!cpu.interrupt_flag());
        // Stack top→: return IP (=1 after CC), CS, FLAGS
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF8);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 1);
        assert_eq!(bus.read_u16(0xFFFA).unwrap(), 0);
        assert_eq!(bus.read_u16(0xFFFC).unwrap(), saved_flags);
    }

    /// CLC/STC toggle CF only; CLD/STD toggle DF only (SDM Vol. 2).
    #[test]
    fn clc_stc_cld_std() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF9; // STC
        mem[1] = 0xF8; // CLC
        mem[2] = 0xFD; // STD
        mem[3] = 0xFC; // CLD
        mem[4] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_cf(false);
        cpu.set_direction_flag(false);
        let other = cpu.rflags;

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap(); // STC
        assert_ne!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & !1, other & !1);

        step(&mut cpu, &mut bus).unwrap(); // CLC
        assert_eq!(cpu.rflags & 1, 0);

        step(&mut cpu, &mut bus).unwrap(); // STD
        assert!(cpu.direction_flag());
        assert_eq!(cpu.rflags & 1, 0); // CF untouched

        step(&mut cpu, &mut bus).unwrap(); // CLD
        assert!(!cpu.direction_flag());
    }

    /// MOV AX, CS is valid (read CS selector).
    #[test]
    fn mov_from_cs_to_ax() {
        // Code at 1000:0000 → linear 0x10000
        let mut mem = vec![0u8; 0x20000];
        // 8C C8 = MOV AX, CS
        mem[0x10000] = 0x8C;
        mem[0x10001] = 0xC8;
        mem[0x10002] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0x1000);
        cpu.rip = 0;

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x1000);
    }

    /// Group 2 D0/D1 count=1: ROL/ROR/RCL/RCR/SHL/SHR/SAR (SDM Vol. 2).
    #[test]
    fn grp2_shift_rotate_by1_reg() {
        let run8 = |modrm_reg: u8, al: u8, cf_in: bool| -> (u8, bool, bool) {
            let mut mem = vec![0u8; 0x10000];
            // D0 C0+8*reg = op AL, 1
            mem[0] = 0xD0;
            mem[1] = 0xC0 | (modrm_reg << 3);
            mem[2] = 0xF4;
            let mut cpu = CpuState::reset();
            cpu.cs = x86_core::SegmentReg::real_mode_code(0);
            cpu.rip = 0;
            cpu.set_al(al);
            cpu.set_cf(cf_in);
            let mut bus = VecBus { mem, ports: vec![] };
            step(&mut cpu, &mut bus).unwrap();
            (cpu.al(), cpu.rflags & 1 != 0, cpu.rflags & (1 << 11) != 0)
        };

        // ROL AL,1: 0x81 → 0x03, CF=1, OF=MSB xor CF = 0 xor 1 = 1
        let (r, cf, of) = run8(0, 0x81, false);
        assert_eq!((r, cf, of), (0x03, true, true));

        // ROR AL,1: 0x03 → 0x81, CF=1, OF = two MSBs differ
        let (r, cf, of) = run8(1, 0x03, false);
        assert_eq!((r, cf), (0x81, true));
        assert!(of);

        // RCL AL,1 with CF=1: 0x40 → 0x81, CF=0, OF=1 xor 0 = 1
        let (r, cf, of) = run8(2, 0x40, true);
        assert_eq!((r, cf, of), (0x81, false, true));

        // RCR AL,1 with CF=1: 0x02 → 0x81, CF=0
        let (r, cf, _) = run8(3, 0x02, true);
        assert_eq!((r, cf), (0x81, false));

        // SHL AL,1: 0x40 → 0x80, CF=0, OF=1, SF=1, ZF=0
        let (r, cf, of) = run8(4, 0x40, false);
        assert_eq!((r, cf, of), (0x80, false, true));

        // SHR AL,1: 0x81 → 0x40, CF=1, OF=original MSB=1
        let (r, cf, of) = run8(5, 0x81, false);
        assert_eq!((r, cf, of), (0x40, true, true));

        // SAR AL,1: 0x81 → 0xC0, CF=1, OF=0, SF=1
        let (r, cf, of) = run8(7, 0x81, false);
        assert_eq!((r, cf, of), (0xC0, true, false));

        // Word SHL AX,1
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xD1;
        mem[1] = 0xE0; // SHL AX,1
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_ax(0x4000);
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x8000);
        assert_eq!(cpu.rflags & 1, 0); // CF
        assert_ne!(cpu.rflags & (1 << 11), 0); // OF
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF
    }

    #[test]
    fn grp2_shl_mem8_and_flags() {
        let mut mem = vec![0u8; 0x10000];
        // D0 26 00 30 = SHL byte [0x3000], 1
        mem[0] = 0xD0;
        mem[1] = 0x26;
        mem[2] = 0x00;
        mem[3] = 0x30;
        mem[4] = 0xF4;
        mem[0x3000] = 0x01;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x3000).unwrap(), 0x02);
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF clear
    }

    #[test]
    fn grp2_reserved_slash6_unsupported() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xD0;
        mem[1] = 0xF0; // /6 AL
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };
        assert_eq!(step(&mut cpu, &mut bus), Err(ExecError::Unsupported(0xD0)));
    }

    #[test]
    fn grp2_rol_does_not_touch_zf() {
        // Rotates leave SF/ZF/AF/PF unchanged (SDM Vol. 2 ROL — Flags Affected).
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xD0;
        mem[1] = 0xC0; // ROL AL,1
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_al(0x01);
        cpu.set_zf(true);
        cpu.set_sf(true);
        cpu.set_pf(false);
        let zf_sf_pf = cpu.rflags & ((1 << 6) | (1 << 7) | (1 << 2));
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x02);
        assert_eq!(cpu.rflags & ((1 << 6) | (1 << 7) | (1 << 2)), zf_sf_pf);
    }

    /// Group 2 C0/C1 imm8 count (masked to 5 bits). Spec: SDM Vol. 2.
    #[test]
    fn grp2_imm8_shl_shr_count0() {
        let mut mem = vec![0u8; 0x10000];
        // C0 E0 03 = SHL AL, 3; C1 E8 04 = SHR AX, 4; C0 E0 00 = SHL AL, 0 (no-op)
        mem[0] = 0xC0;
        mem[1] = 0xE0;
        mem[2] = 0x03;
        mem[3] = 0xC1;
        mem[4] = 0xE8;
        mem[5] = 0x04;
        mem[6] = 0xC0;
        mem[7] = 0xE0;
        mem[8] = 0x00;
        mem[9] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_al(0x01);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap(); // SHL AL,3 → 0x08
        assert_eq!(cpu.al(), 0x08);
        assert_eq!(cpu.rflags & 1, 0);

        cpu.set_ax(0x8000);
        step(&mut cpu, &mut bus).unwrap(); // SHR AX,4 → 0x0800
        assert_eq!(cpu.ax(), 0x0800);

        let flags_before = cpu.rflags;
        cpu.set_al(0x55);
        step(&mut cpu, &mut bus).unwrap(); // SHL AL,0 — unchanged
        assert_eq!(cpu.al(), 0x55);
        assert_eq!(cpu.rflags, flags_before);
    }

    #[test]
    fn grp2_imm8_count_masked_to_5_bits() {
        // COUNT & 0x1F: imm=0x21 → count 1 (SDM Vol. 2).
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xC0;
        mem[1] = 0xE0;
        mem[2] = 0x21; // SHL AL, 0x21 → effective 1
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_al(0x40);
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x80);
    }

    /// CBW/CWD sign-extend AL→AX and AX→DX:AX (SDM Vol. 2).
    #[test]
    fn cbw_cwd_sign_extend() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x98; // CBW
        mem[1] = 0x99; // CWD
        mem[2] = 0x98;
        mem[3] = 0x99;
        mem[4] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_ax(0x0080); // AL negative as i8
        cpu.set_gpr_u16(CpuState::RDX, 0x1234);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap(); // CBW → AX=0xFF80
        assert_eq!(cpu.ax(), 0xFF80);
        step(&mut cpu, &mut bus).unwrap(); // CWD → DX=0xFFFF
        assert_eq!(cpu.gpr_u16(CpuState::RDX), 0xFFFF);

        cpu.set_ax(0x007F);
        step(&mut cpu, &mut bus).unwrap(); // CBW → 0x007F
        assert_eq!(cpu.ax(), 0x007F);
        step(&mut cpu, &mut bus).unwrap(); // CWD → DX=0
        assert_eq!(cpu.gpr_u16(CpuState::RDX), 0);
    }

    /// LEA loads 16-bit EA offset into reg (SDM Vol. 2 LEA).
    #[test]
    fn lea_disp16_and_bx_si() {
        let mut mem = vec![0u8; 0x10000];
        // 8D 06 34 12 = LEA AX, [0x1234]
        // 8D 18 = LEA BX, [BX+SI]  (mod=00 rm=000)
        mem[0] = 0x8D;
        mem[1] = 0x06;
        mem[2] = 0x34;
        mem[3] = 0x12;
        mem[4] = 0x8D;
        mem[5] = 0x18;
        mem[6] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0x9999); // must not affect LEA
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RBX, 0x0100);
        cpu.set_gpr_u16(CpuState::RSI, 0x0020);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x1234);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0x0120);
    }

    #[test]
    fn lea_register_source_unsupported() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x8D;
        mem[1] = 0xC0; // LEA AX, AX — mod=11 → #UD
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };
        assert_eq!(step(&mut cpu, &mut bus), Err(ExecError::Unsupported(0x8D)));
    }

    /// Group 3 NOT/NEG (F6/F7 /2 /3). Spec: SDM Vol. 2 NOT/NEG.
    #[test]
    fn grp3_not_neg() {
        let mut mem = vec![0u8; 0x10000];
        // F6 D0 = NOT AL; F6 D8 = NEG AL; F7 D0 = NOT AX; F7 D8 = NEG AX
        mem[0] = 0xF6;
        mem[1] = 0xD0;
        mem[2] = 0xF6;
        mem[3] = 0xD8;
        mem[4] = 0xF7;
        mem[5] = 0xD0;
        mem[6] = 0xF7;
        mem[7] = 0xD8;
        mem[8] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_al(0x0F);
        cpu.set_zf(true);
        let flags_before_not = cpu.rflags;
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap(); // NOT AL
        assert_eq!(cpu.al(), 0xF0);
        assert_eq!(cpu.rflags, flags_before_not); // NOT: flags unaffected

        cpu.set_al(0x01);
        step(&mut cpu, &mut bus).unwrap(); // NEG AL → 0xFF, CF=1, SF=1
        assert_eq!(cpu.al(), 0xFF);
        assert_ne!(cpu.rflags & 1, 0);
        assert_ne!(cpu.rflags & (1 << 7), 0);
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF clear

        cpu.set_ax(0x00FF);
        let flags_before = cpu.rflags;
        step(&mut cpu, &mut bus).unwrap(); // NOT AX
        assert_eq!(cpu.ax(), 0xFF00);
        assert_eq!(cpu.rflags, flags_before);

        cpu.set_ax(0);
        step(&mut cpu, &mut bus).unwrap(); // NEG AX 0 → 0, CF=0, ZF=1
        assert_eq!(cpu.ax(), 0);
        assert_eq!(cpu.rflags & 1, 0);
        assert_ne!(cpu.rflags & (1 << 6), 0);
    }

    #[test]
    fn grp3_neg_mem8_and_unsupported_test() {
        let mut mem = vec![0u8; 0x10000];
        // F6 1E 00 40 = NEG byte [0x4000]; F6 C0 = TEST AL,imm — unsupported (/0)
        mem[0] = 0xF6;
        mem[1] = 0x1E;
        mem[2] = 0x00;
        mem[3] = 0x40;
        mem[4] = 0xF6;
        mem[5] = 0xC0;
        mem[0x4000] = 0x10;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x4000).unwrap(), 0xF0); // −0x10
        assert_eq!(step(&mut cpu, &mut bus), Err(ExecError::Unsupported(0xF6)));
    }

    /// Group 2 D2/D3 count = CL (SDM Vol. 2).
    #[test]
    fn grp2_cl_shl_sar() {
        let mut mem = vec![0u8; 0x10000];
        // D2 E0 = SHL AL, CL; D3 F8 = SAR AX, CL
        mem[0] = 0xD2;
        mem[1] = 0xE0;
        mem[2] = 0xD3;
        mem[3] = 0xF8;
        mem[4] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_al(0x01);
        cpu.set_gpr_u8_low(CpuState::RCX, 3);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x08);

        cpu.set_ax(0x8000);
        cpu.set_gpr_u8_low(CpuState::RCX, 4);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0xF800); // SAR sign-extends
        assert_eq!(cpu.rflags & 1, 0); // last shifted bit was 0
    }
}
