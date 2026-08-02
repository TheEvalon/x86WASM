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

fn set_add_flags_u8(cpu: &mut CpuState, a: u8, b: u8, result: u8) {
    cpu.set_cf((u16::from(a) + u16::from(b)) > 0xFF);
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x80 != 0);
    cpu.set_pf(parity_even(result));
    cpu.set_af(((a ^ b ^ result) & 0x10) != 0);
    let of = (!(a ^ b) & (a ^ result) & 0x80) != 0;
    cpu.set_of(of);
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

fn set_adc_flags_u8(cpu: &mut CpuState, a: u8, b: u8, cf_in: bool, result: u8) {
    let cf = u8::from(cf_in);
    let sum = u16::from(a) + u16::from(b) + u16::from(cf);
    cpu.set_cf(sum > 0xFF);
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x80 != 0);
    cpu.set_pf(parity_even(result));
    cpu.set_af((u16::from(a & 0xF) + u16::from(b & 0xF) + u16::from(cf)) > 0xF);
    let of = (!(a ^ b) & (a ^ result) & 0x80) != 0;
    cpu.set_of(of);
}

fn set_adc_flags_u16(cpu: &mut CpuState, a: u16, b: u16, cf_in: bool, result: u16) {
    let cf = u16::from(cf_in);
    let sum = u32::from(a) + u32::from(b) + u32::from(cf);
    cpu.set_cf(sum > 0xFFFF);
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x8000 != 0);
    cpu.set_pf(parity_even(result as u8));
    cpu.set_af(((a & 0xF) + (b & 0xF) + cf) > 0xF);
    let of = (!(a ^ b) & (a ^ result) & 0x8000) != 0;
    cpu.set_of(of);
}

fn set_sbb_flags_u8(cpu: &mut CpuState, a: u8, b: u8, cf_in: bool, result: u8) {
    let cf = u8::from(cf_in);
    cpu.set_cf(u16::from(a) < u16::from(b) + u16::from(cf));
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x80 != 0);
    cpu.set_pf(parity_even(result));
    cpu.set_af((a & 0xF) < ((b & 0xF) + cf));
    let of = ((a ^ b) & (a ^ result) & 0x80) != 0;
    cpu.set_of(of);
}

fn set_sbb_flags_u16(cpu: &mut CpuState, a: u16, b: u16, cf_in: bool, result: u16) {
    let cf = u16::from(cf_in);
    cpu.set_cf(u32::from(a) < u32::from(b) + u32::from(cf));
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x8000 != 0);
    cpu.set_pf(parity_even(result as u8));
    cpu.set_af((a & 0xF) < ((b & 0xF) + cf));
    let of = ((a ^ b) & (a ^ result) & 0x8000) != 0;
    cpu.set_of(of);
}

/// Group 1 ALU on 8-bit operands. Spec: Intel SDM Vol. 2 opcode map (80 /r).
/// Returns `Some(result)` to write back, or `None` for CMP.
fn grp1_u8(cpu: &mut CpuState, op: u8, a: u8, b: u8) -> Result<Option<u8>, ExecError> {
    let cf_in = cpu.rflags & 1 != 0;
    match op {
        0 => {
            let r = a.wrapping_add(b);
            set_add_flags_u8(cpu, a, b, r);
            Ok(Some(r))
        }
        1 => {
            let r = a | b;
            set_logic_flags_u8(cpu, r);
            Ok(Some(r))
        }
        2 => {
            let r = a.wrapping_add(b).wrapping_add(u8::from(cf_in));
            set_adc_flags_u8(cpu, a, b, cf_in, r);
            Ok(Some(r))
        }
        3 => {
            let r = a.wrapping_sub(b).wrapping_sub(u8::from(cf_in));
            set_sbb_flags_u8(cpu, a, b, cf_in, r);
            Ok(Some(r))
        }
        4 => {
            let r = a & b;
            set_logic_flags_u8(cpu, r);
            Ok(Some(r))
        }
        5 => {
            let r = a.wrapping_sub(b);
            set_sub_flags_u8(cpu, a, b, r);
            Ok(Some(r))
        }
        6 => {
            let r = a ^ b;
            set_logic_flags_u8(cpu, r);
            Ok(Some(r))
        }
        7 => {
            set_sub_flags_u8(cpu, a, b, a.wrapping_sub(b));
            Ok(None)
        }
        _ => Err(ExecError::Unsupported(0x80)),
    }
}

/// Group 1 ALU on 16-bit operands. Spec: Intel SDM Vol. 2 opcode map (81/83 /r).
fn grp1_u16(cpu: &mut CpuState, op: u8, a: u16, b: u16) -> Result<Option<u16>, ExecError> {
    let cf_in = cpu.rflags & 1 != 0;
    match op {
        0 => {
            let r = a.wrapping_add(b);
            set_add_flags_u16(cpu, a, b, r);
            Ok(Some(r))
        }
        1 => {
            let r = a | b;
            set_logic_flags_u16(cpu, r);
            Ok(Some(r))
        }
        2 => {
            let r = a.wrapping_add(b).wrapping_add(u16::from(cf_in));
            set_adc_flags_u16(cpu, a, b, cf_in, r);
            Ok(Some(r))
        }
        3 => {
            let r = a.wrapping_sub(b).wrapping_sub(u16::from(cf_in));
            set_sbb_flags_u16(cpu, a, b, cf_in, r);
            Ok(Some(r))
        }
        4 => {
            let r = a & b;
            set_logic_flags_u16(cpu, r);
            Ok(Some(r))
        }
        5 => {
            let r = a.wrapping_sub(b);
            set_sub_flags_u16(cpu, a, b, r);
            Ok(Some(r))
        }
        6 => {
            let r = a ^ b;
            set_logic_flags_u16(cpu, r);
            Ok(Some(r))
        }
        7 => {
            set_sub_flags_u16(cpu, a, b, a.wrapping_sub(b));
            Ok(None)
        }
        _ => Err(ExecError::Unsupported(0x81)),
    }
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

/// ModR/M.reg / opcode B0-B7 legacy byte register (AL..BH).
#[inline]
fn read_reg_u8(cpu: &CpuState, reg: u8) -> u8 {
    cpu.gpr_u8(reg as usize)
}

/// Write ModR/M.reg / opcode B0-B7 legacy byte register (AL..BH).
#[inline]
fn write_reg_u8(cpu: &mut CpuState, reg: u8, val: u8) {
    cpu.set_gpr_u8(reg as usize, val);
}

fn read_rm_u8(cpu: &CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<u8, ExecError> {
    let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
    if m.mod_ == 3 {
        // Legacy byte r/m: 0-3 AL/CL/DL/BL, 4-7 AH/CH/DH/BH (SDM Vol. 2 App. B).
        Ok(read_reg_u8(cpu, m.rm))
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
        write_reg_u8(cpu, m.rm, val);
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
        0x91..=0x97 => {
            // XCHG AX, r16 — Spec: Intel SDM Vol. 2 "XCHG" (opcode 90+rw; 90 is NOP).
            // Unsupported here: opsize 32 (XCHG EAX,r32); REX.W (XCHG RAX,r64).
            let idx = (op - 0x90) as usize;
            let ax = cpu.ax();
            let other = cpu.gpr_u16(idx);
            cpu.set_ax(other);
            cpu.set_gpr_u16(idx, ax);
            cpu.set_ip16(next_ip);
        }
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
        0xE0..=0xE2 => {
            // LOOPNE/LOOPE/LOOP rel8 — Spec: Intel SDM Vol. 2 "LOOP/LOOPcc".
            // Address-size 16: count register is CX. Unsupported: asize 32/64 (ECX/RCX).
            let cx = cpu.gpr_u16(CpuState::RCX).wrapping_sub(1);
            cpu.set_gpr_u16(CpuState::RCX, cx);
            let zf = cpu.rflags & (1 << 6) != 0;
            let take = match op {
                0xE0 => cx != 0 && !zf, // LOOPNE / LOOPNZ
                0xE1 => cx != 0 && zf,  // LOOPE / LOOPZ
                0xE2 => cx != 0,        // LOOP
                _ => unreachable!("matched 0xE0..=0xE2"),
            };
            if take {
                cpu.set_ip16(next_ip.wrapping_add(insn.immediate as i16 as u16));
            } else {
                cpu.set_ip16(next_ip);
            }
        }
        0xE3 => {
            // JCXZ rel8 — Spec: Intel SDM Vol. 2 "JCXZ/JECXZ/JRCXZ".
            // Address-size 16: test CX == 0. Unsupported: JECXZ/JRCXZ (ascale 32/64).
            if cpu.gpr_u16(CpuState::RCX) == 0 {
                cpu.set_ip16(next_ip.wrapping_add(insn.immediate as i16 as u16));
            } else {
                cpu.set_ip16(next_ip);
            }
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
        0xC2 => {
            // RET iw — near return with stack release.
            // Spec: Intel SDM Vol. 2 "RET" (near, imm16).
            // Unsupported here: opsize 32.
            let ip = pop16(cpu, bus)?;
            let release = insn.immediate as u16;
            let sp = cpu.gpr_u16(CpuState::RSP).wrapping_add(release);
            cpu.set_gpr_u16(CpuState::RSP, sp);
            cpu.set_ip16(ip);
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
        0xCA => {
            // RETF iw — far return with stack release.
            // Spec: Intel SDM Vol. 2 "RET" (far, imm16).
            // Unsupported here: opsize 32; protected-mode privilege checks.
            let ip = pop16(cpu, bus)?;
            let cs_sel = pop16(cpu, bus)?;
            let release = insn.immediate as u16;
            let sp = cpu.gpr_u16(CpuState::RSP).wrapping_add(release);
            cpu.set_gpr_u16(CpuState::RSP, sp);
            cpu.cs = x86_core::SegmentReg::real_mode_code(cs_sel);
            cpu.set_ip16(ip);
        }
        0xCB => {
            // RETF — far return, 16-bit stack frame (pop IP then CS).
            // Spec: Intel SDM Vol. 2 "RET" (far).
            // Unsupported here: opsize 32.
            let ip = pop16(cpu, bus)?;
            let cs_sel = pop16(cpu, bus)?;
            cpu.cs = x86_core::SegmentReg::real_mode_code(cs_sel);
            cpu.set_ip16(ip);
        }
        0xC8 => {
            // ENTER iw, ib — 16-bit opsize, nesting level 0 only this slice.
            // Spec: Intel SDM Vol. 2 "ENTER".
            // Unsupported here: nesting level > 0 (imm8 & 0x1F != 0); opsize 32 (ENTERD).
            let alloc = insn.immediate as u16;
            let nesting = (insn.displacement as u8) & 0x1F;
            if nesting != 0 {
                return Err(ExecError::Unsupported(0xC8));
            }
            push16(cpu, bus, cpu.gpr_u16(CpuState::RBP))?;
            let frame_temp = cpu.gpr_u16(CpuState::RSP);
            cpu.set_gpr_u16(CpuState::RBP, frame_temp);
            let sp = frame_temp.wrapping_sub(alloc);
            cpu.set_gpr_u16(CpuState::RSP, sp);
            cpu.set_ip16(next_ip);
        }
        0xC9 => {
            // LEAVE — Spec: Intel SDM Vol. 2 "LEAVE".
            // Unsupported here: opsize 32.
            let bp = cpu.gpr_u16(CpuState::RBP);
            cpu.set_gpr_u16(CpuState::RSP, bp);
            let v = pop16(cpu, bus)?;
            cpu.set_gpr_u16(CpuState::RBP, v);
            cpu.set_ip16(next_ip);
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
        0x9E => {
            // SAHF — load SF,ZF,AF,PF,CF from AH. Spec: Intel SDM Vol. 2 "SAHF".
            // Unsupported here: none for real-mode 16-bit; OF unaffected.
            let ah = cpu.ah();
            cpu.set_cf(ah & 1 != 0);
            cpu.set_pf(ah & (1 << 2) != 0);
            cpu.set_af(ah & (1 << 4) != 0);
            cpu.set_zf(ah & (1 << 6) != 0);
            cpu.set_sf(ah & (1 << 7) != 0);
            cpu.set_ip16(next_ip);
        }
        0x9F => {
            // LAHF — AH = SF:ZF:0:AF:0:PF:1:CF. Spec: Intel SDM Vol. 2 "LAHF".
            let mut ah = 1u8 << 1; // reserved bit 1 always set in the transferred image
            if cpu.rflags & 1 != 0 {
                ah |= 1;
            }
            if cpu.rflags & (1 << 2) != 0 {
                ah |= 1 << 2;
            }
            if cpu.rflags & (1 << 4) != 0 {
                ah |= 1 << 4;
            }
            if cpu.rflags & (1 << 6) != 0 {
                ah |= 1 << 6;
            }
            if cpu.rflags & (1 << 7) != 0 {
                ah |= 1 << 7;
            }
            cpu.set_ah(ah);
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
            // Unsupported here: /6 reserved.
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
        0x80 => {
            // Group 1 r/m8, imm8 — Spec: Intel SDM Vol. 2 opcode map / ADD…CMP.
            // Unsupported here: opcode 82 alias; LOCK.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_rm_u8(cpu, bus, &insn)?;
            let b = insn.immediate as u8;
            if let Some(r) = grp1_u8(cpu, m.reg, a, b)? {
                write_rm_u8(cpu, bus, &insn, r)?;
            }
            cpu.set_ip16(next_ip);
        }
        0x81 => {
            // Group 1 r/m16, imm16 — Spec: Intel SDM Vol. 2.
            // Unsupported here: opsize 32 (imm32); LOCK.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_rm_u16(cpu, bus, &insn)?;
            let b = insn.immediate as u16;
            if let Some(r) = grp1_u16(cpu, m.reg, a, b)? {
                write_rm_u16(cpu, bus, &insn, r)?;
            }
            cpu.set_ip16(next_ip);
        }
        0x83 => {
            // Group 1 r/m16, imm8 (sign-extended) — Spec: Intel SDM Vol. 2.
            // Unsupported here: opsize 32; LOCK.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_rm_u16(cpu, bus, &insn)?;
            let b = insn.immediate as i8 as i16 as u16;
            if let Some(r) = grp1_u16(cpu, m.reg, a, b)? {
                write_rm_u16(cpu, bus, &insn, r)?;
            }
            cpu.set_ip16(next_ip);
        }
        0xC0 => {
            // Group 2 r/m8, imm8 — Spec: Intel SDM Vol. 2 (COUNT masked to 5 bits).
            // Unsupported here: /6 reserved.
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
            // Unsupported here: /6 reserved.
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
            // Group 3 r/m8 — TEST/NOT/NEG/MUL/IMUL/DIV/IDIV (/0–/7).
            // Spec: Intel SDM Vol. 2 "TEST"/"NOT"/"NEG"/"MUL"/"IMUL"/"DIV"/"IDIV"; opcode map Group 3.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let v = read_rm_u8(cpu, bus, &insn)?;
            match m.reg {
                0 | 1 => {
                    // TEST r/m8, imm8 — AND; result discarded. Flags like AND.
                    set_logic_flags_u8(cpu, v & (insn.immediate as u8));
                }
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
                4 => {
                    // MUL r/m8 — AX = AL * r/m8. CF=OF=1 iff AH != 0; SF/ZF/AF/PF undefined.
                    let prod = u16::from(cpu.al()).wrapping_mul(u16::from(v));
                    cpu.set_ax(prod);
                    let hi_nz = (prod >> 8) != 0;
                    cpu.set_cf(hi_nz);
                    cpu.set_of(hi_nz);
                }
                5 => {
                    // IMUL r/m8 — AX = AL * r/m8 (signed). CF=OF=1 iff result not in AL.
                    let prod = i16::from(cpu.al() as i8).wrapping_mul(i16::from(v as i8));
                    cpu.set_ax(prod as u16);
                    let fits = prod == i16::from(prod as i8);
                    cpu.set_cf(!fits);
                    cpu.set_of(!fits);
                }
                6 => {
                    // DIV r/m8 — AX / r/m8 → AL=quot, AH=rem. #DE if divisor=0 or quot>0xFF.
                    // Spec: Intel SDM Vol. 2 "DIV"; Vol. 3 §6.15 (#DE). Faulting IP = insn start.
                    if v == 0 {
                        return real_mode_software_interrupt(cpu, bus, 0, cpu.ip16());
                    }
                    let dividend = u32::from(cpu.ax());
                    let quot = dividend / u32::from(v);
                    let rem = dividend % u32::from(v);
                    if quot > 0xFF {
                        return real_mode_software_interrupt(cpu, bus, 0, cpu.ip16());
                    }
                    cpu.set_ax(((rem as u16) << 8) | (quot as u16));
                }
                7 => {
                    // IDIV r/m8 — signed AX / r/m8 → AL=quot, AH=rem. #DE on 0 or quot∉i8.
                    if v == 0 {
                        return real_mode_software_interrupt(cpu, bus, 0, cpu.ip16());
                    }
                    let dividend = cpu.ax() as i16;
                    let divisor = i16::from(v as i8);
                    let Some(quot) = dividend.checked_div(divisor) else {
                        return real_mode_software_interrupt(cpu, bus, 0, cpu.ip16());
                    };
                    if !(i16::from(i8::MIN)..=i16::from(i8::MAX)).contains(&quot) {
                        return real_mode_software_interrupt(cpu, bus, 0, cpu.ip16());
                    }
                    // Safe: checked_div already rejected i16::MIN / -1.
                    let rem = dividend.wrapping_rem(divisor);
                    cpu.set_ax(((rem as u16) << 8) | ((quot as u8) as u16));
                }
                _ => return Err(ExecError::Unsupported(op)),
            }
            cpu.set_ip16(next_ip);
        }
        0xF7 => {
            // Group 3 r/m16 — TEST/NOT/NEG/MUL/IMUL/DIV/IDIV (/0–/7).
            // Spec: Intel SDM Vol. 2 "TEST"/"NOT"/"NEG"/"MUL"/"IMUL"/"DIV"/"IDIV"; opcode map Group 3.
            // Unsupported here: opsize 32.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let v = read_rm_u16(cpu, bus, &insn)?;
            match m.reg {
                0 | 1 => {
                    // TEST r/m16, imm16 — AND; result discarded. Flags like AND.
                    set_logic_flags_u16(cpu, v & (insn.immediate as u16));
                }
                2 => {
                    write_rm_u16(cpu, bus, &insn, !v)?;
                }
                3 => {
                    let r = v.wrapping_neg();
                    write_rm_u16(cpu, bus, &insn, r)?;
                    set_sub_flags_u16(cpu, 0, v, r);
                }
                4 => {
                    // MUL r/m16 — DX:AX = AX * r/m16. CF=OF=1 iff DX != 0; SF/ZF/AF/PF undefined.
                    let prod = u32::from(cpu.ax()).wrapping_mul(u32::from(v));
                    cpu.set_ax(prod as u16);
                    cpu.set_gpr_u16(CpuState::RDX, (prod >> 16) as u16);
                    let hi_nz = (prod >> 16) != 0;
                    cpu.set_cf(hi_nz);
                    cpu.set_of(hi_nz);
                }
                5 => {
                    // IMUL r/m16 — DX:AX = AX * r/m16 (signed). CF=OF=1 iff result not in AX.
                    let prod = i32::from(cpu.ax() as i16).wrapping_mul(i32::from(v as i16));
                    cpu.set_ax(prod as u16);
                    cpu.set_gpr_u16(CpuState::RDX, (prod >> 16) as u16);
                    let fits = prod == i32::from(prod as i16);
                    cpu.set_cf(!fits);
                    cpu.set_of(!fits);
                }
                6 => {
                    // DIV r/m16 — DX:AX / r/m16 → AX=quot, DX=rem. #DE if divisor=0 or quot>0xFFFF.
                    if v == 0 {
                        return real_mode_software_interrupt(cpu, bus, 0, cpu.ip16());
                    }
                    let dividend =
                        (u32::from(cpu.gpr_u16(CpuState::RDX)) << 16) | u32::from(cpu.ax());
                    let quot = dividend / u32::from(v);
                    let rem = dividend % u32::from(v);
                    if quot > 0xFFFF {
                        return real_mode_software_interrupt(cpu, bus, 0, cpu.ip16());
                    }
                    cpu.set_ax(quot as u16);
                    cpu.set_gpr_u16(CpuState::RDX, rem as u16);
                }
                7 => {
                    // IDIV r/m16 — signed DX:AX / r/m16 → AX=quot, DX=rem. #DE on 0 or quot∉i16.
                    if v == 0 {
                        return real_mode_software_interrupt(cpu, bus, 0, cpu.ip16());
                    }
                    let dividend = ((u32::from(cpu.gpr_u16(CpuState::RDX)) << 16)
                        | u32::from(cpu.ax())) as i32;
                    let divisor = i32::from(v as i16);
                    let Some(quot) = dividend.checked_div(divisor) else {
                        return real_mode_software_interrupt(cpu, bus, 0, cpu.ip16());
                    };
                    if !(i32::from(i16::MIN)..=i32::from(i16::MAX)).contains(&quot) {
                        return real_mode_software_interrupt(cpu, bus, 0, cpu.ip16());
                    }
                    // Safe: checked_div already rejected i32::MIN / -1.
                    let rem = dividend.wrapping_rem(divisor);
                    cpu.set_ax(quot as u16);
                    cpu.set_gpr_u16(CpuState::RDX, rem as u16);
                }
                _ => return Err(ExecError::Unsupported(op)),
            }
            cpu.set_ip16(next_ip);
        }
        0xFE => {
            // Group 4 r/m8 — INC (/0) / DEC (/1). Spec: Intel SDM Vol. 2 "INC"/"DEC".
            // Unsupported here: /2–/7 (#UD); AH/CH/DH/BH high-byte rm.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let v = read_rm_u8(cpu, bus, &insn)?;
            let saved_cf = cpu.rflags & 1 != 0;
            match m.reg {
                0 => {
                    let r = v.wrapping_add(1);
                    write_rm_u8(cpu, bus, &insn, r)?;
                    set_add_flags_u8(cpu, v, 1, r);
                }
                1 => {
                    let r = v.wrapping_sub(1);
                    write_rm_u8(cpu, bus, &insn, r)?;
                    set_sub_flags_u8(cpu, v, 1, r);
                }
                _ => return Err(ExecError::Unsupported(op)),
            }
            // INC/DEC do not modify CF (Intel SDM Vol. 2, INC/DEC).
            cpu.set_cf(saved_cf);
            cpu.set_ip16(next_ip);
        }
        0xFF => {
            // Group 5 r/m16 — INC/DEC/CALL/JMP/PUSH.
            // Spec: Intel SDM Vol. 2 "INC"/"DEC"/"CALL"/"JMP"/"PUSH"; opcode map Group 5.
            // Unsupported here: /7 (#UD); opsize 32 (incl. far m16:32); protected-mode transfers.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            match m.reg {
                0 | 1 => {
                    let v = read_rm_u16(cpu, bus, &insn)?;
                    let saved_cf = cpu.rflags & 1 != 0;
                    if m.reg == 0 {
                        let r = v.wrapping_add(1);
                        write_rm_u16(cpu, bus, &insn, r)?;
                        set_add_flags_u16(cpu, v, 1, r);
                    } else {
                        let r = v.wrapping_sub(1);
                        write_rm_u16(cpu, bus, &insn, r)?;
                        set_sub_flags_u16(cpu, v, 1, r);
                    }
                    // INC/DEC do not modify CF (Intel SDM Vol. 2, INC/DEC).
                    cpu.set_cf(saved_cf);
                    cpu.set_ip16(next_ip);
                }
                2 => {
                    // CALL r/m16 near absolute indirect.
                    let target = read_rm_u16(cpu, bus, &insn)?;
                    push16(cpu, bus, next_ip)?;
                    cpu.set_ip16(target);
                }
                3 => {
                    // CALL FAR m16:16 — absolute indirect far (memory only).
                    // Spec: Intel SDM Vol. 2 "CALL" (m16:16); opcode map Group 5 /3.
                    // Register form is invalid (#UD). Unsupported: opsize 32 (m16:32); gates.
                    if m.mod_ == 3 {
                        return Err(ExecError::Unsupported(op));
                    }
                    let (addr, _) = ea_16(cpu, &insn)?;
                    let offset = bus.read_u16(addr)?;
                    let selector = bus.read_u16(addr.wrapping_add(2))?;
                    push16(cpu, bus, cpu.cs.selector)?;
                    push16(cpu, bus, next_ip)?;
                    cpu.cs = x86_core::SegmentReg::real_mode_code(selector);
                    cpu.set_ip16(offset);
                }
                4 => {
                    // JMP r/m16 near absolute indirect.
                    let target = read_rm_u16(cpu, bus, &insn)?;
                    cpu.set_ip16(target);
                }
                5 => {
                    // JMP FAR m16:16 — absolute indirect far (memory only).
                    // Spec: Intel SDM Vol. 2 "JMP" (m16:16); opcode map Group 5 /5.
                    // Register form is invalid (#UD). Unsupported: opsize 32 (m16:32); gates.
                    if m.mod_ == 3 {
                        return Err(ExecError::Unsupported(op));
                    }
                    let (addr, _) = ea_16(cpu, &insn)?;
                    let offset = bus.read_u16(addr)?;
                    let selector = bus.read_u16(addr.wrapping_add(2))?;
                    cpu.cs = x86_core::SegmentReg::real_mode_code(selector);
                    cpu.set_ip16(offset);
                }
                6 => {
                    // PUSH r/m16 — value is read before SP decrement (incl. PUSH SP).
                    let v = read_rm_u16(cpu, bus, &insn)?;
                    push16(cpu, bus, v)?;
                    cpu.set_ip16(next_ip);
                }
                _ => return Err(ExecError::Unsupported(op)),
            }
        }
        0x70..=0x7F => {
            // Jcc rel8 — Spec: Intel SDM Vol. 2 "Jcc".
            // Unsupported here: near rel16/rel32 forms (0F 8x).
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
        0x48..=0x4F => {
            // DEC r16 — Spec: Intel SDM Vol. 2 "DEC".
            // Unsupported here: opsize 32 (DEC r32).
            let idx = (op - 0x48) as usize;
            let old = cpu.gpr_u16(idx);
            let v = old.wrapping_sub(1);
            let saved_cf = cpu.rflags & 1 != 0;
            cpu.set_gpr_u16(idx, v);
            set_sub_flags_u16(cpu, old, 1, v);
            // DEC does not modify CF (Intel SDM Vol. 2, DEC).
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
        0x60 => {
            // PUSHA — push AX,CX,DX,BX, original SP,BP,SI,DI (16-bit).
            // Spec: Intel SDM Vol. 2 "PUSHA/PUSHAD".
            // Unsupported here: opsize 32 (PUSHAD).
            let sp0 = cpu.gpr_u16(CpuState::RSP);
            push16(cpu, bus, cpu.gpr_u16(CpuState::RAX))?;
            push16(cpu, bus, cpu.gpr_u16(CpuState::RCX))?;
            push16(cpu, bus, cpu.gpr_u16(CpuState::RDX))?;
            push16(cpu, bus, cpu.gpr_u16(CpuState::RBX))?;
            push16(cpu, bus, sp0)?;
            push16(cpu, bus, cpu.gpr_u16(CpuState::RBP))?;
            push16(cpu, bus, cpu.gpr_u16(CpuState::RSI))?;
            push16(cpu, bus, cpu.gpr_u16(CpuState::RDI))?;
            cpu.set_ip16(next_ip);
        }
        0x61 => {
            // POPA — pop DI,SI,BP, discard, BX,DX,CX,AX (16-bit).
            // Spec: Intel SDM Vol. 2 "POPA/POPAD".
            // Unsupported here: opsize 32 (POPAD).
            let di = pop16(cpu, bus)?;
            let si = pop16(cpu, bus)?;
            let bp = pop16(cpu, bus)?;
            let _discard_sp = pop16(cpu, bus)?;
            let bx = pop16(cpu, bus)?;
            let dx = pop16(cpu, bus)?;
            let cx = pop16(cpu, bus)?;
            let ax = pop16(cpu, bus)?;
            cpu.set_gpr_u16(CpuState::RDI, di);
            cpu.set_gpr_u16(CpuState::RSI, si);
            cpu.set_gpr_u16(CpuState::RBP, bp);
            cpu.set_gpr_u16(CpuState::RBX, bx);
            cpu.set_gpr_u16(CpuState::RDX, dx);
            cpu.set_gpr_u16(CpuState::RCX, cx);
            cpu.set_gpr_u16(CpuState::RAX, ax);
            cpu.set_ip16(next_ip);
        }
        0x8F => {
            // POP r/m16 — Group /0 only.
            // Spec: Intel SDM Vol. 2 "POP".
            // Unsupported here: 8F /1–/7 (#UD); opsize 32.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if m.reg != 0 {
                return Err(ExecError::Unsupported(0x8F));
            }
            let v = pop16(cpu, bus)?;
            write_rm_u16(cpu, bus, &insn, v)?;
            cpu.set_ip16(next_ip);
        }
        0x68 => {
            // PUSH imm16 — Spec: Intel SDM Vol. 2 "PUSH".
            // Unsupported here: opsize 32 (push imm32).
            push16(cpu, bus, insn.immediate as u16)?;
            cpu.set_ip16(next_ip);
        }
        0x6A => {
            // PUSH imm8 (sign-extended to opsize) — Spec: Intel SDM Vol. 2 "PUSH".
            // Unsupported here: opsize 32.
            let v = insn.immediate as i8 as i16 as u16;
            push16(cpu, bus, v)?;
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
        0xA0 => {
            // MOV AL, moffs8 — Spec: Intel SDM Vol. 2 "MOV".
            // Unsupported here: address-size 32/64.
            let off = insn.immediate as u16;
            let addr = linear_addr(data_seg_for_string_src(cpu, &insn), u64::from(off));
            let v = bus.read_u8(addr)?;
            cpu.set_al(v);
            cpu.set_ip16(next_ip);
        }
        0xA1 => {
            // MOV AX, moffs16 — Spec: Intel SDM Vol. 2 "MOV".
            // Unsupported here: opsize 32; address-size 32/64.
            let off = insn.immediate as u16;
            let addr = linear_addr(data_seg_for_string_src(cpu, &insn), u64::from(off));
            let v = bus.read_u16(addr)?;
            cpu.set_ax(v);
            cpu.set_ip16(next_ip);
        }
        0xA2 => {
            // MOV moffs8, AL — Spec: Intel SDM Vol. 2 "MOV".
            let off = insn.immediate as u16;
            let addr = linear_addr(data_seg_for_string_src(cpu, &insn), u64::from(off));
            bus.write_u8(addr, cpu.al())?;
            cpu.set_ip16(next_ip);
        }
        0xA3 => {
            // MOV moffs16, AX — Spec: Intel SDM Vol. 2 "MOV".
            // Unsupported here: opsize 32; address-size 32/64.
            let off = insn.immediate as u16;
            let addr = linear_addr(data_seg_for_string_src(cpu, &insn), u64::from(off));
            bus.write_u16(addr, cpu.ax())?;
            cpu.set_ip16(next_ip);
        }
        0xA8 => {
            // TEST AL, imm8 — Spec: Intel SDM Vol. 2 "TEST".
            // Flags: CF=OF=0; SF/ZF/PF from (AL & imm); AF undefined (cleared).
            set_logic_flags_u8(cpu, cpu.al() & insn.immediate as u8);
            cpu.set_ip16(next_ip);
        }
        0xA9 => {
            // TEST AX, imm16 — Spec: Intel SDM Vol. 2 "TEST".
            // Unsupported here: opsize 32 (imm32).
            set_logic_flags_u16(cpu, cpu.ax() & insn.immediate as u16);
            cpu.set_ip16(next_ip);
        }
        0xC6 => {
            // Group 11 MOV r/m8, imm8 — Spec: Intel SDM Vol. 2 "MOV" / opcode map.
            // Only /0 is defined; /1–/7 → Unsupported (not #UD delivery yet).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if m.reg != 0 {
                return Err(ExecError::Unsupported(op));
            }
            write_rm_u8(cpu, bus, &insn, insn.immediate as u8)?;
            cpu.set_ip16(next_ip);
        }
        0xC7 => {
            // Group 11 MOV r/m16, imm16 — Spec: Intel SDM Vol. 2 "MOV" / opcode map.
            // Unsupported here: opsize 32 (imm32); /1–/7 → Unsupported.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if m.reg != 0 {
                return Err(ExecError::Unsupported(op));
            }
            write_rm_u16(cpu, bus, &insn, insn.immediate as u16)?;
            cpu.set_ip16(next_ip);
        }
        0xB0..=0xB7 => {
            // MOV r8, imm8 - B0-B3 AL/CL/DL/BL; B4-B7 AH/CH/DH/BH (SDM Vol. 2 MOV).
            write_reg_u8(cpu, op - 0xB0, insn.immediate as u8);
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
            write_reg_u8(cpu, m.reg, v);
            cpu.set_ip16(next_ip);
        }
        0x88 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let v = read_reg_u8(cpu, m.reg);
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
        0x86 => {
            // XCHG r8, r/m8 — Spec: Intel SDM Vol. 2 "XCHG".
            // Flags unchanged. Unsupported here: LOCK bus-lock.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let rm = read_rm_u8(cpu, bus, &insn)?;
            let reg = read_reg_u8(cpu, m.reg);
            write_rm_u8(cpu, bus, &insn, reg)?;
            write_reg_u8(cpu, m.reg, rm);
            cpu.set_ip16(next_ip);
        }
        0x87 => {
            // XCHG r16, r/m16 — Spec: Intel SDM Vol. 2 "XCHG".
            // Flags unchanged. Unsupported here: opsize 32; LOCK bus-lock.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let rm = read_rm_u16(cpu, bus, &insn)?;
            let reg = cpu.gpr_u16(m.reg as usize);
            write_rm_u16(cpu, bus, &insn, reg)?;
            cpu.set_gpr_u16(m.reg as usize, rm);
            cpu.set_ip16(next_ip);
        }
        0x84 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_rm_u8(cpu, bus, &insn)?;
            let b = read_reg_u8(cpu, m.reg);
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
        // XOR ModRM — Spec: Intel SDM Vol. 2 "XOR".
        // Flags: CF=OF=0; SF/ZF/PF from result; AF undefined (cleared here).
        // Unsupported here: opsize 32; LOCK; AH/CH/DH/BH high-byte GPRs; segment-limit faults.
        0x30 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_rm_u8(cpu, bus, &insn)?;
            let b = read_reg_u8(cpu, m.reg);
            let r = a ^ b;
            write_rm_u8(cpu, bus, &insn, r)?;
            set_logic_flags_u8(cpu, r);
            cpu.set_ip16(next_ip);
        }
        0x32 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_reg_u8(cpu, m.reg);
            let b = read_rm_u8(cpu, bus, &insn)?;
            let r = a ^ b;
            write_reg_u8(cpu, m.reg, r);
            set_logic_flags_u8(cpu, r);
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
        // ADD/SUB ModRM — Spec: Intel SDM Vol. 2 "ADD" / "SUB".
        // Flags via set_add_flags_* / set_sub_flags_* (CF/OF/AF/ZF/SF/PF).
        // Unsupported here: opsize 32; LOCK; AH/CH/DH/BH high-byte GPRs; segment-limit faults.
        0x00 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_rm_u8(cpu, bus, &insn)?;
            let b = read_reg_u8(cpu, m.reg);
            let r = a.wrapping_add(b);
            write_rm_u8(cpu, bus, &insn, r)?;
            set_add_flags_u8(cpu, a, b, r);
            cpu.set_ip16(next_ip);
        }
        0x02 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_reg_u8(cpu, m.reg);
            let b = read_rm_u8(cpu, bus, &insn)?;
            let r = a.wrapping_add(b);
            write_reg_u8(cpu, m.reg, r);
            set_add_flags_u8(cpu, a, b, r);
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
        0x28 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_rm_u8(cpu, bus, &insn)?;
            let b = read_reg_u8(cpu, m.reg);
            let r = a.wrapping_sub(b);
            write_rm_u8(cpu, bus, &insn, r)?;
            set_sub_flags_u8(cpu, a, b, r);
            cpu.set_ip16(next_ip);
        }
        0x2A => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_reg_u8(cpu, m.reg);
            let b = read_rm_u8(cpu, bus, &insn)?;
            let r = a.wrapping_sub(b);
            write_reg_u8(cpu, m.reg, r);
            set_sub_flags_u8(cpu, a, b, r);
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
        // CMP ModRM — Spec: Intel SDM Vol. 2 "CMP".
        // Flags via set_sub_flags_* (same as SUB); operands unchanged.
        // Unsupported here: opsize 32; LOCK; AH/CH/DH/BH high-byte GPRs; segment-limit faults.
        0x38 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_rm_u8(cpu, bus, &insn)?;
            let b = read_reg_u8(cpu, m.reg);
            set_sub_flags_u8(cpu, a, b, a.wrapping_sub(b));
            cpu.set_ip16(next_ip);
        }
        0x3A => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_reg_u8(cpu, m.reg);
            let b = read_rm_u8(cpu, bus, &insn)?;
            set_sub_flags_u8(cpu, a, b, a.wrapping_sub(b));
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
        // ADD AX,imm16 — Spec: Intel SDM Vol. 2 "ADD" (accumulator form 05 iw).
        // Flags via set_add_flags_u16 (CF/OF/AF/ZF/SF/PF).
        // Unsupported here: opsize 32 (ADD EAX, imm32).
        0x05 => {
            let a = cpu.ax();
            let b = insn.immediate as u16;
            let r = a.wrapping_add(b);
            cpu.set_ax(r);
            set_add_flags_u16(cpu, a, b, r);
            cpu.set_ip16(next_ip);
        }
        // OR/AND AL/AX,imm — Spec: Intel SDM Vol. 2 "OR" / "AND" (accumulator forms).
        // Flags: CF=OF=0; SF/ZF/PF from result; AF undefined (cleared here).
        // Unsupported here: opsize 32 (imm32 into EAX).
        0x0C => {
            let r = cpu.al() | (insn.immediate as u8);
            cpu.set_al(r);
            set_logic_flags_u8(cpu, r);
            cpu.set_ip16(next_ip);
        }
        0x0D => {
            let r = cpu.ax() | (insn.immediate as u16);
            cpu.set_ax(r);
            set_logic_flags_u16(cpu, r);
            cpu.set_ip16(next_ip);
        }
        0x24 => {
            let r = cpu.al() & (insn.immediate as u8);
            cpu.set_al(r);
            set_logic_flags_u8(cpu, r);
            cpu.set_ip16(next_ip);
        }
        0x25 => {
            let r = cpu.ax() & (insn.immediate as u16);
            cpu.set_ax(r);
            set_logic_flags_u16(cpu, r);
            cpu.set_ip16(next_ip);
        }
        // ADC/SBB AL/AX,imm — Spec: Intel SDM Vol. 2 "ADC" / "SBB" (accumulator forms).
        // dest ← dest ± imm ± CF; flags via set_adc_flags_* / set_sbb_flags_*.
        // Unsupported here: opsize 32 (imm32 into EAX).
        0x14 => {
            let a = cpu.al();
            let b = insn.immediate as u8;
            let cf_in = cpu.rflags & 1 != 0;
            let r = a.wrapping_add(b).wrapping_add(u8::from(cf_in));
            cpu.set_al(r);
            set_adc_flags_u8(cpu, a, b, cf_in, r);
            cpu.set_ip16(next_ip);
        }
        0x15 => {
            let a = cpu.ax();
            let b = insn.immediate as u16;
            let cf_in = cpu.rflags & 1 != 0;
            let r = a.wrapping_add(b).wrapping_add(u16::from(cf_in));
            cpu.set_ax(r);
            set_adc_flags_u16(cpu, a, b, cf_in, r);
            cpu.set_ip16(next_ip);
        }
        0x1C => {
            let a = cpu.al();
            let b = insn.immediate as u8;
            let cf_in = cpu.rflags & 1 != 0;
            let r = a.wrapping_sub(b).wrapping_sub(u8::from(cf_in));
            cpu.set_al(r);
            set_sbb_flags_u8(cpu, a, b, cf_in, r);
            cpu.set_ip16(next_ip);
        }
        0x1D => {
            let a = cpu.ax();
            let b = insn.immediate as u16;
            let cf_in = cpu.rflags & 1 != 0;
            let r = a.wrapping_sub(b).wrapping_sub(u16::from(cf_in));
            cpu.set_ax(r);
            set_sbb_flags_u16(cpu, a, b, cf_in, r);
            cpu.set_ip16(next_ip);
        }
        // SUB/XOR/CMP AL/AX,imm — Spec: Intel SDM Vol. 2 accumulator forms.
        // SUB/XOR write AL/AX; CMP updates flags only (no dest write).
        // Unsupported here: opsize 32 (imm32 into EAX).
        0x2C => {
            let a = cpu.al();
            let b = insn.immediate as u8;
            let r = a.wrapping_sub(b);
            cpu.set_al(r);
            set_sub_flags_u8(cpu, a, b, r);
            cpu.set_ip16(next_ip);
        }
        0x2D => {
            let a = cpu.ax();
            let b = insn.immediate as u16;
            let r = a.wrapping_sub(b);
            cpu.set_ax(r);
            set_sub_flags_u16(cpu, a, b, r);
            cpu.set_ip16(next_ip);
        }
        0x34 => {
            let r = cpu.al() ^ (insn.immediate as u8);
            cpu.set_al(r);
            set_logic_flags_u8(cpu, r);
            cpu.set_ip16(next_ip);
        }
        0x35 => {
            let r = cpu.ax() ^ (insn.immediate as u16);
            cpu.set_ax(r);
            set_logic_flags_u16(cpu, r);
            cpu.set_ip16(next_ip);
        }
        0x3C => {
            let a = cpu.al();
            let b = insn.immediate as u8;
            set_sub_flags_u8(cpu, a, b, a.wrapping_sub(b));
            cpu.set_ip16(next_ip);
        }
        0x3D => {
            let a = cpu.ax();
            let b = insn.immediate as u16;
            set_sub_flags_u16(cpu, a, b, a.wrapping_sub(b));
            cpu.set_ip16(next_ip);
        }
        // ADC/SBB ModRM — Spec: Intel SDM Vol. 2 "ADC" / "SBB".
        // dest ← dest ± src ± CF; flags via set_adc_flags_* / set_sbb_flags_*.
        // Unsupported here: opsize 32; LOCK; segment-limit faults.
        0x10 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_rm_u8(cpu, bus, &insn)?;
            let b = read_reg_u8(cpu, m.reg);
            let cf_in = cpu.rflags & 1 != 0;
            let r = a.wrapping_add(b).wrapping_add(u8::from(cf_in));
            write_rm_u8(cpu, bus, &insn, r)?;
            set_adc_flags_u8(cpu, a, b, cf_in, r);
            cpu.set_ip16(next_ip);
        }
        0x11 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_rm_u16(cpu, bus, &insn)?;
            let b = cpu.gpr_u16(m.reg as usize);
            let cf_in = cpu.rflags & 1 != 0;
            let r = a.wrapping_add(b).wrapping_add(u16::from(cf_in));
            write_rm_u16(cpu, bus, &insn, r)?;
            set_adc_flags_u16(cpu, a, b, cf_in, r);
            cpu.set_ip16(next_ip);
        }
        0x12 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_reg_u8(cpu, m.reg);
            let b = read_rm_u8(cpu, bus, &insn)?;
            let cf_in = cpu.rflags & 1 != 0;
            let r = a.wrapping_add(b).wrapping_add(u8::from(cf_in));
            write_reg_u8(cpu, m.reg, r);
            set_adc_flags_u8(cpu, a, b, cf_in, r);
            cpu.set_ip16(next_ip);
        }
        0x13 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = cpu.gpr_u16(m.reg as usize);
            let b = read_rm_u16(cpu, bus, &insn)?;
            let cf_in = cpu.rflags & 1 != 0;
            let r = a.wrapping_add(b).wrapping_add(u16::from(cf_in));
            cpu.set_gpr_u16(m.reg as usize, r);
            set_adc_flags_u16(cpu, a, b, cf_in, r);
            cpu.set_ip16(next_ip);
        }
        0x18 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_rm_u8(cpu, bus, &insn)?;
            let b = read_reg_u8(cpu, m.reg);
            let cf_in = cpu.rflags & 1 != 0;
            let r = a.wrapping_sub(b).wrapping_sub(u8::from(cf_in));
            write_rm_u8(cpu, bus, &insn, r)?;
            set_sbb_flags_u8(cpu, a, b, cf_in, r);
            cpu.set_ip16(next_ip);
        }
        0x19 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_rm_u16(cpu, bus, &insn)?;
            let b = cpu.gpr_u16(m.reg as usize);
            let cf_in = cpu.rflags & 1 != 0;
            let r = a.wrapping_sub(b).wrapping_sub(u16::from(cf_in));
            write_rm_u16(cpu, bus, &insn, r)?;
            set_sbb_flags_u16(cpu, a, b, cf_in, r);
            cpu.set_ip16(next_ip);
        }
        0x1A => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_reg_u8(cpu, m.reg);
            let b = read_rm_u8(cpu, bus, &insn)?;
            let cf_in = cpu.rflags & 1 != 0;
            let r = a.wrapping_sub(b).wrapping_sub(u8::from(cf_in));
            write_reg_u8(cpu, m.reg, r);
            set_sbb_flags_u8(cpu, a, b, cf_in, r);
            cpu.set_ip16(next_ip);
        }
        0x1B => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = cpu.gpr_u16(m.reg as usize);
            let b = read_rm_u16(cpu, bus, &insn)?;
            let cf_in = cpu.rflags & 1 != 0;
            let r = a.wrapping_sub(b).wrapping_sub(u16::from(cf_in));
            cpu.set_gpr_u16(m.reg as usize, r);
            set_sbb_flags_u16(cpu, a, b, cf_in, r);
            cpu.set_ip16(next_ip);
        }
        // OR/AND ModRM — Spec: Intel SDM Vol. 2 "OR" / "AND".
        // Flags: CF=OF=0; SF/ZF/PF from result; AF undefined (cleared here).
        // Unsupported here: opsize 32; LOCK; segment-limit faults.
        0x08 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_rm_u8(cpu, bus, &insn)?;
            let b = read_reg_u8(cpu, m.reg);
            let r = a | b;
            write_rm_u8(cpu, bus, &insn, r)?;
            set_logic_flags_u8(cpu, r);
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
        0x0A => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_reg_u8(cpu, m.reg);
            let b = read_rm_u8(cpu, bus, &insn)?;
            let r = a | b;
            write_reg_u8(cpu, m.reg, r);
            set_logic_flags_u8(cpu, r);
            cpu.set_ip16(next_ip);
        }
        0x0B => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = cpu.gpr_u16(m.reg as usize);
            let b = read_rm_u16(cpu, bus, &insn)?;
            let r = a | b;
            cpu.set_gpr_u16(m.reg as usize, r);
            set_logic_flags_u16(cpu, r);
            cpu.set_ip16(next_ip);
        }
        0x20 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_rm_u8(cpu, bus, &insn)?;
            let b = read_reg_u8(cpu, m.reg);
            let r = a & b;
            write_rm_u8(cpu, bus, &insn, r)?;
            set_logic_flags_u8(cpu, r);
            cpu.set_ip16(next_ip);
        }
        0x21 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_rm_u16(cpu, bus, &insn)?;
            let b = cpu.gpr_u16(m.reg as usize);
            let r = a & b;
            write_rm_u16(cpu, bus, &insn, r)?;
            set_logic_flags_u16(cpu, r);
            cpu.set_ip16(next_ip);
        }
        0x22 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_reg_u8(cpu, m.reg);
            let b = read_rm_u8(cpu, bus, &insn)?;
            let r = a & b;
            write_reg_u8(cpu, m.reg, r);
            set_logic_flags_u8(cpu, r);
            cpu.set_ip16(next_ip);
        }
        0x23 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = cpu.gpr_u16(m.reg as usize);
            let b = read_rm_u16(cpu, bus, &insn)?;
            let r = a & b;
            cpu.set_gpr_u16(m.reg as usize, r);
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
    fn grp3_neg_mem8() {
        let mut mem = vec![0u8; 0x10000];
        // F6 1E 00 40 = NEG byte [0x4000]
        mem[0] = 0xF6;
        mem[1] = 0x1E;
        mem[2] = 0x00;
        mem[3] = 0x40;
        mem[0x4000] = 0x10;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x4000).unwrap(), 0xF0); // −0x10
    }

    /// Group 3 TEST/MUL (F6/F7 /0,/1,/4). Spec: SDM Vol. 2 TEST/MUL.
    #[test]
    fn grp3_test_mul_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 0: F6 C0 0F       TEST AL, 0x0F
        // 3: F6 C8 01       TEST AL, 1 (/1 alias)
        // 6: F7 C0 34 12    TEST AX, 0x1234
        // A: F6 E3          MUL BL
        // C: F7 E3          MUL BX
        // E: F6 06 00 40 FF TEST byte [0x4000], 0xFF
        // 13: F4            HLT
        mem[0] = 0xF6;
        mem[1] = 0xC0;
        mem[2] = 0x0F;
        mem[3] = 0xF6;
        mem[4] = 0xC8;
        mem[5] = 0x01;
        mem[6] = 0xF7;
        mem[7] = 0xC0;
        mem[8] = 0x34;
        mem[9] = 0x12;
        mem[0xA] = 0xF6;
        mem[0xB] = 0xE3;
        mem[0xC] = 0xF7;
        mem[0xD] = 0xE3;
        mem[0xE] = 0xF6;
        mem[0xF] = 0x06;
        mem[0x10] = 0x00;
        mem[0x11] = 0x40;
        mem[0x12] = 0xFF;
        mem[0x13] = 0xF4;
        mem[0x4000] = 0xF0;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_al(0xF0);
        cpu.set_cf(true);
        cpu.set_of(true);
        let mut bus = VecBus { mem, ports: vec![] };

        // TEST AL, 0x0F → 0xF0 & 0x0F = 0; ZF=1, CF=OF=0
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xF0); // unchanged
        assert_eq!(cpu.rflags & 1, 0); // CF
        assert_eq!(cpu.rflags & (1 << 11), 0); // OF
        assert_ne!(cpu.rflags & (1 << 6), 0); // ZF

        // TEST AL, 1 → 0xF0 & 1 = 0; ZF=1
        step(&mut cpu, &mut bus).unwrap();
        assert_ne!(cpu.rflags & (1 << 6), 0);

        // TEST AX, 0x1234 with AX=0 → 0; ZF=1
        cpu.set_ax(0);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0);
        assert_ne!(cpu.rflags & (1 << 6), 0);

        // MUL BL: AL=0x10, BL=0x10 → AX=0x0100; AH!=0 → CF=OF=1
        cpu.set_al(0x10);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x10);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0100);
        assert_ne!(cpu.rflags & 1, 0);
        assert_ne!(cpu.rflags & (1 << 11), 0);

        // MUL BX: AX=0x0002, BX=0x0003 → DX:AX=0:6; DX=0 → CF=OF=0
        cpu.set_ax(0x0002);
        cpu.set_gpr_u16(CpuState::RBX, 0x0003);
        cpu.set_gpr_u16(CpuState::RDX, 0xFFFF);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 6);
        assert_eq!(cpu.gpr_u16(CpuState::RDX), 0);
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 11), 0);

        // TEST byte [0x4000], 0xFF → 0xF0; SF=1, ZF=0
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x4000).unwrap(), 0xF0); // unchanged
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF
    }

    /// Group 3 IMUL/DIV/IDIV (F6/F7 /5–/7). Spec: SDM Vol. 2 IMUL/DIV/IDIV; Vol. 3 §6.15 (#DE).
    #[test]
    fn grp3_imul_div_idiv_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 0: F6 EB          IMUL BL
        // 2: F7 EB          IMUL BX
        // 4: F6 F3          DIV BL
        // 6: F7 F3          DIV BX
        // 8: F6 FB          IDIV BL
        // A: F7 FB          IDIV BX
        // C: F6 36 00 40    DIV byte [0x4000]
        // 10: F4            HLT
        mem[0] = 0xF6;
        mem[1] = 0xEB;
        mem[2] = 0xF7;
        mem[3] = 0xEB;
        mem[4] = 0xF6;
        mem[5] = 0xF3;
        mem[6] = 0xF7;
        mem[7] = 0xF3;
        mem[8] = 0xF6;
        mem[9] = 0xFB;
        mem[0xA] = 0xF7;
        mem[0xB] = 0xFB;
        mem[0xC] = 0xF6;
        mem[0xD] = 0x36;
        mem[0xE] = 0x00;
        mem[0xF] = 0x40;
        mem[0x10] = 0xF4;
        mem[0x4000] = 5;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };

        // IMUL BL: AL=-2 (0xFE), BL=-3 (0xFD) → AX=6; fits in AL → CF=OF=0
        cpu.set_al(0xFE);
        cpu.set_gpr_u8_low(CpuState::RBX, 0xFD);
        cpu.set_cf(true);
        cpu.set_of(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 6);
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 11), 0);

        // IMUL BX: AX=0x0100, BX=0x0100 → DX:AX=0x0001_0000; does not fit in AX → CF=OF=1
        cpu.set_ax(0x0100);
        cpu.set_gpr_u16(CpuState::RBX, 0x0100);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RDX), 1);
        assert_ne!(cpu.rflags & 1, 0);
        assert_ne!(cpu.rflags & (1 << 11), 0);

        // DIV BL: AX=0x0105 / BL=3 → AL=0x57, AH=0
        cpu.set_ax(0x0105);
        cpu.set_gpr_u8_low(CpuState::RBX, 3);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0057);

        // DIV BX: DX:AX=0:1000 / BX=7 → AX=142 (0x8E), DX=6
        cpu.set_ax(1000);
        cpu.set_gpr_u16(CpuState::RDX, 0);
        cpu.set_gpr_u16(CpuState::RBX, 7);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 142);
        assert_eq!(cpu.gpr_u16(CpuState::RDX), 6);

        // IDIV BL: AX=-25 (0xFFE7) / BL=7 → AL=-3 (0xFD), AH=-4 (0xFC)
        cpu.set_ax(0xFFE7);
        cpu.set_gpr_u8_low(CpuState::RBX, 7);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0xFCFD);

        // IDIV BX: DX:AX=-1000 / BX=7 → AX=-142, DX=-6
        // -1000 as i32 = 0xFFFF_FC18 → DX=0xFFFF, AX=0xFC18
        cpu.set_ax(0xFC18);
        cpu.set_gpr_u16(CpuState::RDX, 0xFFFF);
        cpu.set_gpr_u16(CpuState::RBX, 7);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax() as i16, -142);
        assert_eq!(cpu.gpr_u16(CpuState::RDX) as i16, -6);

        // DIV byte [0x4000]: AX=26 / 5 → AL=5, AH=1
        cpu.set_ax(26);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0105);
    }

    /// DIV/IDIV #DE (vector 0): divisor 0 or quotient overflow; fault IP = insn start.
    #[test]
    fn grp3_div_idiv_de_fault() {
        let mut mem = vec![0u8; 0x10000];
        // IVT[0] → handler at 0000:0900
        mem[0] = 0x00;
        mem[1] = 0x09;
        mem[2] = 0x00;
        mem[3] = 0x00;
        // Place code away from IVT: CS base 0x1000 (selector 0x0100), IP 0
        // linear 0x1000: F6 F3 = DIV BL (divisor 0)
        // linear 0x1002: F6 F3 = DIV BL (quot overflow)
        // linear 0x1004: F7 FB = IDIV BX (i32::MIN / -1)
        mem[0x1000] = 0xF6;
        mem[0x1001] = 0xF3;
        mem[0x1002] = 0xF6;
        mem[0x1003] = 0xF3;
        mem[0x1004] = 0xF7;
        mem[0x1005] = 0xFB;
        mem[0x900] = 0xF4; // handler HLT

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0x0100);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        let mut bus = VecBus { mem, ports: vec![] };

        // DIV BL with BL=0 → #DE; saved IP = 0 (faulting insn)
        cpu.set_ax(0x0100);
        cpu.set_gpr_u8_low(CpuState::RBX, 0);
        let ax_before = cpu.ax();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.cs.selector, 0);
        assert_eq!(cpu.ip16(), 0x0900);
        assert!(!cpu.interrupt_flag());
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0); // fault IP
        assert_eq!(bus.read_u16(0xFFFA).unwrap(), 0x0100); // CS
        assert_eq!(cpu.ax(), ax_before); // no partial update

        // Resume at overflow DIV: AX=0x0200 / BL=1 → quot 0x200 > 0xFF → #DE
        cpu.cs = x86_core::SegmentReg::real_mode_code(0x0100);
        cpu.rip = 2;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        cpu.set_ax(0x0200);
        cpu.set_gpr_u8_low(CpuState::RBX, 1);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0900);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 2);

        // IDIV BX: DX:AX = i32::MIN / -1 → #DE (quot overflow)
        cpu.cs = x86_core::SegmentReg::real_mode_code(0x0100);
        cpu.rip = 4;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        cpu.set_ax(0);
        cpu.set_gpr_u16(CpuState::RDX, 0x8000);
        cpu.set_gpr_u16(CpuState::RBX, 0xFFFF); // -1
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0900);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 4);
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

    /// XCHG r/m↔reg and XCHG AX,r16; flags unchanged (SDM Vol. 2 XCHG).
    #[test]
    fn xchg_reg_mem_and_ax_forms() {
        let mut mem = vec![0u8; 0x10000];
        // 86 C3 = XCHG AL, BL
        // 87 06 00 30 = XCHG AX, [0x3000]
        // 91 = XCHG AX, CX
        // 97 = XCHG AX, DI
        mem[0] = 0x86;
        mem[1] = 0xC3;
        mem[2] = 0x87;
        mem[3] = 0x06;
        mem[4] = 0x00;
        mem[5] = 0x30;
        mem[6] = 0x91;
        mem[7] = 0x97;
        mem[8] = 0xF4;
        mem[0x3000] = 0x34;
        mem[0x3001] = 0x12;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_al(0xAA);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x55);
        cpu.rflags = 0x246; // arbitrary non-zero flags; must be preserved
        let flags_before = cpu.rflags;
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x55);
        assert_eq!(cpu.gpr_u8_low(CpuState::RBX), 0xAA);
        assert_eq!(cpu.rflags, flags_before);

        cpu.set_ax(0xABCD);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x1234);
        assert_eq!(bus.read_u16(0x3000).unwrap(), 0xABCD);
        assert_eq!(cpu.rflags, flags_before);

        cpu.set_ax(0x1111);
        cpu.set_gpr_u16(CpuState::RCX, 0x2222);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x2222);
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0x1111);

        cpu.set_gpr_u16(CpuState::RDI, 0x3333);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x3333);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x2222);
        assert_eq!(cpu.rflags, flags_before);
    }

    #[test]
    fn xchg_reg16_reg16_modrm() {
        // 87 D8 = XCHG AX, BX (mod=11)
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x87;
        mem[1] = 0xD8;
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_ax(0x1000);
        cpu.set_gpr_u16(CpuState::RBX, 0x2000);
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x2000);
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0x1000);
    }

    /// PUSH imm16 / sign-extended imm8 (SDM Vol. 2 PUSH).
    #[test]
    fn push_imm16_and_imm8() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x68;
        mem[1] = 0x34;
        mem[2] = 0x12; // PUSH 0x1234
        mem[3] = 0x6A;
        mem[4] = 0xFE; // PUSH -2 → 0xFFFE
        mem[5] = 0x58; // POP AX
        mem[6] = 0x5B; // POP BX
        mem[7] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFC);
        assert_eq!(bus.read_u16(0xFFFC).unwrap(), 0x1234);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFA);
        assert_eq!(bus.read_u16(0xFFFA).unwrap(), 0xFFFE);

        step(&mut cpu, &mut bus).unwrap(); // POP AX ← 0xFFFE
        assert_eq!(cpu.ax(), 0xFFFE);
        step(&mut cpu, &mut bus).unwrap(); // POP BX ← 0x1234
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0x1234);
    }

    /// LAHF/SAHF transfer SF ZF AF PF CF via AH (SDM Vol. 2).
    #[test]
    fn lahf_sahf_round_trip() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x9F; // LAHF
        mem[1] = 0x9E; // SAHF
        mem[2] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        // SF ZF AF PF CF = 1 0 1 0 1 → AH pattern 1x0x0x0x with bit1=1 → 0b1001_0011 = 0x93
        cpu.set_sf(true);
        cpu.set_zf(false);
        cpu.set_af(true);
        cpu.set_pf(false);
        cpu.set_cf(true);
        cpu.set_of(true); // must survive SAHF
        cpu.set_ax(0x0000);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!((cpu.ax() >> 8) as u8, 0x93);

        // Clear status flags then restore via SAHF; OF stays set
        cpu.set_sf(false);
        cpu.set_zf(true);
        cpu.set_af(false);
        cpu.set_pf(true);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert!(cpu.rflags & (1 << 7) != 0); // SF
        assert!(cpu.rflags & (1 << 6) == 0); // ZF
        assert!(cpu.rflags & (1 << 4) != 0); // AF
        assert!(cpu.rflags & (1 << 2) == 0); // PF
        assert!(cpu.rflags & 1 != 0); // CF
        assert!(cpu.rflags & (1 << 11) != 0); // OF preserved
    }

    /// DEC r16: result/flags; CF preserved (SDM Vol. 2 DEC).
    #[test]
    fn dec_r16_preserves_cf() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x48; // DEC AX
        mem[1] = 0x4B; // DEC BX
        mem[2] = 0x4F; // DEC DI
        mem[3] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_ax(1);
        cpu.set_gpr_u16(CpuState::RBX, 0);
        cpu.set_gpr_u16(CpuState::RDI, 0x8000);
        cpu.set_cf(true);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0);
        assert_ne!(cpu.rflags & (1 << 6), 0); // ZF
        assert_ne!(cpu.rflags & 1, 0); // CF preserved

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0xFFFF);
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF
        assert_ne!(cpu.rflags & 1, 0); // CF still set

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x7FFF);
        assert_ne!(cpu.rflags & (1 << 11), 0); // OF: 0x8000-1
        assert_ne!(cpu.rflags & 1, 0);
    }

    /// Group 1 80/81/83 imm ALU — results and flags (SDM Vol. 2).
    #[test]
    fn grp1_imm_alu() {
        let mut mem = vec![0u8; 0x10000];
        // 80 C0 01 = ADD AL,1
        // 80 E0 0F = AND AL,0x0F
        // 80 F8 05 = CMP AL,5
        // 81 C3 00 10 = ADD BX,0x1000
        // 83 EB 01 = SUB BX,1 (imm8 sign-ext)
        // 83 D8 FF = SBB AX,-1 with CF
        mem[0] = 0x80;
        mem[1] = 0xC0;
        mem[2] = 0x01;
        mem[3] = 0x80;
        mem[4] = 0xE0;
        mem[5] = 0x0F;
        mem[6] = 0x80;
        mem[7] = 0xF8;
        mem[8] = 0x05;
        mem[9] = 0x81;
        mem[10] = 0xC3;
        mem[11] = 0x00;
        mem[12] = 0x10;
        mem[13] = 0x83;
        mem[14] = 0xEB;
        mem[15] = 0x01;
        mem[16] = 0x83;
        mem[17] = 0xD8;
        mem[18] = 0xFF;
        mem[19] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_al(0x10);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap(); // ADD AL,1
        assert_eq!(cpu.al(), 0x11);
        assert_eq!(cpu.rflags & 1, 0);

        step(&mut cpu, &mut bus).unwrap(); // AND AL,0x0F
        assert_eq!(cpu.al(), 0x01);
        assert_eq!(cpu.rflags & 1, 0); // logic clears CF
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF=0

        let al_before = cpu.al();
        step(&mut cpu, &mut bus).unwrap(); // CMP AL,5 → 1-5
        assert_eq!(cpu.al(), al_before); // CMP no write
        assert_ne!(cpu.rflags & 1, 0); // CF=1 (borrow)
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF=0

        cpu.set_gpr_u16(CpuState::RBX, 0x0200);
        step(&mut cpu, &mut bus).unwrap(); // ADD BX,0x1000
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0x1200);

        step(&mut cpu, &mut bus).unwrap(); // SUB BX,1
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0x11FF);

        cpu.set_ax(0x0001);
        cpu.set_cf(true);
        step(&mut cpu, &mut bus).unwrap(); // SBB AX, -1 (=0xFFFF): 1 - (-1) - 1 = 1
        assert_eq!(cpu.ax(), 0x0001);
    }

    #[test]
    fn grp1_adc_or_xor_mem() {
        // 80 06 00 40 7F = ADD byte [0x4000], 0x7F
        // 80 0E 00 40 01 = OR  byte [0x4000], 1
        // 80 36 00 40 FF = XOR byte [0x4000], 0xFF
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x80;
        mem[1] = 0x06;
        mem[2] = 0x00;
        mem[3] = 0x40;
        mem[4] = 0x7F;
        mem[5] = 0x80;
        mem[6] = 0x0E;
        mem[7] = 0x00;
        mem[8] = 0x40;
        mem[9] = 0x01;
        mem[10] = 0x80;
        mem[11] = 0x36;
        mem[12] = 0x00;
        mem[13] = 0x40;
        mem[14] = 0xFF;
        mem[0x4000] = 0x01;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x4000).unwrap(), 0x80);
        assert_ne!(cpu.rflags & (1 << 11), 0); // OF: 0x01+0x7F → 0x80

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x4000).unwrap(), 0x81);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x4000).unwrap(), 0x7E);
    }

    /// LOOP/LOOPcc decrement CX then branch; JCXZ tests CX (SDM Vol. 2).
    #[test]
    fn loop_loopcc_jcxz() {
        let mut mem = vec![0u8; 0x10000];
        // E2 FE = LOOP $-0 (rel8=-2) → branch back to self while CX≠0 after dec
        // After CX hits 0, fall through.
        mem[0] = 0xE2;
        mem[1] = 0xFE;
        // E0 02 = LOOPNE +2; E1 02 = LOOPE +2; padding HLTs
        mem[2] = 0xE0;
        mem[3] = 0x02;
        mem[4] = 0xF4; // skip target when not taken
        mem[5] = 0xF4;
        mem[6] = 0x90; // taken landing
        mem[7] = 0xE1;
        mem[8] = 0x02;
        mem[9] = 0xF4;
        mem[10] = 0xF4;
        mem[11] = 0x90;
        // E3 02 = JCXZ +2
        mem[12] = 0xE3;
        mem[13] = 0x02;
        mem[14] = 0xF4;
        mem[15] = 0xF4;
        mem[16] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RCX, 3);
        let mut bus = VecBus { mem, ports: vec![] };

        // LOOP three times: CX 3→2→1→0, then fall through to IP=2
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 2);
        assert_eq!(cpu.ip16(), 0);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 1);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.ip16(), 2);

        // LOOPNE: CX=2, ZF=0 → take; then CX=1, ZF=1 → no take
        cpu.set_gpr_u16(CpuState::RCX, 2);
        cpu.set_zf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 1);
        assert_eq!(cpu.ip16(), 6); // taken → next_ip(4)+2

        cpu.rip = 2;
        cpu.set_gpr_u16(CpuState::RCX, 2);
        cpu.set_zf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 1);
        assert_eq!(cpu.ip16(), 4); // not taken

        // LOOPE: ZF=1 and CX after dec ≠0 → take
        cpu.rip = 7;
        cpu.set_gpr_u16(CpuState::RCX, 1);
        cpu.set_zf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.ip16(), 9); // CX became 0 → not taken

        cpu.rip = 7;
        cpu.set_gpr_u16(CpuState::RCX, 2);
        cpu.set_zf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 11); // taken

        // JCXZ: CX==0 takes; CX!=0 falls through
        cpu.rip = 12;
        cpu.set_gpr_u16(CpuState::RCX, 0);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 16);
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0); // unchanged

        cpu.rip = 12;
        cpu.set_gpr_u16(CpuState::RCX, 1);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 14);
    }

    /// OR/AND ModRM 08–0B / 20–23 — results and logic flags (SDM Vol. 2 OR/AND).
    #[test]
    fn and_or_modrm_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 08 D8 = OR  AL, BL
        // 0A C3 = OR  AL, BL  (reg ← r/m; same regs after first)
        // 09 D8 = OR  AX, BX
        // 0B C3 = OR  AX, BX
        // 20 D8 = AND AL, BL
        // 22 C3 = AND AL, BL
        // 21 D8 = AND AX, BX
        // 23 C3 = AND AX, BX
        // 09 06 00 40 = OR  word [0x4000], AX
        // 23 06 00 40 = AND AX, word [0x4000]
        mem[0] = 0x08;
        mem[1] = 0xD8;
        mem[2] = 0x0A;
        mem[3] = 0xC3;
        mem[4] = 0x09;
        mem[5] = 0xD8;
        mem[6] = 0x0B;
        mem[7] = 0xC3;
        mem[8] = 0x20;
        mem[9] = 0xD8;
        mem[10] = 0x22;
        mem[11] = 0xC3;
        mem[12] = 0x21;
        mem[13] = 0xD8;
        mem[14] = 0x23;
        mem[15] = 0xC3;
        mem[16] = 0x09;
        mem[17] = 0x06;
        mem[18] = 0x00;
        mem[19] = 0x40;
        mem[20] = 0x23;
        mem[21] = 0x06;
        mem[22] = 0x00;
        mem[23] = 0x40;
        mem[24] = 0xF4;
        mem[0x4000] = 0x0F;
        mem[0x4001] = 0xF0;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_al(0xF0);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x0F);
        cpu.set_cf(true);
        cpu.set_of(true);
        let mut bus = VecBus { mem, ports: vec![] };

        // OR AL, BL (08): r/m ← r/m | reg → AL |= BL
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xFF);
        assert_eq!(cpu.rflags & 1, 0); // CF cleared
        assert_eq!(cpu.rflags & (1 << 11), 0); // OF cleared
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF

        // OR AL, BL (0A): reg ← reg | r/m
        cpu.set_al(0x10);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x01);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x11);

        // OR AX, BX (09): r/m ← r/m | reg
        cpu.set_ax(0xF000);
        cpu.set_gpr_u16(CpuState::RBX, 0x0FFF);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0xFFFF);

        // OR AX, BX (0B): reg ← reg | r/m
        cpu.set_ax(0x1000);
        cpu.set_gpr_u16(CpuState::RBX, 0x0200);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x1200);

        // AND AL, BL (20)
        cpu.set_al(0xF3);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x0F);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x03);
        assert_eq!(cpu.rflags & 1, 0);

        // AND AL, BL (22)
        cpu.set_al(0xAA);
        cpu.set_gpr_u8_low(CpuState::RBX, 0xF0);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xA0);

        // AND AX, BX (21)
        cpu.set_ax(0xFF00);
        cpu.set_gpr_u16(CpuState::RBX, 0x0FF0);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0F00);

        // AND AX, BX (23)
        cpu.set_ax(0x1234);
        cpu.set_gpr_u16(CpuState::RBX, 0x00FF);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0034);
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF=0

        // OR [0x4000], AX — mem destination
        cpu.set_ax(0x00F0);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x4000).unwrap(), 0xF0FF);

        // AND AX, [0x4000]
        cpu.set_ax(0xFFFF);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0xF0FF);
    }

    /// ADC/SBB ModRM 10–13 / 18–1B — results and flags with CF in (SDM Vol. 2 ADC/SBB).
    #[test]
    fn adc_sbb_modrm_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 10 D8 = ADC AL, BL
        // 12 C3 = ADC AL, BL  (reg ← r/m)
        // 11 D8 = ADC AX, BX
        // 13 C3 = ADC AX, BX
        // 18 D8 = SBB AL, BL
        // 1A C3 = SBB AL, BL
        // 19 D8 = SBB AX, BX
        // 1B C3 = SBB AX, BX
        // 11 06 00 40 = ADC word [0x4000], AX
        // 1B 06 00 40 = SBB AX, word [0x4000]
        mem[0] = 0x10;
        mem[1] = 0xD8;
        mem[2] = 0x12;
        mem[3] = 0xC3;
        mem[4] = 0x11;
        mem[5] = 0xD8;
        mem[6] = 0x13;
        mem[7] = 0xC3;
        mem[8] = 0x18;
        mem[9] = 0xD8;
        mem[10] = 0x1A;
        mem[11] = 0xC3;
        mem[12] = 0x19;
        mem[13] = 0xD8;
        mem[14] = 0x1B;
        mem[15] = 0xC3;
        mem[16] = 0x11;
        mem[17] = 0x06;
        mem[18] = 0x00;
        mem[19] = 0x40;
        mem[20] = 0x1B;
        mem[21] = 0x06;
        mem[22] = 0x00;
        mem[23] = 0x40;
        mem[24] = 0xF4;
        mem[0x4000] = 0x00;
        mem[0x4001] = 0x80; // 0x8000

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };

        // ADC AL, BL (10): 0x10 + 0x20 + CF1 = 0x31
        cpu.set_al(0x10);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x20);
        cpu.set_cf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x31);
        assert_eq!(cpu.rflags & 1, 0); // CF clear

        // ADC AL, BL (12): reg ← reg + r/m + CF; 0x7F + 0 + CF1 → 0x80, OF set
        cpu.set_al(0x7F);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x00);
        cpu.set_cf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x80);
        assert_ne!(cpu.rflags & (1 << 11), 0); // OF
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF

        // ADC AX, BX (11): 0x1000 + 0x0200 + CF0 = 0x1200
        cpu.set_ax(0x1000);
        cpu.set_gpr_u16(CpuState::RBX, 0x0200);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x1200);

        // ADC AX, BX (13): 0xFFFF + 1 + CF0 → 0, CF set, ZF set
        cpu.set_ax(0xFFFF);
        cpu.set_gpr_u16(CpuState::RBX, 0x0001);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0000);
        assert_ne!(cpu.rflags & 1, 0); // CF
        assert_ne!(cpu.rflags & (1 << 6), 0); // ZF

        // SBB AL, BL (18): 0x05 - 0x02 - CF1 = 0x02
        cpu.set_al(0x05);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x02);
        cpu.set_cf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x02);
        assert_eq!(cpu.rflags & 1, 0);

        // SBB AL, BL (1A): 0x00 - 0x00 - CF1 = 0xFF, CF set
        cpu.set_al(0x00);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x00);
        cpu.set_cf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xFF);
        assert_ne!(cpu.rflags & 1, 0); // CF
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF

        // SBB AX, BX (19): 0x1000 - 0x0001 - CF0 = 0x0FFF
        cpu.set_ax(0x1000);
        cpu.set_gpr_u16(CpuState::RBX, 0x0001);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0FFF);

        // SBB AX, BX (1B): 0x0000 - 0x0001 - CF0 = 0xFFFF, CF set
        cpu.set_ax(0x0000);
        cpu.set_gpr_u16(CpuState::RBX, 0x0001);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0xFFFF);
        assert_ne!(cpu.rflags & 1, 0);

        // ADC [0x4000], AX — mem dest: 0x8000 + 0x0001 + CF1 = 0x8002
        cpu.set_ax(0x0001);
        cpu.set_cf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x4000).unwrap(), 0x8002);

        // SBB AX, [0x4000]: 0x8003 - 0x8002 - CF0 = 0x0001
        cpu.set_ax(0x8003);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0001);
        assert_eq!(cpu.rflags & 1, 0);
    }

    /// ADC/SBB AL/AX,imm — 14/15/1C/1D (SDM Vol. 2 ADC/SBB accumulator forms).
    #[test]
    fn adc_sbb_al_ax_imm_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 14 01       ADC AL, 0x01
        // 15 00 10    ADC AX, 0x1000
        // 1C 02       SBB AL, 0x02
        // 1D 01 00    SBB AX, 0x0001
        // 14 FF       ADC AL, 0xFF  (CF+wrap)
        // 1C 00       SBB AL, 0     (with CF)
        mem[0] = 0x14;
        mem[1] = 0x01;
        mem[2] = 0x15;
        mem[3] = 0x00;
        mem[4] = 0x10;
        mem[5] = 0x1C;
        mem[6] = 0x02;
        mem[7] = 0x1D;
        mem[8] = 0x01;
        mem[9] = 0x00;
        mem[10] = 0x14;
        mem[11] = 0xFF;
        mem[12] = 0x1C;
        mem[13] = 0x00;
        mem[14] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };

        // ADC AL, 1 with CF=1: 0x10 + 0x01 + 1 = 0x12; AH preserved
        cpu.set_ax(0xAB10);
        cpu.set_cf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x12);
        assert_eq!(cpu.ax(), 0xAB12);
        assert_eq!(cpu.rflags & 1, 0);

        // ADC AX, 0x1000 with CF=0: 0x0200 + 0x1000 = 0x1200
        cpu.set_ax(0x0200);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x1200);

        // SBB AL, 2 with CF=1: 0x05 - 0x02 - 1 = 0x02
        cpu.set_ax(0xCD05);
        cpu.set_cf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x02);
        assert_eq!(cpu.ax(), 0xCD02);
        assert_eq!(cpu.rflags & 1, 0);

        // SBB AX, 1 with CF=0: 0x1000 - 1 = 0x0FFF
        cpu.set_ax(0x1000);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0FFF);

        // ADC AL, 0xFF with CF=0: 0x01 + 0xFF = 0x00, CF set, ZF set
        cpu.set_al(0x01);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x00);
        assert_ne!(cpu.rflags & 1, 0);
        assert_ne!(cpu.rflags & (1 << 6), 0); // ZF

        // SBB AL, 0 with CF=1: 0x00 - 0 - 1 = 0xFF, CF set, SF set
        cpu.set_al(0x00);
        cpu.set_cf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xFF);
        assert_ne!(cpu.rflags & 1, 0);
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF
    }

    /// OR/AND AL/AX,imm — 0C/0D/24/25 (SDM Vol. 2 OR/AND accumulator forms).
    #[test]
    fn and_or_al_ax_imm_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 0C 0F       OR  AL, 0x0F
        // 0D F0 0F    OR  AX, 0x0FF0
        // 24 F0       AND AL, 0xF0
        // 25 FF 00    AND AX, 0x00FF
        // 0C 00       OR  AL, 0     (ZF)
        mem[0] = 0x0C;
        mem[1] = 0x0F;
        mem[2] = 0x0D;
        mem[3] = 0xF0;
        mem[4] = 0x0F;
        mem[5] = 0x24;
        mem[6] = 0xF0;
        mem[7] = 0x25;
        mem[8] = 0xFF;
        mem[9] = 0x00;
        mem[10] = 0x0C;
        mem[11] = 0x00;
        mem[12] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_ax(0x12F0); // AH=0x12 must survive AL ops
        cpu.set_cf(true);
        cpu.set_of(true);
        let mut bus = VecBus { mem, ports: vec![] };

        // OR AL, 0x0F → AL = 0xFF; CF/OF cleared; SF set
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xFF);
        assert_eq!(cpu.ax(), 0x12FF);
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 11), 0);
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF

        // OR AX, 0x0FF0
        cpu.set_ax(0xF000);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0xFFF0);
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF

        // AND AL, 0xF0
        cpu.set_ax(0x34AB);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xA0);
        assert_eq!(cpu.ax(), 0x34A0);
        assert_eq!(cpu.rflags & 1, 0);

        // AND AX, 0x00FF
        cpu.set_ax(0x1234);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0034);
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF=0

        // OR AL, 0 → ZF
        cpu.set_al(0);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0);
        assert_ne!(cpu.rflags & (1 << 6), 0); // ZF
    }

    /// XOR ModRM byte 30/32 — results and logic flags (SDM Vol. 2 XOR).
    #[test]
    fn xor_modrm_byte_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 30 D8 = XOR AL, BL  (r/m ← r/m ^ reg)
        // 32 C3 = XOR AL, BL  (reg ← reg ^ r/m)
        // 30 06 00 40 = XOR byte [0x4000], AL
        // 32 06 00 40 = XOR AL, byte [0x4000]
        mem[0] = 0x30;
        mem[1] = 0xD8;
        mem[2] = 0x32;
        mem[3] = 0xC3;
        mem[4] = 0x30;
        mem[5] = 0x06;
        mem[6] = 0x00;
        mem[7] = 0x40;
        mem[8] = 0x32;
        mem[9] = 0x06;
        mem[10] = 0x00;
        mem[11] = 0x40;
        mem[12] = 0xF4;
        mem[0x4000] = 0xF0;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };

        // XOR AL, BL (30): 0xF0 ^ 0x0F = 0xFF; CF/OF cleared; SF set
        cpu.set_al(0xF0);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x0F);
        cpu.set_cf(true);
        cpu.set_of(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xFF);
        assert_eq!(cpu.rflags & 1, 0); // CF cleared
        assert_eq!(cpu.rflags & (1 << 11), 0); // OF cleared
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF

        // XOR AL, BL (32): 0xAA ^ 0x55 = 0xFF
        cpu.set_al(0xAA);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x55);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xFF);

        // XOR [0x4000], AL (30): 0xF0 ^ 0x0F = 0xFF
        cpu.set_al(0x0F);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x4000).unwrap(), 0xFF);

        // XOR AL, [0x4000] (32): 0x11 ^ 0xFF = 0xEE; ZF clear
        cpu.set_al(0x11);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xEE);
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF
    }

    /// ADD/SUB ModRM byte 00/02/28/2A — results and arithmetic flags (SDM Vol. 2 ADD/SUB).
    #[test]
    fn add_sub_modrm_byte_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 00 D8 = ADD AL, BL  (r/m ← r/m + reg)
        // 02 C3 = ADD AL, BL  (reg ← reg + r/m)
        // 28 D8 = SUB AL, BL
        // 2A C3 = SUB AL, BL
        // 00 06 00 40 = ADD byte [0x4000], AL
        // 2A 06 00 40 = SUB AL, byte [0x4000]
        mem[0] = 0x00;
        mem[1] = 0xD8;
        mem[2] = 0x02;
        mem[3] = 0xC3;
        mem[4] = 0x28;
        mem[5] = 0xD8;
        mem[6] = 0x2A;
        mem[7] = 0xC3;
        mem[8] = 0x00;
        mem[9] = 0x06;
        mem[10] = 0x00;
        mem[11] = 0x40;
        mem[12] = 0x2A;
        mem[13] = 0x06;
        mem[14] = 0x00;
        mem[15] = 0x40;
        mem[16] = 0xF4;
        mem[0x4000] = 0x10;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };

        // ADD AL, BL (00): 0x70 + 0x10 = 0x80; CF=0; SF set; OF set (signed overflow)
        cpu.set_al(0x70);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x10);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x80);
        assert_eq!(cpu.rflags & 1, 0); // CF
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF
        assert_ne!(cpu.rflags & (1 << 11), 0); // OF

        // ADD AL, BL (02): 0x01 + 0x02 = 0x03; ZF clear
        cpu.set_al(0x01);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x02);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x03);
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF

        // SUB AL, BL (28): 0x05 - 0x10 = 0xF5; CF set; SF set
        cpu.set_al(0x05);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x10);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xF5);
        assert_ne!(cpu.rflags & 1, 0); // CF
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF

        // SUB AL, BL (2A): 0x10 - 0x10 = 0; ZF set; CF clear
        cpu.set_al(0x10);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x10);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0);
        assert_ne!(cpu.rflags & (1 << 6), 0); // ZF
        assert_eq!(cpu.rflags & 1, 0); // CF

        // ADD [0x4000], AL (00): 0x10 + 0x05 = 0x15
        cpu.set_al(0x05);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x4000).unwrap(), 0x15);

        // SUB AL, [0x4000] (2A): 0x20 - 0x15 = 0x0B
        cpu.set_al(0x20);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x0B);
        assert_eq!(cpu.rflags & 1, 0); // CF
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF
    }

    /// CMP ModRM byte 38/3A — flags only, operands unchanged (SDM Vol. 2 CMP).
    #[test]
    fn cmp_modrm_byte_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 38 D8 = CMP AL, BL  (r/m − reg → flags)
        // 3A C3 = CMP AL, BL  (reg − r/m → flags)
        // 38 06 00 40 = CMP byte [0x4000], AL
        // 3A 06 00 40 = CMP AL, byte [0x4000]
        mem[0] = 0x38;
        mem[1] = 0xD8;
        mem[2] = 0x3A;
        mem[3] = 0xC3;
        mem[4] = 0x38;
        mem[5] = 0x06;
        mem[6] = 0x00;
        mem[7] = 0x40;
        mem[8] = 0x3A;
        mem[9] = 0x06;
        mem[10] = 0x00;
        mem[11] = 0x40;
        mem[12] = 0xF4;
        mem[0x4000] = 0x10;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };

        // CMP AL, BL (38): 0x05 − 0x10 → CF/SF set; AL unchanged
        cpu.set_al(0x05);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x10);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x05);
        assert_eq!(cpu.gpr_u8_low(CpuState::RBX), 0x10);
        assert_ne!(cpu.rflags & 1, 0); // CF
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF

        // CMP AL, BL (3A): 0x10 − 0x10 → ZF; CF clear; AL unchanged
        cpu.set_al(0x10);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x10);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x10);
        assert_ne!(cpu.rflags & (1 << 6), 0); // ZF
        assert_eq!(cpu.rflags & 1, 0); // CF

        // CMP [0x4000], AL (38): 0x10 − 0x05 → CF clear; mem unchanged
        cpu.set_al(0x05);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x4000).unwrap(), 0x10);
        assert_eq!(cpu.rflags & 1, 0); // CF
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF

        // CMP AL, [0x4000] (3A): 0x05 − 0x10 → CF/SF; AL unchanged
        cpu.set_al(0x05);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x05);
        assert_eq!(bus.read_u8(0x4000).unwrap(), 0x10);
        assert_ne!(cpu.rflags & 1, 0); // CF
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF
    }

    /// SUB/XOR/CMP AL/AX,imm — 2C/2D/34/35/3C/3D (SDM Vol. 2 accumulator forms).
    #[test]
    fn sub_xor_cmp_al_ax_imm_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 2C 01       SUB AL, 0x01
        // 2D 00 10    SUB AX, 0x1000
        // 34 0F       XOR AL, 0x0F
        // 35 FF 00    XOR AX, 0x00FF
        // 3C 05       CMP AL, 0x05
        // 3D 34 12    CMP AX, 0x1234
        // 2C 01       SUB AL, 1  (borrow → CF)
        mem[0] = 0x2C;
        mem[1] = 0x01;
        mem[2] = 0x2D;
        mem[3] = 0x00;
        mem[4] = 0x10;
        mem[5] = 0x34;
        mem[6] = 0x0F;
        mem[7] = 0x35;
        mem[8] = 0xFF;
        mem[9] = 0x00;
        mem[10] = 0x3C;
        mem[11] = 0x05;
        mem[12] = 0x3D;
        mem[13] = 0x34;
        mem[14] = 0x12;
        mem[15] = 0x2C;
        mem[16] = 0x01;
        mem[17] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };

        // SUB AL, 1: 0x10 - 1 = 0x0F; AH preserved
        cpu.set_ax(0xAB10);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x0F);
        assert_eq!(cpu.ax(), 0xAB0F);
        assert_eq!(cpu.rflags & 1, 0); // CF
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF

        // SUB AX, 0x1000: 0x2000 - 0x1000 = 0x1000
        cpu.set_ax(0x2000);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x1000);
        assert_eq!(cpu.rflags & 1, 0);

        // XOR AL, 0x0F: 0xF0 ^ 0x0F = 0xFF; CF/OF cleared; SF set
        cpu.set_ax(0x12F0);
        cpu.set_cf(true);
        cpu.set_of(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xFF);
        assert_eq!(cpu.ax(), 0x12FF);
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 11), 0);
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF

        // XOR AX, 0x00FF: 0x1234 ^ 0x00FF = 0x12CB
        cpu.set_ax(0x1234);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x12CB);

        // CMP AL, 5: 5 - 5 → ZF; AL unchanged
        cpu.set_ax(0xCD05);
        let al_before = cpu.al();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), al_before);
        assert_eq!(cpu.ax(), 0xCD05);
        assert_ne!(cpu.rflags & (1 << 6), 0); // ZF
        assert_eq!(cpu.rflags & 1, 0); // CF

        // CMP AX, 0x1234: 0x1000 - 0x1234 → CF set; AX unchanged
        cpu.set_ax(0x1000);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x1000);
        assert_ne!(cpu.rflags & 1, 0); // CF
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF

        // SUB AL, 1: 0x00 - 1 = 0xFF, CF set, SF set
        cpu.set_al(0x00);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xFF);
        assert_ne!(cpu.rflags & 1, 0); // CF
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF
    }

    /// ADD AX,imm16 — 05 iw (SDM Vol. 2 ADD accumulator form).
    #[test]
    fn add_ax_imm_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 05 34 12    ADD AX, 0x1234
        // 05 01 00    ADD AX, 0x0001  (carry from 0xFFFF)
        // 05 00 80    ADD AX, 0x8000  (signed overflow)
        mem[0] = 0x05;
        mem[1] = 0x34;
        mem[2] = 0x12;
        mem[3] = 0x05;
        mem[4] = 0x01;
        mem[5] = 0x00;
        mem[6] = 0x05;
        mem[7] = 0x00;
        mem[8] = 0x80;
        mem[9] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };

        // ADD AX, 0x1234: 0x1000 + 0x1234 = 0x2234; CF/OF/ZF clear
        cpu.set_ax(0x1000);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x2234);
        assert_eq!(cpu.rflags & 1, 0); // CF
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF
        assert_eq!(cpu.rflags & (1 << 11), 0); // OF

        // ADD AX, 1: 0xFFFF + 1 = 0; CF and ZF set
        cpu.set_ax(0xFFFF);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0000);
        assert_ne!(cpu.rflags & 1, 0); // CF
        assert_ne!(cpu.rflags & (1 << 6), 0); // ZF

        // ADD AX, 0x8000: 0x8000 + 0x8000 = 0; CF and OF set
        cpu.set_ax(0x8000);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0000);
        assert_ne!(cpu.rflags & 1, 0); // CF
        assert_ne!(cpu.rflags & (1 << 11), 0); // OF
        assert_ne!(cpu.rflags & (1 << 6), 0); // ZF
    }

    /// FE/FF Group 4/5 INC/DEC r/m — /0 INC, /1 DEC; CF preserved (SDM Vol. 2 INC/DEC).
    #[test]
    fn grp4_grp5_inc_dec_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // FE C0          INC AL
        // FE C3          INC BL
        // FE C8          DEC AL
        // FE 06 00 40    INC byte [0x4000]
        // FE 0E 00 40    DEC byte [0x4000]
        // FF C0          INC AX
        // FF C8          DEC AX
        // FF 06 00 40    INC word [0x4000]
        // FF 0E 00 40    DEC word [0x4000]
        // FE D0          FE /2 — unsupported
        mem[0] = 0xFE;
        mem[1] = 0xC0;
        mem[2] = 0xFE;
        mem[3] = 0xC3;
        mem[4] = 0xFE;
        mem[5] = 0xC8;
        mem[6] = 0xFE;
        mem[7] = 0x06;
        mem[8] = 0x00;
        mem[9] = 0x40;
        mem[10] = 0xFE;
        mem[11] = 0x0E;
        mem[12] = 0x00;
        mem[13] = 0x40;
        mem[14] = 0xFF;
        mem[15] = 0xC0;
        mem[16] = 0xFF;
        mem[17] = 0xC8;
        mem[18] = 0xFF;
        mem[19] = 0x06;
        mem[20] = 0x00;
        mem[21] = 0x40;
        mem[22] = 0xFF;
        mem[23] = 0x0E;
        mem[24] = 0x00;
        mem[25] = 0x40;
        mem[26] = 0xFE;
        mem[27] = 0xD0;
        mem[28] = 0xF4;
        mem[0x4000] = 0x7F;
        mem[0x4001] = 0x00;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };

        // INC AL: 0xFF → 0; ZF; CF preserved
        cpu.set_al(0xFF);
        cpu.set_cf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x00);
        assert_ne!(cpu.rflags & (1 << 6), 0); // ZF
        assert_ne!(cpu.rflags & 1, 0); // CF preserved

        // INC BL: 0x7F → 0x80; OF; CF preserved clear
        cpu.set_gpr_u8_low(CpuState::RBX, 0x7F);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u8_low(CpuState::RBX), 0x80);
        assert_ne!(cpu.rflags & (1 << 11), 0); // OF
        assert_eq!(cpu.rflags & 1, 0); // CF preserved

        // DEC AL: 0x00 → 0xFF; SF; CF preserved
        cpu.set_al(0x00);
        cpu.set_cf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xFF);
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF
        assert_ne!(cpu.rflags & 1, 0); // CF preserved

        // INC byte [0x4000]: 0x7F → 0x80
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x4000).unwrap(), 0x80);
        assert_ne!(cpu.rflags & (1 << 11), 0); // OF

        // DEC byte [0x4000]: 0x80 → 0x7F
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x4000).unwrap(), 0x7F);

        // INC AX: 0x7FFF → 0x8000; OF; CF preserved
        cpu.set_ax(0x7FFF);
        cpu.set_cf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x8000);
        assert_ne!(cpu.rflags & (1 << 11), 0); // OF
        assert_ne!(cpu.rflags & 1, 0); // CF preserved

        // DEC AX: 0x0001 → 0; ZF; CF preserved clear
        cpu.set_ax(0x0001);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0000);
        assert_ne!(cpu.rflags & (1 << 6), 0); // ZF
        assert_eq!(cpu.rflags & 1, 0); // CF preserved

        // INC word [0x4000]: 0x007F → 0x0080
        bus.write_u16(0x4000, 0x007F).unwrap();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x4000).unwrap(), 0x0080);

        // DEC word [0x4000]: 0x0080 → 0x007F
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x4000).unwrap(), 0x007F);

        // FE /2 unsupported
        assert!(matches!(
            step(&mut cpu, &mut bus),
            Err(ExecError::Unsupported(0xFE))
        ));
    }

    /// FF Group 5 CALL/JMP/PUSH r/m — /2 CALL near, /4 JMP near, /6 PUSH (SDM Vol. 2).
    #[test]
    fn grp5_call_jmp_push_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 0: FF D0          CALL AX
        // 2: F4             HLT (return landing)
        // 3: FF 16 00 40    CALL word [0x4000]
        // 7: F4             HLT
        // 8: FF E3          JMP BX
        // A: F4             HLT (should not reach)
        // B: FF 26 00 40    JMP word [0x4000]
        // F: F4             HLT (should not reach)
        // 10: FF F0         PUSH AX
        // 12: FF 36 00 40   PUSH word [0x4000]
        // 16: FF D8         FF /3 CALL far reg — #UD / Unsupported
        // 18: F4            HLT
        mem[0] = 0xFF;
        mem[1] = 0xD0;
        mem[2] = 0xF4;
        mem[3] = 0xFF;
        mem[4] = 0x16;
        mem[5] = 0x00;
        mem[6] = 0x40;
        mem[7] = 0xF4;
        mem[8] = 0xFF;
        mem[9] = 0xE3;
        mem[0xA] = 0xF4;
        mem[0xB] = 0xFF;
        mem[0xC] = 0x26;
        mem[0xD] = 0x00;
        mem[0xE] = 0x40;
        mem[0xF] = 0xF4;
        mem[0x10] = 0xFF;
        mem[0x11] = 0xF0;
        mem[0x12] = 0xFF;
        mem[0x13] = 0x36;
        mem[0x14] = 0x00;
        mem[0x15] = 0x40;
        mem[0x16] = 0xFF;
        mem[0x17] = 0xD8;
        mem[0x18] = 0xF4;
        // Call/jmp targets
        mem[0x800] = 0xC3; // RET (near)
        mem[0x900] = 0xF4; // HLT
        mem[0x4000] = 0x00;
        mem[0x4001] = 0x09; // word 0x0900
        mem[0x4002] = 0x34;
        mem[0x4003] = 0x12; // word 0x1234 for PUSH mem

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        // CALL AX → 0x800: push return IP 2, jump
        cpu.set_ax(0x0800);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0800);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFC);
        assert_eq!(bus.read_u16(0xFFFC).unwrap(), 2);

        // RET back to HLT at 2
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 2);
        step(&mut cpu, &mut bus).unwrap(); // HLT
        assert!(cpu.halted);

        // CALL word [0x4000] → 0x900
        cpu.halted = false;
        cpu.rip = 3;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0900);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFC);
        assert_eq!(bus.read_u16(0xFFFC).unwrap(), 7); // return after CALL mem

        // JMP BX → 0x900
        cpu.rip = 8;
        cpu.set_gpr_u16(CpuState::RBX, 0x0900);
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0900);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE); // no stack change

        // JMP word [0x4000] → 0x900
        cpu.rip = 0xB;
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0900);

        // PUSH AX
        cpu.rip = 0x10;
        cpu.set_ax(0xABCD);
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x12);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFC);
        assert_eq!(bus.read_u16(0xFFFC).unwrap(), 0xABCD);

        // PUSH word [0x4000] — use 0x1234 at 0x4002 via displacement change:
        // encoding still [0x4000]; overwrite target word for this step.
        bus.write_u16(0x4000, 0x1234).unwrap();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x16);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFA);
        assert_eq!(bus.read_u16(0xFFFA).unwrap(), 0x1234);

        // FF /3 register form unsupported (#UD)
        assert!(matches!(
            step(&mut cpu, &mut bus),
            Err(ExecError::Unsupported(0xFF))
        ));
    }

    /// FF Group 5 far CALL/JMP m16:16 — /3 CALL far, /5 JMP far (SDM Vol. 2).
    #[test]
    fn grp5_call_jmp_far_real_mode() {
        let mut mem = vec![0u8; 0x20000];
        // 0: FF 1E 00 40    CALL FAR [0x4000]
        // 4: F4             HLT (return landing after RETF)
        // 5: FF 2E 00 40    JMP FAR [0x4000]
        // 9: F4             HLT (should not reach after JMP)
        // A: FF D8          CALL FAR AX — #UD
        // C: FF E8          JMP FAR AX — #UD
        // E: F4             HLT
        mem[0] = 0xFF;
        mem[1] = 0x1E;
        mem[2] = 0x00;
        mem[3] = 0x40;
        mem[4] = 0xF4;
        mem[5] = 0xFF;
        mem[6] = 0x2E;
        mem[7] = 0x00;
        mem[8] = 0x40;
        mem[9] = 0xF4;
        mem[0xA] = 0xFF;
        mem[0xB] = 0xD8;
        mem[0xC] = 0xFF;
        mem[0xD] = 0xE8;
        mem[0xE] = 0xF4;
        // Far pointer at DS:0x4000 → CS:IP = 0x1000:0x0200 → linear 0x10200
        mem[0x4000] = 0x00;
        mem[0x4001] = 0x02; // offset 0x0200
        mem[0x4002] = 0x00;
        mem[0x4003] = 0x10; // selector 0x1000
                            // Target: RETF then HLT at 0x1000:0x0200
        let target = (0x1000u64 << 4) + 0x0200;
        mem[target as usize] = 0xCB; // RETF
        mem[target as usize + 1] = 0xF4; // HLT (JMP landing)

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        // CALL FAR [0x4000]: push CS/IP, load 0x1000:0x0200
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.cs.selector, 0x1000);
        assert_eq!(cpu.cs.base, 0x1000u64 << 4);
        assert_eq!(cpu.ip16(), 0x0200);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFA);
        assert_eq!(bus.read_u16(0xFFFA).unwrap(), 4); // return IP
        assert_eq!(bus.read_u16(0xFFFC).unwrap(), 0); // return CS

        // RETF back to HLT at 4
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.cs.selector, 0);
        assert_eq!(cpu.ip16(), 4);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE);
        step(&mut cpu, &mut bus).unwrap(); // HLT
        assert!(cpu.halted);

        // JMP FAR [0x4000] → 0x1000:0x0200 (HLT after we overwrite RETF)
        cpu.halted = false;
        cpu.rip = 5;
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        bus.write_u8(target, 0xF4).unwrap(); // HLT at far target
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.cs.selector, 0x1000);
        assert_eq!(cpu.ip16(), 0x0200);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE); // no stack change
        step(&mut cpu, &mut bus).unwrap(); // HLT
        assert!(cpu.halted);

        // Register forms of far CALL/JMP are #UD
        cpu.halted = false;
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0xA;
        assert!(matches!(
            step(&mut cpu, &mut bus),
            Err(ExecError::Unsupported(0xFF))
        ));
        cpu.rip = 0xC;
        assert!(matches!(
            step(&mut cpu, &mut bus),
            Err(ExecError::Unsupported(0xFF))
        ));
    }

    /// AH/CH/DH/BH via ModR/M reg and r/m for MOV and OR (SDM Vol. 1 ┬º3.4.1.1; Vol. 2 MOV/OR).
    #[test]
    fn high_byte_modrm_mov_or_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 88 E0 = MOV AL, AH   (r/m=AL, reg=AH)
        // 8A E3 = MOV AH, BL   (reg=AH, r/m=BL)
        // 88 FD = MOV CH, BH   (r/m=CH, reg=BH)
        // 8A F9 = MOV BH, CL   (reg=BH, r/m=CL)
        // 08 E5 = OR  CH, AH   (r/m=CH, reg=AH)
        // 0A F1 = OR  DH, CL   (reg=DH, r/m=CL)
        // B4 77 = MOV AH, 0x77
        // B7 88 = MOV BH, 0x88
        // 80 E4 0F = AND AH, 0x0F  (Group 1 /4, r/m=AH)
        mem[0] = 0x88;
        mem[1] = 0xE0;
        mem[2] = 0x8A;
        mem[3] = 0xE3;
        mem[4] = 0x88;
        mem[5] = 0xFD;
        mem[6] = 0x8A;
        mem[7] = 0xF9;
        mem[8] = 0x08;
        mem[9] = 0xE5;
        mem[10] = 0x0A;
        mem[11] = 0xF1;
        mem[12] = 0xB4;
        mem[13] = 0x77;
        mem[14] = 0xB7;
        mem[15] = 0x88;
        mem[16] = 0x80;
        mem[17] = 0xE4; // mod=3,reg=4(AND),rm=4(AH)
        mem[18] = 0x0F;
        mem[19] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        // AX=0xABCD, BX=0x1234, CX=0x5678, DX=0x9ABC
        cpu.set_ax(0xABCD);
        cpu.set_gpr_u16(CpuState::RBX, 0x1234);
        cpu.set_gpr_u16(CpuState::RCX, 0x5678);
        cpu.set_gpr_u16(CpuState::RDX, 0x9ABC);
        let mut bus = VecBus { mem, ports: vec![] };

        // MOV AL, AH ΓåÆ AL=0xAB, AH unchanged ΓåÆ AX=0xABAB
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0xABAB);

        // MOV AH, BL ΓåÆ AH=0x34, AL preserved ΓåÆ AX=0x34AB
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x34AB);
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0x1234);

        // MOV CH, BH ΓåÆ CH=0x12, CL preserved ΓåÆ CX=0x1278
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0x1278);
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0x1234);

        // MOV BH, CL ΓåÆ BH=0x78, BL preserved ΓåÆ BX=0x7834
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0x7834);

        // OR CH, AH ΓåÆ CH |= AH = 0x12 | 0x34 = 0x36 ΓåÆ CX=0x3678
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0x3678);
        assert_eq!(cpu.ax(), 0x34AB);

        // OR DH, CL ΓåÆ DH |= CL = 0x9A | 0x78 = 0xFA ΓåÆ DX=0xFABC
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RDX), 0xFABC);

        // MOV AH, 0x77 ΓåÆ AX=0x77AB
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x77AB);

        // MOV BH, 0x88 ΓåÆ BX=0x8834
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0x8834);

        // AND AH, 0x0F ΓåÆ AH=0x07, AL preserved ΓåÆ AX=0x07AB
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x07AB);
        assert_eq!(cpu.rflags & 1, 0); // CF cleared by AND
    }

    /// XCHG high-byte reg Γåö r/m (SDM Vol. 2 XCHG).
    #[test]
    fn high_byte_xchg_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 86 E3 = XCHG AH, BL
        mem[0] = 0x86;
        mem[1] = 0xE3;
        mem[2] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_ax(0x11AA);
        cpu.set_gpr_u16(CpuState::RBX, 0x22BB);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        // AHΓåöBL: AH=0xBB, BL=0x11; AL/BH preserved
        assert_eq!(cpu.ax(), 0xBBAA);
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0x2211);
    }

    /// MOV C6/C7 r/m,imm — Spec: Intel SDM Vol. 2 MOV.
    #[test]
    fn mov_rm_imm_c6_c7_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // C6 C0 5A = MOV AL, 0x5A
        // C6 06 00 40 99 = MOV byte [0x4000], 0x99
        // C7 C3 34 12 = MOV BX, 0x1234
        // C7 06 00 30 CD AB = MOV word [0x3000], 0xABCD
        // C6 C8 00 = MOV /1 — unsupported
        mem[0] = 0xC6;
        mem[1] = 0xC0;
        mem[2] = 0x5A;
        mem[3] = 0xC6;
        mem[4] = 0x06;
        mem[5] = 0x00;
        mem[6] = 0x40;
        mem[7] = 0x99;
        mem[8] = 0xC7;
        mem[9] = 0xC3;
        mem[10] = 0x34;
        mem[11] = 0x12;
        mem[12] = 0xC7;
        mem[13] = 0x06;
        mem[14] = 0x00;
        mem[15] = 0x30;
        mem[16] = 0xCD;
        mem[17] = 0xAB;
        mem[18] = 0xC6;
        mem[19] = 0xC8;
        mem[20] = 0x00;
        mem[21] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_al(0);
        cpu.rflags = 0x246;
        let flags_before = cpu.rflags;
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x5A);
        assert_eq!(cpu.rflags, flags_before); // MOV does not touch flags

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x4000).unwrap(), 0x99);
        assert_eq!(cpu.rflags, flags_before);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0x1234);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x3000).unwrap(), 0xABCD);

        assert_eq!(step(&mut cpu, &mut bus), Err(ExecError::Unsupported(0xC6)));
    }

    /// MOV A0–A3 AL/AX ↔ moffs — Spec: Intel SDM Vol. 2 MOV.
    #[test]
    fn mov_moffs_a0_a3_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // A0 00 40 = MOV AL, [0x4000]
        // A2 00 50 = MOV [0x5000], AL
        // A1 00 30 = MOV AX, [0x3000]
        // A3 00 60 = MOV [0x6000], AX
        // 2E A0 00 10 = MOV AL, CS:[0x1000]
        mem[0] = 0xA0;
        mem[1] = 0x00;
        mem[2] = 0x40;
        mem[3] = 0xA2;
        mem[4] = 0x00;
        mem[5] = 0x50;
        mem[6] = 0xA1;
        mem[7] = 0x00;
        mem[8] = 0x30;
        mem[9] = 0xA3;
        mem[10] = 0x00;
        mem[11] = 0x60;
        mem[12] = 0x2E;
        mem[13] = 0xA0;
        mem[14] = 0x00;
        mem[15] = 0x10;
        mem[16] = 0xF4;
        mem[0x4000] = 0xAB;
        mem[0x3000] = 0x34;
        mem[0x3001] = 0x12;
        mem[0x1000] = 0xCD; // CS=0 → linear 0x1000

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xAB);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x5000).unwrap(), 0xAB);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x1234);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x6000).unwrap(), 0x1234);

        cpu.set_al(0);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xCD);
    }

    /// TEST A8/A9 AL/AX,imm — Spec: Intel SDM Vol. 2 TEST.
    #[test]
    fn test_al_ax_imm_a8_a9_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // A8 0F = TEST AL, 0x0F
        // A9 FF 00 = TEST AX, 0x00FF
        // A8 00 = TEST AL, 0 (ZF)
        mem[0] = 0xA8;
        mem[1] = 0x0F;
        mem[2] = 0xA9;
        mem[3] = 0xFF;
        mem[4] = 0x00;
        mem[5] = 0xA8;
        mem[6] = 0x00;
        mem[7] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_al(0xF0);
        cpu.set_cf(true);
        cpu.set_of(true);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xF0); // unchanged
        assert_eq!(cpu.rflags & 1, 0); // CF cleared
        assert_eq!(cpu.rflags & (1 << 11), 0); // OF cleared
        assert_eq!(cpu.rflags & (1 << 6), 1 << 6); // ZF (0xF0 & 0x0F == 0)

        cpu.set_ax(0x12F0);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x12F0);
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF=0 (0x12F0 & 0x00FF == 0x00F0)
        assert_eq!(cpu.rflags & (1 << 7), 0); // SF from 16-bit result (bit 15 clear)

        cpu.set_al(0x55);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x55);
        assert_eq!(cpu.rflags & (1 << 6), 1 << 6); // ZF
    }

    /// PUSHA stack layout then POPA restores GPRs (except SP from the saved slot).
    /// Spec: Intel SDM Vol. 2 "PUSHA/PUSHAD", "POPA/POPAD".
    #[test]
    fn pusha_popa_stack_layout_and_round_trip() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x60; // PUSHA
        mem[1] = 0x61; // POPA
        mem[2] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let sp0 = 0xFFFE_u16;
        cpu.set_gpr_u16(CpuState::RSP, sp0);
        cpu.set_gpr_u16(CpuState::RAX, 0x1111);
        cpu.set_gpr_u16(CpuState::RCX, 0x2222);
        cpu.set_gpr_u16(CpuState::RDX, 0x3333);
        cpu.set_gpr_u16(CpuState::RBX, 0x4444);
        cpu.set_gpr_u16(CpuState::RBP, 0x5555);
        cpu.set_gpr_u16(CpuState::RSI, 0x6666);
        cpu.set_gpr_u16(CpuState::RDI, 0x7777);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RSP), sp0.wrapping_sub(16));
        // Highest addresses first: AX at sp0-2 … DI at sp0-16.
        assert_eq!(bus.read_u16(u64::from(sp0 - 2)).unwrap(), 0x1111); // AX
        assert_eq!(bus.read_u16(u64::from(sp0 - 4)).unwrap(), 0x2222); // CX
        assert_eq!(bus.read_u16(u64::from(sp0 - 6)).unwrap(), 0x3333); // DX
        assert_eq!(bus.read_u16(u64::from(sp0 - 8)).unwrap(), 0x4444); // BX
        assert_eq!(bus.read_u16(u64::from(sp0 - 10)).unwrap(), sp0); // original SP
        assert_eq!(bus.read_u16(u64::from(sp0 - 12)).unwrap(), 0x5555); // BP
        assert_eq!(bus.read_u16(u64::from(sp0 - 14)).unwrap(), 0x6666); // SI
        assert_eq!(bus.read_u16(u64::from(sp0 - 16)).unwrap(), 0x7777); // DI

        // Clobber GPRs (leave SP as after PUSHA).
        cpu.set_gpr_u16(CpuState::RAX, 0);
        cpu.set_gpr_u16(CpuState::RCX, 0);
        cpu.set_gpr_u16(CpuState::RDX, 0);
        cpu.set_gpr_u16(CpuState::RBX, 0);
        cpu.set_gpr_u16(CpuState::RBP, 0);
        cpu.set_gpr_u16(CpuState::RSI, 0);
        cpu.set_gpr_u16(CpuState::RDI, 0);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RAX), 0x1111);
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0x2222);
        assert_eq!(cpu.gpr_u16(CpuState::RDX), 0x3333);
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0x4444);
        assert_eq!(cpu.gpr_u16(CpuState::RBP), 0x5555);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x6666);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x7777);
        // POPA discards the saved SP; SP ends at the pre-PUSHA value.
        assert_eq!(cpu.gpr_u16(CpuState::RSP), sp0);
    }

    /// ENTER nesting level 0 + LEAVE round-trip (SDM Vol. 2 ENTER/LEAVE).
    #[test]
    fn enter_level0_leave_round_trip() {
        let mut mem = vec![0u8; 0x10000];
        // ENTER 8, 0
        mem[0] = 0xC8;
        mem[1] = 0x08;
        mem[2] = 0x00;
        mem[3] = 0x00;
        mem[4] = 0xC9; // LEAVE
        mem[5] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let sp0 = 0xFFFE_u16;
        cpu.set_gpr_u16(CpuState::RSP, sp0);
        cpu.set_gpr_u16(CpuState::RBP, 0xABCD);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        // After ENTER 8,0: PUSH old BP; BP = new frame; SP = BP - 8.
        assert_eq!(bus.read_u16(u64::from(sp0 - 2)).unwrap(), 0xABCD);
        let frame = sp0 - 2;
        assert_eq!(cpu.gpr_u16(CpuState::RBP), frame);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), frame.wrapping_sub(8));

        step(&mut cpu, &mut bus).unwrap(); // LEAVE
        assert_eq!(cpu.gpr_u16(CpuState::RBP), 0xABCD);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), sp0);
    }

    /// ENTER with nesting level > 0 is explicitly unsupported this slice.
    #[test]
    fn enter_nesting_nonzero_unsupported() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xC8;
        mem[1] = 0x00;
        mem[2] = 0x00;
        mem[3] = 0x01; // nesting = 1
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };
        assert!(matches!(
            step(&mut cpu, &mut bus),
            Err(ExecError::Unsupported(0xC8))
        ));
    }

    /// RET iw / RETF iw release stack bytes after the return frame.
    /// Spec: Intel SDM Vol. 2 "RET".
    #[test]
    fn ret_retf_imm16_release_stack() {
        let mut mem = vec![0u8; 0x10000];
        // Near: RET 4 with IP on stack and 4 dummy bytes below the frame.
        mem[0] = 0xC2;
        mem[1] = 0x04;
        mem[2] = 0x00;
        // Far: RETF 2 at 0x100
        mem[0x100] = 0xCA;
        mem[0x101] = 0x02;
        mem[0x102] = 0x00;

        // Near frame at SP=0xFFF0: IP=0x2000, then 4 pad bytes, then marker 0xBEEF at 0xFFF6.
        mem[0xFFF0] = 0x00;
        mem[0xFFF1] = 0x20; // return IP
        mem[0xFFF2] = 0x11;
        mem[0xFFF3] = 0x11;
        mem[0xFFF4] = 0x22;
        mem[0xFFF5] = 0x22;
        mem[0xFFF6] = 0xEF;
        mem[0xFFF7] = 0xBE;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFF0);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x2000);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF6);
        assert_eq!(bus.read_u16(0xFFF6).unwrap(), 0xBEEF);

        // Far frame at SP=0xFFF0: IP, CS, then 2 pad bytes, marker at 0xFFF6.
        bus.mem[0xFFF0] = 0x34;
        bus.mem[0xFFF1] = 0x12; // IP
        bus.mem[0xFFF2] = 0x00;
        bus.mem[0xFFF3] = 0x30; // CS 0x3000
        bus.mem[0xFFF4] = 0xAA;
        bus.mem[0xFFF5] = 0xAA;
        bus.mem[0xFFF6] = 0xEF;
        bus.mem[0xFFF7] = 0xBE;
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0x100;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFF0);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x1234);
        assert_eq!(cpu.cs.selector, 0x3000);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF6);
    }

    /// POP r/m16 (8F /0) reg and mem forms; /1 unsupported.
    /// Spec: Intel SDM Vol. 2 "POP".
    #[test]
    fn pop_rm16_reg_mem_and_invalid_reg() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x8F;
        mem[1] = 0xC3; // POP BX
        mem[2] = 0x8F;
        mem[3] = 0x06;
        mem[4] = 0x00;
        mem[5] = 0x40; // POP [0x4000]
        mem[6] = 0x8F;
        mem[7] = 0xC8; // /1 — unsupported

        // Stack: 0xAAAA then 0xBBBB
        mem[0xFFFA] = 0xBB;
        mem[0xFFFB] = 0xBB;
        mem[0xFFFC] = 0xAA;
        mem[0xFFFD] = 0xAA;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFC);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0xAAAA);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE);

        cpu.set_gpr_u16(CpuState::RSP, 0xFFFA);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x4000).unwrap(), 0xBBBB);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFC);

        assert!(matches!(
            step(&mut cpu, &mut bus),
            Err(ExecError::Unsupported(0x8F))
        ));
    }
}
