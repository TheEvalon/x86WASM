//! Reference interpreter for the lab opcode subset (M1 + early M2).
//!
//! Semantics follow Intel SDM Vol. 2 / Vol. 3 for the implemented forms only.

#![forbid(unsafe_code)]

use thiserror::Error;
use x86_core::CpuState;
use x86_decode::{decode, DecodeError, DecodedInsn};
use x86_mmu::{checked_linear_addr, linear_addr};

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
    fn read_u32(&mut self, addr: u64) -> Result<u32, ExecError> {
        let lo = self.read_u16(addr)?;
        let hi = self.read_u16(addr.wrapping_add(2))?;
        Ok(u32::from(lo) | (u32::from(hi) << 16))
    }
    fn write_u32(&mut self, addr: u64, val: u32) -> Result<(), ExecError> {
        self.write_u16(addr, val as u16)?;
        self.write_u16(addr.wrapping_add(2), (val >> 16) as u16)
    }
    fn port_in_u8(&mut self, port: u16) -> Result<u8, ExecError>;
    fn port_out_u8(&mut self, port: u16, val: u8) -> Result<(), ExecError>;
    /// Default: two consecutive byte ports (port, port+1). Machine buses may override.
    fn port_in_u16(&mut self, port: u16) -> Result<u16, ExecError> {
        let lo = self.port_in_u8(port)?;
        let hi = self.port_in_u8(port.wrapping_add(1))?;
        Ok(u16::from_le_bytes([lo, hi]))
    }
    fn port_out_u16(&mut self, port: u16, val: u16) -> Result<(), ExecError> {
        let bytes = val.to_le_bytes();
        self.port_out_u8(port, bytes[0])?;
        self.port_out_u8(port.wrapping_add(1), bytes[1])
    }
    /// Default: two consecutive word ports (port, port+2). Machine buses may override.
    fn port_in_u32(&mut self, port: u16) -> Result<u32, ExecError> {
        let lo = self.port_in_u16(port)?;
        let hi = self.port_in_u16(port.wrapping_add(2))?;
        Ok(u32::from(lo) | (u32::from(hi) << 16))
    }
    fn port_out_u32(&mut self, port: u16, val: u32) -> Result<(), ExecError> {
        self.port_out_u16(port, val as u16)?;
        self.port_out_u16(port.wrapping_add(2), (val >> 16) as u16)
    }

    /// Drain a device-model IRQ latch into the CPU (PIC stub).
    ///
    /// Default: none. Test buses may return a vector after N memory ops so
    /// REP can observe an interrupt between iterations. Full 8259 is later.
    fn poll_external_irq(&mut self) -> Option<u8> {
        None
    }
}

/// Host-visible execution errors.
///
/// Architectural faults delivered through the real-mode IVT return `Ok(())`
/// from [`step`] after vectoring:
/// - `#DE` 0, `#BR` 5, `#UD` 6, `#SS` 12, `#GP` 13 (and software INT vectors)
///
/// Remaining host errors:
/// - `Decode`: truncated fetch, or sparse-table misses that are **not**
///   architectural `#UD` (valid-but-unimplemented primary opcodes — see
///   [`real_mode_primary_opcode_is_ud`])
/// - `MemoryFault`: bus errors that could not be classified as `#GP`/`#SS`
///   (IVT delivery failure; stack helpers used during delivery stay unchecked)
/// - `Unsupported`: valid-but-unimplemented forms reached after decode
///   (ENTER/PUSHA/POPA/LEAVE with address-size 32 under 0x67 — needs ESP stack;
///   MOVSQ/… qword strings are not architectural in REX-less real mode)
/// - `ArchFault`: internal only — converted to IVT delivery inside [`step`];
///   never returned to callers of [`step`]/[`run`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ExecError {
    #[error(transparent)]
    Decode(#[from] DecodeError),
    #[error("memory fault at {0:#x}")]
    MemoryFault(u64),
    #[error("unsupported encoding for opcode 0x{0:02X}")]
    Unsupported(u8),
    /// Pending real-mode IVT delivery (`vector`); consumed by [`step`].
    #[error("architectural fault vector {0}")]
    ArchFault(u8),
}

/// SF/ZF/PF from an 8-bit BCD-adjust result (DAA/DAS/AAM/AAD).
/// Spec: Intel SDM Vol. 2 DAA/DAS/AAM/AAD — Flags Affected.
fn set_bcd_szp_flags_u8(cpu: &mut CpuState, result: u8) {
    cpu.set_sf(result & 0x80 != 0);
    cpu.set_zf(result == 0);
    cpu.set_pf(parity_even(result));
}

/// DAA — Decimal Adjust AL after Addition.
/// Spec: Intel SDM Vol. 2 "DAA". OF undefined (left unchanged).
fn exec_daa(cpu: &mut CpuState) {
    let old_al = cpu.al();
    let old_cf = cpu.rflags & 1 != 0;
    let af = cpu.rflags & (1 << 4) != 0;
    let mut al = old_al;
    cpu.set_cf(false);
    if (al & 0x0F) > 9 || af {
        let (r, carry) = al.overflowing_add(6);
        al = r;
        cpu.set_cf(old_cf || carry);
        cpu.set_af(true);
    } else {
        cpu.set_af(false);
    }
    if old_al > 0x99 || old_cf {
        al = al.wrapping_add(0x60);
        cpu.set_cf(true);
    } else {
        cpu.set_cf(false);
    }
    cpu.set_al(al);
    set_bcd_szp_flags_u8(cpu, al);
}

/// DAS — Decimal Adjust AL after Subtraction.
/// Spec: Intel SDM Vol. 2 "DAS". OF undefined (left unchanged).
fn exec_das(cpu: &mut CpuState) {
    let old_al = cpu.al();
    let old_cf = cpu.rflags & 1 != 0;
    let af = cpu.rflags & (1 << 4) != 0;
    let mut al = old_al;
    cpu.set_cf(false);
    if (al & 0x0F) > 9 || af {
        let (r, borrow) = al.overflowing_sub(6);
        al = r;
        cpu.set_cf(old_cf || borrow);
        cpu.set_af(true);
    } else {
        cpu.set_af(false);
    }
    if old_al > 0x99 || old_cf {
        al = al.wrapping_sub(0x60);
        cpu.set_cf(true);
    } else {
        cpu.set_cf(false);
    }
    cpu.set_al(al);
    set_bcd_szp_flags_u8(cpu, al);
}

/// AAA — ASCII Adjust After Addition.
/// Spec: Intel SDM Vol. 2 "AAA". OF/SF/ZF/PF undefined (left unchanged).
fn exec_aaa(cpu: &mut CpuState) {
    let al = cpu.al();
    let af = cpu.rflags & (1 << 4) != 0;
    if (al & 0x0F) > 9 || af {
        let ax = cpu.ax().wrapping_add(0x106);
        cpu.set_ax(ax);
        cpu.set_af(true);
        cpu.set_cf(true);
    } else {
        cpu.set_af(false);
        cpu.set_cf(false);
    }
    cpu.set_al(cpu.al() & 0x0F);
}

/// AAS — ASCII Adjust AL After Subtraction.
/// Spec: Intel SDM Vol. 2 "AAS". OF/SF/ZF/PF undefined (left unchanged).
fn exec_aas(cpu: &mut CpuState) {
    let al = cpu.al();
    let af = cpu.rflags & (1 << 4) != 0;
    if (al & 0x0F) > 9 || af {
        let ax = cpu.ax().wrapping_sub(0x106);
        cpu.set_ax(ax);
        cpu.set_af(true);
        cpu.set_cf(true);
    } else {
        cpu.set_af(false);
        cpu.set_cf(false);
    }
    cpu.set_al(cpu.al() & 0x0F);
}

fn parity_even(v: u8) -> bool {
    v.count_ones().is_multiple_of(2)
}

/// Two/three-operand IMUL (and Group 3 word IMUL fit check): CF=OF=1 iff signed
/// product does not fit in i16. SF/ZF/AF/PF undefined (left unchanged).
/// Spec: Intel SDM Vol. 2 "IMUL".
fn set_imul_flags_i16(cpu: &mut CpuState, prod: i32) {
    let fits = prod == i32::from(prod as i16);
    cpu.set_cf(!fits);
    cpu.set_of(!fits);
}

/// Two-operand IMUL opsize-32: CF=OF=1 iff signed product does not fit in i32.
/// SF/ZF/AF/PF undefined (left unchanged). Spec: Intel SDM Vol. 2 "IMUL".
fn set_imul_flags_i32(cpu: &mut CpuState, prod: i64) {
    let fits = prod == i64::from(prod as i32);
    cpu.set_cf(!fits);
    cpu.set_of(!fits);
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

fn set_logic_flags_u32(cpu: &mut CpuState, result: u32) {
    cpu.set_cf(false);
    cpu.set_of(false);
    cpu.set_af(false);
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x8000_0000 != 0);
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

fn set_add_flags_u32(cpu: &mut CpuState, a: u32, b: u32, result: u32) {
    cpu.set_cf((a as u64) + (b as u64) > 0xFFFF_FFFF);
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x8000_0000 != 0);
    cpu.set_pf(parity_even(result as u8));
    cpu.set_af(((a ^ b ^ result) & 0x10) != 0);
    let of = (!(a ^ b) & (a ^ result) & 0x8000_0000) != 0;
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

fn set_sub_flags_u32(cpu: &mut CpuState, a: u32, b: u32, result: u32) {
    cpu.set_cf(a < b);
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x8000_0000 != 0);
    cpu.set_pf(parity_even(result as u8));
    cpu.set_af(((a ^ b ^ result) & 0x10) != 0);
    let of = ((a ^ b) & (a ^ result) & 0x8000_0000) != 0;
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

fn set_adc_flags_u32(cpu: &mut CpuState, a: u32, b: u32, cf_in: bool, result: u32) {
    let cf = u32::from(cf_in);
    let sum = u64::from(a) + u64::from(b) + u64::from(cf);
    cpu.set_cf(sum > 0xFFFF_FFFF);
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x8000_0000 != 0);
    cpu.set_pf(parity_even(result as u8));
    cpu.set_af(((a & 0xF) + (b & 0xF) + cf) > 0xF);
    let of = (!(a ^ b) & (a ^ result) & 0x8000_0000) != 0;
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

fn set_sbb_flags_u32(cpu: &mut CpuState, a: u32, b: u32, cf_in: bool, result: u32) {
    let cf = u32::from(cf_in);
    cpu.set_cf(u64::from(a) < u64::from(b) + u64::from(cf));
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x8000_0000 != 0);
    cpu.set_pf(parity_even(result as u8));
    cpu.set_af((a & 0xF) < ((b & 0xF) + cf));
    let of = ((a ^ b) & (a ^ result) & 0x8000_0000) != 0;
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

/// Group 1 ALU on 32-bit operands (opsize override in 16-bit default modes).
/// Spec: Intel SDM Vol. 2 opcode map (81/83 /r); Vol. 2 Ch. 2 (66H).
fn grp1_u32(cpu: &mut CpuState, op: u8, a: u32, b: u32) -> Result<Option<u32>, ExecError> {
    let cf_in = cpu.rflags & 1 != 0;
    match op {
        0 => {
            let r = a.wrapping_add(b);
            set_add_flags_u32(cpu, a, b, r);
            Ok(Some(r))
        }
        1 => {
            let r = a | b;
            set_logic_flags_u32(cpu, r);
            Ok(Some(r))
        }
        2 => {
            let r = a.wrapping_add(b).wrapping_add(u32::from(cf_in));
            set_adc_flags_u32(cpu, a, b, cf_in, r);
            Ok(Some(r))
        }
        3 => {
            let r = a.wrapping_sub(b).wrapping_sub(u32::from(cf_in));
            set_sbb_flags_u32(cpu, a, b, cf_in, r);
            Ok(Some(r))
        }
        4 => {
            let r = a & b;
            set_logic_flags_u32(cpu, r);
            Ok(Some(r))
        }
        5 => {
            let r = a.wrapping_sub(b);
            set_sub_flags_u32(cpu, a, b, r);
            Ok(Some(r))
        }
        6 => {
            let r = a ^ b;
            set_logic_flags_u32(cpu, r);
            Ok(Some(r))
        }
        7 => {
            set_sub_flags_u32(cpu, a, b, a.wrapping_sub(b));
            Ok(None)
        }
        _ => Err(ExecError::Unsupported(0x81)),
    }
}

/// Real-mode default operand size is 16; 0x66 selects 32.
/// Spec: Intel SDM Vol. 2 Chapter 2; Vol. 1 §3.6.
fn opsz32(insn: &DecodedInsn) -> bool {
    insn.prefixes.op_size_override
}

/// Real-mode default address size is 16; 0x67 selects 32.
/// Spec: Intel SDM Vol. 1 §3.6; Vol. 2 Chapter 2 (address-size attribute).
fn asize32(insn: &DecodedInsn) -> bool {
    insn.prefixes.addr_size_override
}

/// Effective address from ModRM using the instruction address-size attribute.
/// Returns `(linear, is_register, uses_ss)` — `uses_ss` selects `#SS` vs `#GP`
/// when a bus `MemoryFault` is classified or a segment-limit fault is raised
/// (SDM Vol. 3 §5.3, §6.15).
/// Real-mode segmentation remains `selector << 4` (base + offset).
fn ea(
    cpu: &CpuState,
    insn: &DecodedInsn,
    access_size: u64,
) -> Result<(u64, bool, bool), ExecError> {
    if asize32(insn) {
        ea_32(cpu, insn, access_size)
    } else {
        ea_16(cpu, insn, access_size)
    }
}

/// 16-bit effective address from ModRM (real-mode / 16-bit address size).
fn ea_16(
    cpu: &CpuState,
    insn: &DecodedInsn,
    access_size: u64,
) -> Result<(u64, bool, bool), ExecError> {
    let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
    if m.mod_ == 3 {
        return Ok((0, true, false));
    }
    let off = u64::from(calc_ea16(cpu, m.mod_, m.rm, insn.displacement)?);
    let (seg, uses_ss) = match insn.prefixes.segment_override {
        Some(0x2E) => (&cpu.cs, false),
        Some(0x36) => (&cpu.ss, true),
        Some(0x26) => (&cpu.es, false),
        Some(0x64) => (&cpu.fs, false),
        Some(0x65) => (&cpu.gs, false),
        Some(0x3E) | None => {
            // Default DS, except BP-based uses SS.
            if m.rm == 2 || m.rm == 3 || (m.rm == 6 && m.mod_ != 0) {
                (&cpu.ss, true)
            } else {
                (&cpu.ds, false)
            }
        }
        _ => (&cpu.ds, false),
    };
    let addr = checked_linear_addr(seg, off, access_size)
        .map_err(|_| ExecError::ArchFault(if uses_ss { 12 } else { 13 }))?;
    Ok((addr, false, uses_ss))
}

/// Linear address for a data/stack access with cached segment-limit enforcement.
/// Spec: Intel SDM Vol. 3 §5.3; Vol. 2 MOV real-address `#GP`/`#SS`.
fn seg_linear_checked(
    seg: &x86_core::SegmentReg,
    offset: u64,
    size: u64,
    uses_ss: bool,
) -> Result<u64, ExecError> {
    checked_linear_addr(seg, offset, size)
        .map_err(|_| ExecError::ArchFault(if uses_ss { 12 } else { 13 }))
}

/// Absolute moffs offset from address-size attribute.
/// Spec: Intel SDM Vol. 2 MOV (moffs16 / moffs32).
fn moffs_offset(insn: &DecodedInsn) -> u64 {
    if insn.prefixes.addr_size_override {
        insn.immediate as u32 as u64
    } else {
        u64::from(insn.immediate as u16)
    }
}

/// 32-bit effective address from ModRM/SIB (real-mode with 0x67).
/// Spec: Intel SDM Vol. 1 §3.6; Vol. 2 Chapter 2 (32-bit addressing forms).
fn ea_32(
    cpu: &CpuState,
    insn: &DecodedInsn,
    access_size: u64,
) -> Result<(u64, bool, bool), ExecError> {
    let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
    if m.mod_ == 3 {
        return Ok((0, true, false));
    }
    let off = u64::from(calc_ea32(cpu, insn)?);
    let (seg, uses_ss) = match insn.prefixes.segment_override {
        Some(0x2E) => (&cpu.cs, false),
        Some(0x36) => (&cpu.ss, true),
        Some(0x26) => (&cpu.es, false),
        Some(0x64) => (&cpu.fs, false),
        Some(0x65) => (&cpu.gs, false),
        Some(0x3E) | None => {
            // Default DS; SS when base is EBP/ESP (incl. SIB base).
            let uses_ss = if m.rm == 4 {
                let sib = insn.sib.ok_or(ExecError::Unsupported(insn.opcode))?;
                let base = sib & 7;
                base == 4 || (base == 5 && m.mod_ != 0)
            } else {
                m.rm == 5 && m.mod_ != 0
            };
            if uses_ss {
                (&cpu.ss, true)
            } else {
                (&cpu.ds, false)
            }
        }
        _ => (&cpu.ds, false),
    };
    let addr = checked_linear_addr(seg, off, access_size)
        .map_err(|_| ExecError::ArchFault(if uses_ss { 12 } else { 13 }))?;
    Ok((addr, false, uses_ss))
}

/// Map a bus `MemoryFault` to `#SS` (vector 12) or `#GP` (vector 13).
/// Spec: Intel SDM Vol. 3 §6.15 (#SS / #GP).
fn classify_mem_fault(err: ExecError, uses_ss: bool) -> ExecError {
    match err {
        ExecError::MemoryFault(_) => ExecError::ArchFault(if uses_ss { 12 } else { 13 }),
        e => e,
    }
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

/// 32-bit ModRM/SIB effective address (offset only).
/// Spec: Intel SDM Vol. 2 Chapter 2 — 32-bit addressing forms + SIB.
fn calc_ea32(cpu: &CpuState, insn: &DecodedInsn) -> Result<u32, ExecError> {
    let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
    let disp = insn.displacement as u32;
    if m.rm == 4 {
        let sib = insn.sib.ok_or(ExecError::Unsupported(insn.opcode))?;
        let scale = 1u32 << (sib >> 6);
        let index = (sib >> 3) & 7;
        let base_reg = sib & 7;
        let index_val = if index == 4 {
            0
        } else {
            cpu.gpr_u32(index as usize).wrapping_mul(scale)
        };
        let base_val = if base_reg == 5 && m.mod_ == 0 {
            0
        } else {
            cpu.gpr_u32(base_reg as usize)
        };
        return Ok(base_val.wrapping_add(index_val).wrapping_add(disp));
    }
    let base = match (m.mod_, m.rm) {
        (0, 5) => return Ok(disp),
        (_, 0) => cpu.gpr_u32(CpuState::RAX),
        (_, 1) => cpu.gpr_u32(CpuState::RCX),
        (_, 2) => cpu.gpr_u32(CpuState::RDX),
        (_, 3) => cpu.gpr_u32(CpuState::RBX),
        (_, 5) => cpu.gpr_u32(CpuState::RBP),
        (_, 6) => cpu.gpr_u32(CpuState::RSI),
        (_, 7) => cpu.gpr_u32(CpuState::RDI),
        _ => return Err(ExecError::Unsupported(insn.opcode)),
    };
    Ok(match m.mod_ {
        0 => base,
        1 | 2 => base.wrapping_add(disp),
        _ => return Err(ExecError::Unsupported(insn.opcode)),
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
        let (addr, _, uses_ss) = ea(cpu, insn, 1)?;
        bus.read_u8(addr)
            .map_err(|e| classify_mem_fault(e, uses_ss))
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
        let (addr, _, uses_ss) = ea(cpu, insn, 1)?;
        bus.write_u8(addr, val)
            .map_err(|e| classify_mem_fault(e, uses_ss))
    }
}

fn read_rm_u16(cpu: &CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<u16, ExecError> {
    let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
    if m.mod_ == 3 {
        Ok(cpu.gpr_u16(m.rm as usize))
    } else {
        let (addr, _, uses_ss) = ea(cpu, insn, 2)?;
        bus.read_u16(addr)
            .map_err(|e| classify_mem_fault(e, uses_ss))
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
        let (addr, _, uses_ss) = ea(cpu, insn, 2)?;
        bus.write_u16(addr, val)
            .map_err(|e| classify_mem_fault(e, uses_ss))
    }
}

fn read_rm_u32(cpu: &CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<u32, ExecError> {
    let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
    if m.mod_ == 3 {
        Ok(cpu.gpr_u32(m.rm as usize))
    } else {
        let (addr, _, uses_ss) = ea(cpu, insn, 4)?;
        bus.read_u32(addr)
            .map_err(|e| classify_mem_fault(e, uses_ss))
    }
}

fn write_rm_u32(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    insn: &DecodedInsn,
    val: u32,
) -> Result<(), ExecError> {
    let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
    if m.mod_ == 3 {
        cpu.set_gpr_u32(m.rm as usize, val);
        Ok(())
    } else {
        let (addr, _, uses_ss) = ea(cpu, insn, 4)?;
        bus.write_u32(addr, val)
            .map_err(|e| classify_mem_fault(e, uses_ss))
    }
}

/// Stack push without `#SS` classification (used by IVT delivery itself).
fn push16_unchecked(cpu: &mut CpuState, bus: &mut dyn Bus, val: u16) -> Result<(), ExecError> {
    let old_sp = cpu.gpr_u16(CpuState::RSP);
    let sp = old_sp.wrapping_sub(2);
    cpu.set_gpr_u16(CpuState::RSP, sp);
    let addr = linear_addr(&cpu.ss, u64::from(sp));
    match bus.write_u16(addr, val) {
        Ok(()) => Ok(()),
        Err(e) => {
            cpu.set_gpr_u16(CpuState::RSP, old_sp);
            Err(e)
        }
    }
}

fn push16(cpu: &mut CpuState, bus: &mut dyn Bus, val: u16) -> Result<(), ExecError> {
    let old_sp = cpu.gpr_u16(CpuState::RSP);
    let sp = old_sp.wrapping_sub(2);
    // Limit check before mutating SP (SDM Vol. 3 §5.3 / §6.15 #SS).
    seg_linear_checked(&cpu.ss, u64::from(sp), 2, true)?;
    cpu.set_gpr_u16(CpuState::RSP, sp);
    let addr = linear_addr(&cpu.ss, u64::from(sp));
    match bus.write_u16(addr, val) {
        Ok(()) => Ok(()),
        Err(e) => {
            cpu.set_gpr_u16(CpuState::RSP, old_sp);
            Err(classify_mem_fault(e, true))
        }
    }
}

fn pop16(cpu: &mut CpuState, bus: &mut dyn Bus) -> Result<u16, ExecError> {
    let sp = cpu.gpr_u16(CpuState::RSP);
    let addr = seg_linear_checked(&cpu.ss, u64::from(sp), 2, true)?;
    let v = bus
        .read_u16(addr)
        .map_err(|e| classify_mem_fault(e, true))?;
    cpu.set_gpr_u16(CpuState::RSP, sp.wrapping_add(2));
    Ok(v)
}

/// PUSH with 32-bit operand size; address-size 16 still uses SP (decrement by 4).
/// Spec: Intel SDM Vol. 2 "PUSH"; Vol. 1 §3.6.
fn push32(cpu: &mut CpuState, bus: &mut dyn Bus, val: u32) -> Result<(), ExecError> {
    let old_sp = cpu.gpr_u16(CpuState::RSP);
    let sp = old_sp.wrapping_sub(4);
    seg_linear_checked(&cpu.ss, u64::from(sp), 4, true)?;
    cpu.set_gpr_u16(CpuState::RSP, sp);
    let addr = linear_addr(&cpu.ss, u64::from(sp));
    match bus.write_u32(addr, val) {
        Ok(()) => Ok(()),
        Err(e) => {
            cpu.set_gpr_u16(CpuState::RSP, old_sp);
            Err(classify_mem_fault(e, true))
        }
    }
}

fn pop32(cpu: &mut CpuState, bus: &mut dyn Bus) -> Result<u32, ExecError> {
    let sp = cpu.gpr_u16(CpuState::RSP);
    let addr = seg_linear_checked(&cpu.ss, u64::from(sp), 4, true)?;
    let v = bus
        .read_u32(addr)
        .map_err(|e| classify_mem_fault(e, true))?;
    cpu.set_gpr_u16(CpuState::RSP, sp.wrapping_add(4));
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
    // Caller must reject MOV CS and reserved Sreg encodings (#UD) before calling.
    // Sticky limit/AR: SDM Vol. 3 §3.4.2–§3.4.3 (unreal-mode descriptor cache).
    match sreg {
        0 => cpu.es.load_real_mode_selector(selector),
        2 => cpu.ss.load_real_mode_selector(selector),
        3 => cpu.ds.load_real_mode_selector(selector),
        4 => cpu.fs.load_real_mode_selector(selector),
        5 => cpu.gs.load_real_mode_selector(selector),
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

/// ESI/EDI step for address-size 32 string ops (SDM Vol. 1 §3.4.3 / §3.6).
fn string_index_delta32(cpu: &CpuState, size: u32) -> u32 {
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

/// SS override on string/moffs source → `#SS`; otherwise `#GP`.
/// Spec: Intel SDM Vol. 3 §6.15 (#SS / #GP).
fn string_src_uses_ss(insn: &DecodedInsn) -> bool {
    matches!(insn.prefixes.segment_override, Some(0x36))
}

fn map_string_src_fault(err: ExecError, insn: &DecodedInsn) -> ExecError {
    classify_mem_fault(err, string_src_uses_ss(insn))
}

/// ES: string destination / SCAS — not SS → `#GP`.
fn map_es_mem_fault(err: ExecError) -> ExecError {
    classify_mem_fault(err, false)
}

/// String source linear address with cached segment-limit check.
/// Spec: Intel SDM Vol. 3 §5.3; Vol. 2 MOVS/LODS/CMPS/OUTS.
fn string_src_linear(
    cpu: &CpuState,
    insn: &DecodedInsn,
    offset: u64,
    size: u64,
) -> Result<u64, ExecError> {
    seg_linear_checked(
        data_seg_for_string_src(cpu, insn),
        offset,
        size,
        string_src_uses_ss(insn),
    )
}

/// ES:(E)DI string destination / SCAS linear address with limit check → `#GP`.
/// Spec: Intel SDM Vol. 3 §5.3; Vol. 2 MOVS/STOS/SCAS/INS.
fn string_es_linear(cpu: &CpuState, offset: u64, size: u64) -> Result<u64, ExecError> {
    seg_linear_checked(&cpu.es, offset, size, false)
}

fn zf_set(cpu: &CpuState) -> bool {
    cpu.rflags & (1 << 6) != 0
}

/// One MOVSB iteration (no IP update). Spec: SDM Vol. 2 MOVS/MOVSB.
fn movsb_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    if asize32(insn) {
        let si = cpu.gpr_u32(CpuState::RSI);
        let di = cpu.gpr_u32(CpuState::RDI);
        let src = string_src_linear(cpu, insn, u64::from(si), 1)?;
        let dst = string_es_linear(cpu, u64::from(di), 1)?;
        let v = bus
            .read_u8(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        bus.write_u8(dst, v).map_err(map_es_mem_fault)?;
        let d = string_index_delta32(cpu, 1);
        cpu.set_gpr_u32(CpuState::RSI, si.wrapping_add(d));
        cpu.set_gpr_u32(CpuState::RDI, di.wrapping_add(d));
    } else {
        let si = cpu.gpr_u16(CpuState::RSI);
        let di = cpu.gpr_u16(CpuState::RDI);
        let src = string_src_linear(cpu, insn, u64::from(si), 1)?;
        let dst = string_es_linear(cpu, u64::from(di), 1)?;
        let v = bus
            .read_u8(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        bus.write_u8(dst, v).map_err(map_es_mem_fault)?;
        let d = string_index_delta(cpu, 1);
        cpu.set_gpr_u16(CpuState::RSI, si.wrapping_add(d));
        cpu.set_gpr_u16(CpuState::RDI, di.wrapping_add(d));
    }
    Ok(())
}

/// One STOSB iteration (no IP update). Spec: SDM Vol. 2 STOS/STOSB.
fn stosb_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    if asize32(insn) {
        let di = cpu.gpr_u32(CpuState::RDI);
        let dst = string_es_linear(cpu, u64::from(di), 1)?;
        bus.write_u8(dst, cpu.al()).map_err(map_es_mem_fault)?;
        let d = string_index_delta32(cpu, 1);
        cpu.set_gpr_u32(CpuState::RDI, di.wrapping_add(d));
    } else {
        let di = cpu.gpr_u16(CpuState::RDI);
        let dst = string_es_linear(cpu, u64::from(di), 1)?;
        bus.write_u8(dst, cpu.al()).map_err(map_es_mem_fault)?;
        let d = string_index_delta(cpu, 1);
        cpu.set_gpr_u16(CpuState::RDI, di.wrapping_add(d));
    }
    Ok(())
}

/// One LODSB iteration (no IP update). Spec: SDM Vol. 2 LODS/LODSB.
fn lodsb_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    if asize32(insn) {
        let si = cpu.gpr_u32(CpuState::RSI);
        let src = string_src_linear(cpu, insn, u64::from(si), 1)?;
        let v = bus
            .read_u8(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        cpu.set_al(v);
        let d = string_index_delta32(cpu, 1);
        cpu.set_gpr_u32(CpuState::RSI, si.wrapping_add(d));
    } else {
        let si = cpu.gpr_u16(CpuState::RSI);
        let src = string_src_linear(cpu, insn, u64::from(si), 1)?;
        let v = bus
            .read_u8(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        cpu.set_al(v);
        let d = string_index_delta(cpu, 1);
        cpu.set_gpr_u16(CpuState::RSI, si.wrapping_add(d));
    }
    Ok(())
}

/// One SCASB iteration (no IP update). Spec: SDM Vol. 2 SCAS/SCASB.
fn scasb_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    if asize32(insn) {
        let di = cpu.gpr_u32(CpuState::RDI);
        let addr = string_es_linear(cpu, u64::from(di), 1)?;
        let mem = bus.read_u8(addr).map_err(map_es_mem_fault)?;
        let al = cpu.al();
        let result = al.wrapping_sub(mem);
        set_sub_flags_u8(cpu, al, mem, result);
        let d = string_index_delta32(cpu, 1);
        cpu.set_gpr_u32(CpuState::RDI, di.wrapping_add(d));
    } else {
        let di = cpu.gpr_u16(CpuState::RDI);
        let addr = string_es_linear(cpu, u64::from(di), 1)?;
        let mem = bus.read_u8(addr).map_err(map_es_mem_fault)?;
        let al = cpu.al();
        let result = al.wrapping_sub(mem);
        set_sub_flags_u8(cpu, al, mem, result);
        let d = string_index_delta(cpu, 1);
        cpu.set_gpr_u16(CpuState::RDI, di.wrapping_add(d));
    }
    Ok(())
}

/// One CMPSB iteration (no IP update). Spec: SDM Vol. 2 CMPS/CMPSB.
fn cmpsb_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    if asize32(insn) {
        let si = cpu.gpr_u32(CpuState::RSI);
        let di = cpu.gpr_u32(CpuState::RDI);
        let src = string_src_linear(cpu, insn, u64::from(si), 1)?;
        let dst = string_es_linear(cpu, u64::from(di), 1)?;
        let a = bus
            .read_u8(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        let b = bus.read_u8(dst).map_err(map_es_mem_fault)?;
        let result = a.wrapping_sub(b);
        set_sub_flags_u8(cpu, a, b, result);
        let d = string_index_delta32(cpu, 1);
        cpu.set_gpr_u32(CpuState::RSI, si.wrapping_add(d));
        cpu.set_gpr_u32(CpuState::RDI, di.wrapping_add(d));
    } else {
        let si = cpu.gpr_u16(CpuState::RSI);
        let di = cpu.gpr_u16(CpuState::RDI);
        let src = string_src_linear(cpu, insn, u64::from(si), 1)?;
        let dst = string_es_linear(cpu, u64::from(di), 1)?;
        let a = bus
            .read_u8(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        let b = bus.read_u8(dst).map_err(map_es_mem_fault)?;
        let result = a.wrapping_sub(b);
        set_sub_flags_u8(cpu, a, b, result);
        let d = string_index_delta(cpu, 1);
        cpu.set_gpr_u16(CpuState::RSI, si.wrapping_add(d));
        cpu.set_gpr_u16(CpuState::RDI, di.wrapping_add(d));
    }
    Ok(())
}

/// One MOVSW iteration (no IP update). Spec: SDM Vol. 2 MOVS/MOVSW.
fn movsw_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    if asize32(insn) {
        let si = cpu.gpr_u32(CpuState::RSI);
        let di = cpu.gpr_u32(CpuState::RDI);
        let src = string_src_linear(cpu, insn, u64::from(si), 2)?;
        let dst = string_es_linear(cpu, u64::from(di), 2)?;
        let v = bus
            .read_u16(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        bus.write_u16(dst, v).map_err(map_es_mem_fault)?;
        let d = string_index_delta32(cpu, 2);
        cpu.set_gpr_u32(CpuState::RSI, si.wrapping_add(d));
        cpu.set_gpr_u32(CpuState::RDI, di.wrapping_add(d));
    } else {
        let si = cpu.gpr_u16(CpuState::RSI);
        let di = cpu.gpr_u16(CpuState::RDI);
        let src = string_src_linear(cpu, insn, u64::from(si), 2)?;
        let dst = string_es_linear(cpu, u64::from(di), 2)?;
        let v = bus
            .read_u16(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        bus.write_u16(dst, v).map_err(map_es_mem_fault)?;
        let d = string_index_delta(cpu, 2);
        cpu.set_gpr_u16(CpuState::RSI, si.wrapping_add(d));
        cpu.set_gpr_u16(CpuState::RDI, di.wrapping_add(d));
    }
    Ok(())
}

/// One MOVSD iteration (no IP update). Spec: SDM Vol. 2 MOVS/MOVSD (opsize 32).
fn movsd_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    if asize32(insn) {
        let si = cpu.gpr_u32(CpuState::RSI);
        let di = cpu.gpr_u32(CpuState::RDI);
        let src = string_src_linear(cpu, insn, u64::from(si), 4)?;
        let dst = string_es_linear(cpu, u64::from(di), 4)?;
        let v = bus
            .read_u32(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        bus.write_u32(dst, v).map_err(map_es_mem_fault)?;
        let d = string_index_delta32(cpu, 4);
        cpu.set_gpr_u32(CpuState::RSI, si.wrapping_add(d));
        cpu.set_gpr_u32(CpuState::RDI, di.wrapping_add(d));
    } else {
        let si = cpu.gpr_u16(CpuState::RSI);
        let di = cpu.gpr_u16(CpuState::RDI);
        let src = string_src_linear(cpu, insn, u64::from(si), 4)?;
        let dst = string_es_linear(cpu, u64::from(di), 4)?;
        let v = bus
            .read_u32(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        bus.write_u32(dst, v).map_err(map_es_mem_fault)?;
        let d = string_index_delta(cpu, 4);
        cpu.set_gpr_u16(CpuState::RSI, si.wrapping_add(d));
        cpu.set_gpr_u16(CpuState::RDI, di.wrapping_add(d));
    }
    Ok(())
}

/// One STOSW iteration (no IP update). Spec: SDM Vol. 2 STOS/STOSW.
fn stosw_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    if asize32(insn) {
        let di = cpu.gpr_u32(CpuState::RDI);
        let dst = string_es_linear(cpu, u64::from(di), 2)?;
        bus.write_u16(dst, cpu.ax()).map_err(map_es_mem_fault)?;
        let d = string_index_delta32(cpu, 2);
        cpu.set_gpr_u32(CpuState::RDI, di.wrapping_add(d));
    } else {
        let di = cpu.gpr_u16(CpuState::RDI);
        let dst = string_es_linear(cpu, u64::from(di), 2)?;
        bus.write_u16(dst, cpu.ax()).map_err(map_es_mem_fault)?;
        let d = string_index_delta(cpu, 2);
        cpu.set_gpr_u16(CpuState::RDI, di.wrapping_add(d));
    }
    Ok(())
}

/// One STOSD iteration (no IP update). Spec: SDM Vol. 2 STOS/STOSD (opsize 32).
fn stosd_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    if asize32(insn) {
        let di = cpu.gpr_u32(CpuState::RDI);
        let dst = string_es_linear(cpu, u64::from(di), 4)?;
        bus.write_u32(dst, cpu.eax()).map_err(map_es_mem_fault)?;
        let d = string_index_delta32(cpu, 4);
        cpu.set_gpr_u32(CpuState::RDI, di.wrapping_add(d));
    } else {
        let di = cpu.gpr_u16(CpuState::RDI);
        let dst = string_es_linear(cpu, u64::from(di), 4)?;
        bus.write_u32(dst, cpu.eax()).map_err(map_es_mem_fault)?;
        let d = string_index_delta(cpu, 4);
        cpu.set_gpr_u16(CpuState::RDI, di.wrapping_add(d));
    }
    Ok(())
}

/// One LODSW iteration (no IP update). Spec: SDM Vol. 2 LODS/LODSW.
fn lodsw_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    if asize32(insn) {
        let si = cpu.gpr_u32(CpuState::RSI);
        let src = string_src_linear(cpu, insn, u64::from(si), 2)?;
        let v = bus
            .read_u16(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        cpu.set_ax(v);
        let d = string_index_delta32(cpu, 2);
        cpu.set_gpr_u32(CpuState::RSI, si.wrapping_add(d));
    } else {
        let si = cpu.gpr_u16(CpuState::RSI);
        let src = string_src_linear(cpu, insn, u64::from(si), 2)?;
        let v = bus
            .read_u16(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        cpu.set_ax(v);
        let d = string_index_delta(cpu, 2);
        cpu.set_gpr_u16(CpuState::RSI, si.wrapping_add(d));
    }
    Ok(())
}

/// One LODSD iteration (no IP update). Spec: SDM Vol. 2 LODS/LODSD (opsize 32).
fn lodsd_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    if asize32(insn) {
        let si = cpu.gpr_u32(CpuState::RSI);
        let src = string_src_linear(cpu, insn, u64::from(si), 4)?;
        let v = bus
            .read_u32(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        cpu.set_eax(v);
        let d = string_index_delta32(cpu, 4);
        cpu.set_gpr_u32(CpuState::RSI, si.wrapping_add(d));
    } else {
        let si = cpu.gpr_u16(CpuState::RSI);
        let src = string_src_linear(cpu, insn, u64::from(si), 4)?;
        let v = bus
            .read_u32(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        cpu.set_eax(v);
        let d = string_index_delta(cpu, 4);
        cpu.set_gpr_u16(CpuState::RSI, si.wrapping_add(d));
    }
    Ok(())
}

/// One SCASW iteration (no IP update). Spec: SDM Vol. 2 SCAS/SCASW.
fn scasw_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    if asize32(insn) {
        let di = cpu.gpr_u32(CpuState::RDI);
        let addr = string_es_linear(cpu, u64::from(di), 2)?;
        let mem = bus.read_u16(addr).map_err(map_es_mem_fault)?;
        let ax = cpu.ax();
        let result = ax.wrapping_sub(mem);
        set_sub_flags_u16(cpu, ax, mem, result);
        let d = string_index_delta32(cpu, 2);
        cpu.set_gpr_u32(CpuState::RDI, di.wrapping_add(d));
    } else {
        let di = cpu.gpr_u16(CpuState::RDI);
        let addr = string_es_linear(cpu, u64::from(di), 2)?;
        let mem = bus.read_u16(addr).map_err(map_es_mem_fault)?;
        let ax = cpu.ax();
        let result = ax.wrapping_sub(mem);
        set_sub_flags_u16(cpu, ax, mem, result);
        let d = string_index_delta(cpu, 2);
        cpu.set_gpr_u16(CpuState::RDI, di.wrapping_add(d));
    }
    Ok(())
}

/// One SCASD iteration (no IP update). Spec: SDM Vol. 2 SCAS/SCASD (opsize 32).
fn scasd_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    if asize32(insn) {
        let di = cpu.gpr_u32(CpuState::RDI);
        let addr = string_es_linear(cpu, u64::from(di), 4)?;
        let mem = bus.read_u32(addr).map_err(map_es_mem_fault)?;
        let eax = cpu.eax();
        let result = eax.wrapping_sub(mem);
        set_sub_flags_u32(cpu, eax, mem, result);
        let d = string_index_delta32(cpu, 4);
        cpu.set_gpr_u32(CpuState::RDI, di.wrapping_add(d));
    } else {
        let di = cpu.gpr_u16(CpuState::RDI);
        let addr = string_es_linear(cpu, u64::from(di), 4)?;
        let mem = bus.read_u32(addr).map_err(map_es_mem_fault)?;
        let eax = cpu.eax();
        let result = eax.wrapping_sub(mem);
        set_sub_flags_u32(cpu, eax, mem, result);
        let d = string_index_delta(cpu, 4);
        cpu.set_gpr_u16(CpuState::RDI, di.wrapping_add(d));
    }
    Ok(())
}

/// One CMPSW iteration (no IP update). Spec: SDM Vol. 2 CMPS/CMPSW.
fn cmpsw_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    if asize32(insn) {
        let si = cpu.gpr_u32(CpuState::RSI);
        let di = cpu.gpr_u32(CpuState::RDI);
        let src = string_src_linear(cpu, insn, u64::from(si), 2)?;
        let dst = string_es_linear(cpu, u64::from(di), 2)?;
        let a = bus
            .read_u16(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        let b = bus.read_u16(dst).map_err(map_es_mem_fault)?;
        let result = a.wrapping_sub(b);
        set_sub_flags_u16(cpu, a, b, result);
        let d = string_index_delta32(cpu, 2);
        cpu.set_gpr_u32(CpuState::RSI, si.wrapping_add(d));
        cpu.set_gpr_u32(CpuState::RDI, di.wrapping_add(d));
    } else {
        let si = cpu.gpr_u16(CpuState::RSI);
        let di = cpu.gpr_u16(CpuState::RDI);
        let src = string_src_linear(cpu, insn, u64::from(si), 2)?;
        let dst = string_es_linear(cpu, u64::from(di), 2)?;
        let a = bus
            .read_u16(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        let b = bus.read_u16(dst).map_err(map_es_mem_fault)?;
        let result = a.wrapping_sub(b);
        set_sub_flags_u16(cpu, a, b, result);
        let d = string_index_delta(cpu, 2);
        cpu.set_gpr_u16(CpuState::RSI, si.wrapping_add(d));
        cpu.set_gpr_u16(CpuState::RDI, di.wrapping_add(d));
    }
    Ok(())
}

/// One CMPSD iteration (no IP update). Spec: SDM Vol. 2 CMPS/CMPSD (opsize 32).
fn cmpsd_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    if asize32(insn) {
        let si = cpu.gpr_u32(CpuState::RSI);
        let di = cpu.gpr_u32(CpuState::RDI);
        let src = string_src_linear(cpu, insn, u64::from(si), 4)?;
        let dst = string_es_linear(cpu, u64::from(di), 4)?;
        let a = bus
            .read_u32(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        let b = bus.read_u32(dst).map_err(map_es_mem_fault)?;
        let result = a.wrapping_sub(b);
        set_sub_flags_u32(cpu, a, b, result);
        let d = string_index_delta32(cpu, 4);
        cpu.set_gpr_u32(CpuState::RSI, si.wrapping_add(d));
        cpu.set_gpr_u32(CpuState::RDI, di.wrapping_add(d));
    } else {
        let si = cpu.gpr_u16(CpuState::RSI);
        let di = cpu.gpr_u16(CpuState::RDI);
        let src = string_src_linear(cpu, insn, u64::from(si), 4)?;
        let dst = string_es_linear(cpu, u64::from(di), 4)?;
        let a = bus
            .read_u32(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        let b = bus.read_u32(dst).map_err(map_es_mem_fault)?;
        let result = a.wrapping_sub(b);
        set_sub_flags_u32(cpu, a, b, result);
        let d = string_index_delta(cpu, 4);
        cpu.set_gpr_u16(CpuState::RSI, si.wrapping_add(d));
        cpu.set_gpr_u16(CpuState::RDI, di.wrapping_add(d));
    }
    Ok(())
}

/// One INSB iteration (no IP update). Spec: SDM Vol. 2 INS/INSB/INSW/INSD.
/// Port = DX; destination = ES:(E)DI (no segment override for dest).
fn insb_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    let port = cpu.gpr_u16(CpuState::RDX);
    if asize32(insn) {
        let di = cpu.gpr_u32(CpuState::RDI);
        let dst = string_es_linear(cpu, u64::from(di), 1)?;
        let v = bus.port_in_u8(port)?;
        bus.write_u8(dst, v).map_err(map_es_mem_fault)?;
        let d = string_index_delta32(cpu, 1);
        cpu.set_gpr_u32(CpuState::RDI, di.wrapping_add(d));
    } else {
        let di = cpu.gpr_u16(CpuState::RDI);
        let dst = string_es_linear(cpu, u64::from(di), 1)?;
        let v = bus.port_in_u8(port)?;
        bus.write_u8(dst, v).map_err(map_es_mem_fault)?;
        let d = string_index_delta(cpu, 1);
        cpu.set_gpr_u16(CpuState::RDI, di.wrapping_add(d));
    }
    Ok(())
}

/// One INSW iteration (no IP update). Spec: SDM Vol. 2 INS/INSW.
fn insw_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    let port = cpu.gpr_u16(CpuState::RDX);
    if asize32(insn) {
        let di = cpu.gpr_u32(CpuState::RDI);
        let dst = string_es_linear(cpu, u64::from(di), 2)?;
        let v = bus.port_in_u16(port)?;
        bus.write_u16(dst, v).map_err(map_es_mem_fault)?;
        let d = string_index_delta32(cpu, 2);
        cpu.set_gpr_u32(CpuState::RDI, di.wrapping_add(d));
    } else {
        let di = cpu.gpr_u16(CpuState::RDI);
        let dst = string_es_linear(cpu, u64::from(di), 2)?;
        let v = bus.port_in_u16(port)?;
        bus.write_u16(dst, v).map_err(map_es_mem_fault)?;
        let d = string_index_delta(cpu, 2);
        cpu.set_gpr_u16(CpuState::RDI, di.wrapping_add(d));
    }
    Ok(())
}

/// One INSD iteration (no IP update). Spec: SDM Vol. 2 INS/INSD (opsize 32).
fn insd_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    let port = cpu.gpr_u16(CpuState::RDX);
    if asize32(insn) {
        let di = cpu.gpr_u32(CpuState::RDI);
        let dst = string_es_linear(cpu, u64::from(di), 4)?;
        let v = bus.port_in_u32(port)?;
        bus.write_u32(dst, v).map_err(map_es_mem_fault)?;
        let d = string_index_delta32(cpu, 4);
        cpu.set_gpr_u32(CpuState::RDI, di.wrapping_add(d));
    } else {
        let di = cpu.gpr_u16(CpuState::RDI);
        let dst = string_es_linear(cpu, u64::from(di), 4)?;
        let v = bus.port_in_u32(port)?;
        bus.write_u32(dst, v).map_err(map_es_mem_fault)?;
        let d = string_index_delta(cpu, 4);
        cpu.set_gpr_u16(CpuState::RDI, di.wrapping_add(d));
    }
    Ok(())
}

/// One OUTSB iteration (no IP update). Spec: SDM Vol. 2 OUTS/OUTSB/OUTSW/OUTSD.
/// Port = DX; source = DS:(E)SI (segment override allowed).
fn outsb_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    let port = cpu.gpr_u16(CpuState::RDX);
    if asize32(insn) {
        let si = cpu.gpr_u32(CpuState::RSI);
        let src = string_src_linear(cpu, insn, u64::from(si), 1)?;
        let v = bus
            .read_u8(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        bus.port_out_u8(port, v)?;
        let d = string_index_delta32(cpu, 1);
        cpu.set_gpr_u32(CpuState::RSI, si.wrapping_add(d));
    } else {
        let si = cpu.gpr_u16(CpuState::RSI);
        let src = string_src_linear(cpu, insn, u64::from(si), 1)?;
        let v = bus
            .read_u8(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        bus.port_out_u8(port, v)?;
        let d = string_index_delta(cpu, 1);
        cpu.set_gpr_u16(CpuState::RSI, si.wrapping_add(d));
    }
    Ok(())
}

/// One OUTSW iteration (no IP update). Spec: SDM Vol. 2 OUTS/OUTSW.
fn outsw_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    let port = cpu.gpr_u16(CpuState::RDX);
    if asize32(insn) {
        let si = cpu.gpr_u32(CpuState::RSI);
        let src = string_src_linear(cpu, insn, u64::from(si), 2)?;
        let v = bus
            .read_u16(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        bus.port_out_u16(port, v)?;
        let d = string_index_delta32(cpu, 2);
        cpu.set_gpr_u32(CpuState::RSI, si.wrapping_add(d));
    } else {
        let si = cpu.gpr_u16(CpuState::RSI);
        let src = string_src_linear(cpu, insn, u64::from(si), 2)?;
        let v = bus
            .read_u16(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        bus.port_out_u16(port, v)?;
        let d = string_index_delta(cpu, 2);
        cpu.set_gpr_u16(CpuState::RSI, si.wrapping_add(d));
    }
    Ok(())
}

/// One OUTSD iteration (no IP update). Spec: SDM Vol. 2 OUTS/OUTSD (opsize 32).
fn outsd_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    let port = cpu.gpr_u16(CpuState::RDX);
    if asize32(insn) {
        let si = cpu.gpr_u32(CpuState::RSI);
        let src = string_src_linear(cpu, insn, u64::from(si), 4)?;
        let v = bus
            .read_u32(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        bus.port_out_u32(port, v)?;
        let d = string_index_delta32(cpu, 4);
        cpu.set_gpr_u32(CpuState::RSI, si.wrapping_add(d));
    } else {
        let si = cpu.gpr_u16(CpuState::RSI);
        let src = string_src_linear(cpu, insn, u64::from(si), 4)?;
        let v = bus
            .read_u32(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        bus.port_out_u32(port, v)?;
        let d = string_index_delta(cpu, 4);
        cpu.set_gpr_u16(CpuState::RSI, si.wrapping_add(d));
    }
    Ok(())
}

/// Real-mode `#NMI` vector (Intel SDM Vol. 3 §6.3.3 / §6.15).
const VECTOR_NMI: u8 = 2;

/// Service a latched platform `#NMI` if pending.
///
/// Not gated by `RFLAGS.IF`. Clears `halted` so NMI can wake `HLT`.
/// Spec: Intel SDM Vol. 3 §6.3.3, §6.7 (NMI); §6.4 (real-address delivery).
/// Stub: no SMRAM/SMI, no NMI blocking window after delivery.
fn service_pending_nmi(cpu: &mut CpuState, bus: &mut dyn Bus) -> Result<bool, ExecError> {
    if !cpu.pending_nmi {
        return Ok(false);
    }
    cpu.pending_nmi = false;
    cpu.halted = false;
    real_mode_software_interrupt(cpu, bus, VECTOR_NMI, cpu.ip16())?;
    Ok(true)
}

/// Service a latched maskable external IRQ if `IF=1`.
///
/// Pulls [`Bus::poll_external_irq`] into [`CpuState::pending_irq`], then
/// delivers via the real-mode IVT when enabled. Return IP is the current
/// instruction start (REP string ops leave IP unadvanced until completion).
///
/// Spec: Intel SDM Vol. 2 "REP/REPE/REPNE" (service pending interrupts between
/// iterations); Vol. 3 §6.8.1 (maskable interrupts when IF=1).
/// Stub: not a full 8259 — no priority / IRR / EOI.
fn service_pending_external_interrupt(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
) -> Result<bool, ExecError> {
    if let Some(vector) = bus.poll_external_irq() {
        cpu.request_interrupt(vector);
    }
    if !cpu.interrupt_flag() {
        return Ok(false);
    }
    let Some(vector) = cpu.pending_irq.take() else {
        return Ok(false);
    };
    real_mode_software_interrupt(cpu, bus, vector, cpu.ip16())?;
    Ok(true)
}

/// REP / REPE / REPNE wrapper — count = CX (asize16) or ECX (asize32).
///
/// Spec: Intel SDM Vol. 2 "REP/REPE/REPNE/REPZ/REPNZ"; Vol. 1 §3.6.
/// - `zf_terminate`: `None` = unconditional REP (MOVS/STOS/LODS);
///   `Some(true)` = REPE (stop when ZF=0 after an iteration);
///   `Some(false)` = REPNE (stop when ZF=1 after an iteration).
/// - Returns `Ok(true)` if a maskable external interrupt suspended the repeat
///   (IP already at the handler; CX/SI/DI preserved for resume).
///
/// Unsupported here: asize 64 (RCX). Per-instruction IRQ poll is in [`step`].
fn exec_string_with_rep<F>(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    insn: &DecodedInsn,
    zf_terminate: Option<bool>,
    mut once: F,
) -> Result<bool, ExecError>
where
    F: FnMut(&mut CpuState, &mut dyn Bus, &DecodedInsn) -> Result<(), ExecError>,
{
    let use_rep = insn.prefixes.rep || insn.prefixes.repne;
    if !use_rep {
        once(cpu, bus, insn)?;
        return Ok(false);
    }

    let use_ecx = asize32(insn);
    loop {
        if use_ecx {
            let ecx = cpu.gpr_u32(CpuState::RCX);
            if ecx == 0 {
                break;
            }
        } else {
            let cx = cpu.gpr_u16(CpuState::RCX);
            if cx == 0 {
                break;
            }
        }
        // SDM: service pending interrupts before each string iteration.
        // `#NMI` outranks maskable IRQs (Vol. 3 §6.7).
        if service_pending_nmi(cpu, bus)? {
            return Ok(true);
        }
        if service_pending_external_interrupt(cpu, bus)? {
            return Ok(true);
        }
        if use_ecx {
            let ecx = cpu.gpr_u32(CpuState::RCX);
            once(cpu, bus, insn)?;
            cpu.set_gpr_u32(CpuState::RCX, ecx.wrapping_sub(1));
        } else {
            let cx = cpu.gpr_u16(CpuState::RCX);
            once(cpu, bus, insn)?;
            cpu.set_gpr_u16(CpuState::RCX, cx.wrapping_sub(1));
        }
        if let Some(continue_while_zf) = zf_terminate {
            // REPE (`true`): stop when ZF=0. REPNE (`false`): stop when ZF=1.
            let zf = zf_set(cpu);
            if continue_while_zf != zf {
                break;
            }
        }
    }
    Ok(false)
}

/// Run a (possibly repeated) string op; advance IP only if not IRQ-suspended.
fn exec_string_op<F>(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    insn: &DecodedInsn,
    next_ip: u16,
    zf_terminate: Option<bool>,
    once: F,
) -> Result<(), ExecError>
where
    F: FnMut(&mut CpuState, &mut dyn Bus, &DecodedInsn) -> Result<(), ExecError>,
{
    if exec_string_with_rep(cpu, bus, insn, zf_terminate, once)? {
        return Ok(());
    }
    cpu.set_ip16(next_ip);
    Ok(())
}

/// Read far pointer `m16:16` (offset then selector) for LES/LDS.
/// Spec: Intel SDM Vol. 2 LES/LDS — memory operand only (mod=11 is #UD).
fn read_far_ptr16(
    cpu: &CpuState,
    bus: &mut dyn Bus,
    insn: &DecodedInsn,
) -> Result<(u16, u16), ExecError> {
    let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
    if m.mod_ == 3 {
        // Caller should deliver #UD; keep helper defensive.
        return Err(ExecError::Unsupported(insn.opcode));
    }
    let (addr, _, uses_ss) = ea(cpu, insn, 4)?;
    let offset = bus
        .read_u16(addr)
        .map_err(|e| classify_mem_fault(e, uses_ss))?;
    let selector = bus
        .read_u16(addr.wrapping_add(2))
        .map_err(|e| classify_mem_fault(e, uses_ss))?;
    Ok((offset, selector))
}

/// Read far pointer `m16:32` (offset32 then selector16) for LES/LDS opsize-32.
/// Spec: Intel SDM Vol. 2 LES/LDS; Ch. 2 (66H).
fn read_far_ptr32(
    cpu: &CpuState,
    bus: &mut dyn Bus,
    insn: &DecodedInsn,
) -> Result<(u32, u16), ExecError> {
    let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
    if m.mod_ == 3 {
        return Err(ExecError::Unsupported(insn.opcode));
    }
    let (addr, _, uses_ss) = ea(cpu, insn, 6)?;
    let offset = bus
        .read_u32(addr)
        .map_err(|e| classify_mem_fault(e, uses_ss))?;
    let selector = bus
        .read_u16(addr.wrapping_add(4))
        .map_err(|e| classify_mem_fault(e, uses_ss))?;
    Ok((offset, selector))
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

fn set_shift_result_flags_u32(cpu: &mut CpuState, result: u32) {
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x8000_0000 != 0);
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
        6 => Err(ExecError::Unsupported(0xD0)), // reserved; callers deliver #UD
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

/// Group 2 dword ops (D1/C1/D3 under OsZ32). Spec: SDM Vol. 2 ROL/ROR/RCL/RCR/SHL/SHR/SAR.
/// COUNT masked to 5 bits; RCL/RCR use COUNT mod 33.
fn grp2_u32(cpu: &mut CpuState, reg: u8, mut val: u32, raw_count: u8) -> Result<u32, ExecError> {
    let count = raw_count & 0x1F;
    if count == 0 {
        return Ok(val);
    }
    match reg {
        0 => {
            let n = count % 32;
            if n != 0 {
                val = val.rotate_left(u32::from(n));
            }
            let new_cf = (val & 1) != 0;
            cpu.set_cf(new_cf);
            if count == 1 {
                cpu.set_of(((val & 0x8000_0000) != 0) ^ new_cf);
            }
            Ok(val)
        }
        1 => {
            let n = count % 32;
            if n != 0 {
                val = val.rotate_right(u32::from(n));
            }
            let new_cf = (val & 0x8000_0000) != 0;
            cpu.set_cf(new_cf);
            if count == 1 {
                cpu.set_of(((val ^ (val << 1)) & 0x8000_0000) != 0);
            }
            Ok(val)
        }
        2 => {
            let n = count % 33;
            for _ in 0..n {
                let new_cf = (val & 0x8000_0000) != 0;
                val = (val << 1) | u32::from(cpu.rflags & 1 != 0);
                cpu.set_cf(new_cf);
            }
            if count == 1 {
                let cf = cpu.rflags & 1 != 0;
                cpu.set_of(((val & 0x8000_0000) != 0) ^ cf);
            }
            Ok(val)
        }
        3 => {
            let n = count % 33;
            for _ in 0..n {
                let new_cf = (val & 1) != 0;
                val = (val >> 1) | (u32::from(cpu.rflags & 1 != 0) << 31);
                cpu.set_cf(new_cf);
            }
            if count == 1 {
                cpu.set_of(((val ^ (val << 1)) & 0x8000_0000) != 0);
            }
            Ok(val)
        }
        4 => {
            for _ in 0..count {
                cpu.set_cf((val & 0x8000_0000) != 0);
                val <<= 1;
            }
            if count == 1 {
                let cf = cpu.rflags & 1 != 0;
                cpu.set_of(((val & 0x8000_0000) != 0) ^ cf);
            }
            set_shift_result_flags_u32(cpu, val);
            Ok(val)
        }
        5 => {
            let orig = val;
            for _ in 0..count {
                cpu.set_cf((val & 1) != 0);
                val >>= 1;
            }
            if count == 1 {
                cpu.set_of((orig & 0x8000_0000) != 0);
            }
            set_shift_result_flags_u32(cpu, val);
            Ok(val)
        }
        6 => Err(ExecError::Unsupported(0xD1)),
        7 => {
            for _ in 0..count {
                cpu.set_cf((val & 1) != 0);
                val = ((val as i32) >> 1) as u32;
            }
            if count == 1 {
                cpu.set_of(false);
            }
            set_shift_result_flags_u32(cpu, val);
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

/// Primary opcodes that are architectural `#UD` in real-address mode when the
/// sparse decoder table has no entry (or rejects the opcode).
///
/// **Rule (sparse tables):** do **not** treat every `UnsupportedOpcode` as `#UD`.
/// Only opcodes the SDM classifies as invalid/unrecognized in real mode vector
/// through the IVT. Valid-but-unimplemented primaries (x87 `D8`–`DF`, `WAIT`/`9B`,
/// `IN`/`OUT` EAX forms `E5`/`E7`/`ED`/`EF`, two-byte escape `0F`, Grp1 alias `82`,
/// …) remain host `Decode(UnsupportedOpcode)`.
///
/// Note: `D6` and `F1` are reserved/undefined but do **not** generate `#UD`
/// (Intel SDM Vol. 3 §6.15 — Invalid Opcode Exception).
///
/// Spec: Intel SDM Vol. 3 §6.15 (#UD); Vol. 2 ARPL (Real-Address Mode Exceptions).
fn real_mode_primary_opcode_is_ud(opcode: u8) -> bool {
    matches!(opcode, 0x63) // ARPL — not recognized in real-address mode
}

fn fetch_decode(cpu: &CpuState, bus: &mut dyn Bus) -> Result<x86_decode::DecodedInsn, ExecError> {
    // Grow the window until decode succeeds or we hit the 15-byte SDM limit.
    let mut buf = Vec::with_capacity(15);
    loop {
        if buf.len() >= 15 {
            return Err(ExecError::Decode(DecodeError::TooLong));
        }
        // Real-mode fetch still uses IP low 16 bits; enforce cached CS.limit.
        // Spec: Intel SDM Vol. 3 §5.3; §6.15 (#GP). Bus MemoryFault → #GP (CS).
        let ip = u64::from(cpu.ip16()).wrapping_add(buf.len() as u64) & 0xFFFF;
        let addr = seg_linear_checked(&cpu.cs, ip, 1, false)?;
        buf.push(
            bus.read_u8(addr)
                .map_err(|e| classify_mem_fault(e, false))?,
        );
        match decode(&buf) {
            Ok(insn) => return Ok(insn),
            Err(DecodeError::Truncated) => continue,
            Err(DecodeError::UnsupportedOpcode(op)) if real_mode_primary_opcode_is_ud(op) => {
                return Err(ExecError::ArchFault(6));
            }
            Err(e) => return Err(ExecError::Decode(e)),
        }
    }
}

/// Real-mode software interrupt delivery through the IVT at `IDTR.base`.
///
/// Uses unchecked stack pushes so a delivery-time bus fault stays `MemoryFault`
/// (not re-classified as a nested `#SS` ArchFault). Spec: Intel SDM Vol. 2
/// "INT n/INTO/INT3/INT1"; Vol. 3 §6.4.
fn real_mode_software_interrupt(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    vector: u8,
    return_ip: u16,
) -> Result<(), ExecError> {
    let flags16 = cpu.rflags as u16;
    push16_unchecked(cpu, bus, flags16)?;
    push16_unchecked(cpu, bus, cpu.cs.selector)?;
    push16_unchecked(cpu, bus, return_ip)?;
    // Clear IF and TF (Vol. 2 INT n Operation, real-address mode).
    cpu.rflags &= !((1 << 9) | (1 << 8));
    let entry = cpu.idtr.base.wrapping_add(u64::from(vector) * 4);
    let offset = bus.read_u16(entry)?;
    let selector = bus.read_u16(entry.wrapping_add(2))?;
    cpu.cs = x86_core::SegmentReg::real_mode_code(selector);
    cpu.set_ip16(offset);
    Ok(())
}

/// Real-mode exception fault delivery (#DE, #UD, #BR, #SS, #GP, …) through the IVT.
///
/// Saved IP is the faulting instruction address (instruction start).
/// Spec: Intel SDM Vol. 3 §6.4 (real-address mode), §6.15 (exception reference).
/// Note: #OF from INTO is a trap (use [`real_mode_software_interrupt`] with next IP).
fn real_mode_exception(cpu: &mut CpuState, bus: &mut dyn Bus, vector: u8) -> Result<(), ExecError> {
    real_mode_software_interrupt(cpu, bus, vector, cpu.ip16())
}

/// #UD — Invalid Opcode Exception (vector 6).
/// Spec: Intel SDM Vol. 3 §6.15 (#UD).
fn real_mode_ud(cpu: &mut CpuState, bus: &mut dyn Bus) -> Result<(), ExecError> {
    real_mode_exception(cpu, bus, 6)
}

/// Load/store GDTR/IDTR pseudo-descriptor `m16&32` (limit16 + base32).
/// Spec: Intel SDM Vol. 2 "LGDT/SGDT" / "LIDT/SIDT"; Vol. 3 §2.4.1 / §2.4.3.
///
/// Operand-size 16: base uses bits 23:0 (bits 31:24 ignored on load; stored 0 on store).
/// Operand-size 32 (`0x66`): full 32-bit base. Memory form only (mod=11 → `#UD`).
fn dtr_pseudo_desc(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    insn: &DecodedInsn,
    load: bool,
    idtr: bool,
) -> Result<(), ExecError> {
    let m = insn.modrm.ok_or(ExecError::Unsupported(0x01))?;
    if m.mod_ == 3 {
        // Spec: SDM Vol. 2 LGDT/SGDT / LIDT/SIDT — register form #UD
        return Err(ExecError::ArchFault(6));
    }
    let (addr, _, uses_ss) = ea(cpu, insn, 6)?;
    let dtr = if idtr { &mut cpu.idtr } else { &mut cpu.gdtr };
    if load {
        let limit = bus
            .read_u16(addr)
            .map_err(|e| classify_mem_fault(e, uses_ss))?;
        let mut base = u64::from(
            bus.read_u32(addr.wrapping_add(2))
                .map_err(|e| classify_mem_fault(e, uses_ss))?,
        );
        if !opsz32(insn) {
            // Spec: SDM Vol. 2 LGDT/LIDT — 16-bit operand-size uses 24-bit base.
            base &= 0x00FF_FFFF;
        }
        dtr.limit = limit;
        dtr.base = base;
    } else {
        bus.write_u16(addr, dtr.limit)
            .map_err(|e| classify_mem_fault(e, uses_ss))?;
        let mut base = dtr.base as u32;
        if !opsz32(insn) {
            // Spec: SDM Vol. 2 SGDT/SIDT — 16-bit operand-size stores base[31:24]=0.
            base &= 0x00FF_FFFF;
        }
        bus.write_u32(addr.wrapping_add(2), base)
            .map_err(|e| classify_mem_fault(e, uses_ss))?;
    }
    Ok(())
}

/// Two-byte opcode map (0F xx). Spec: Intel SDM Vol. 2 Chapter 2; "LGDT"/"SGDT"/"IMUL".
fn step_two_byte(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    insn: &DecodedInsn,
    next_ip: u16,
) -> Result<(), ExecError> {
    match insn.opcode {
        0x06 => {
            // CLTS — Spec: Intel SDM Vol. 2 "CLTS—Clear Task-Switched Flag in
            // CR0"; Vol. 3 §2.5 (CR0.TS = bit 3). Clears TS only; all other
            // CR0 bits (including PE) are unchanged. Real-mode path only —
            // protected-mode CPL=0 / #GP(0) checks are out of scope here.
            cpu.cr0 &= !(1u64 << 3);
            cpu.set_ip16(next_ip);
            Ok(())
        }
        0x01 => {
            // Group 7 — Spec: Intel SDM Vol. 2 opcode map 2;
            // "SGDT"/"SIDT"/"LGDT"/"LIDT"/"SMSW"/"LMSW"/"INVLPG".
            // Unsupported here: /5 (extensions); protected-mode entry from PE;
            // paging/TLB invalidate side effects (real-mode INVLPG is a NOP).
            let m = insn.modrm.ok_or(ExecError::Unsupported(0x01))?;
            match m.reg {
                0 => {
                    // SGDT m16&32
                    dtr_pseudo_desc(cpu, bus, insn, false, false)?;
                    cpu.set_ip16(next_ip);
                    Ok(())
                }
                1 => {
                    // SIDT m16&32
                    dtr_pseudo_desc(cpu, bus, insn, false, true)?;
                    cpu.set_ip16(next_ip);
                    Ok(())
                }
                2 => {
                    // LGDT m16&32
                    dtr_pseudo_desc(cpu, bus, insn, true, false)?;
                    cpu.set_ip16(next_ip);
                    Ok(())
                }
                3 => {
                    // LIDT m16&32
                    dtr_pseudo_desc(cpu, bus, insn, true, true)?;
                    cpu.set_ip16(next_ip);
                    Ok(())
                }
                4 => {
                    // SMSW r/m16 — Spec: SDM Vol. 2 "SMSW"; stores CR0[15:0].
                    // Memory destination is always 16-bit; register + opsize32
                    // zero-extends into r32 (deterministic; upper bits undefined in SDM).
                    let msw = cpu.cr0 as u16;
                    if m.mod_ == 3 && opsz32(insn) {
                        cpu.set_gpr_u32(m.rm as usize, u32::from(msw));
                    } else {
                        write_rm_u16(cpu, bus, insn, msw)?;
                    }
                    cpu.set_ip16(next_ip);
                    Ok(())
                }
                6 => {
                    // LMSW r/m16 — Spec: SDM Vol. 2 "LMSW"; Vol. 3 §2.5 (CR0.PE).
                    // Loads CR0[15:0]. Cannot clear PE once set. Setting PE here
                    // is sticky in CR0 only — this emulator does **not** switch
                    // to protected-mode descriptor loads (segment MOV / far JMP
                    // stay real-mode / sticky-unreal `selector << 4`).
                    let src = read_rm_u16(cpu, bus, insn)?;
                    let pe_was = cpu.cr0 & 1 != 0;
                    let mut low = u64::from(src);
                    if pe_was {
                        low |= 1; // Spec: LMSW cannot clear PE
                    }
                    cpu.cr0 = (cpu.cr0 & !0xFFFF) | low;
                    cpu.set_ip16(next_ip);
                    Ok(())
                }
                7 => {
                    // INVLPG m — Spec: Intel SDM Vol. 2 "INVLPG—Invalidate TLB
                    // Entries". Register form (mod=11) → #UD. In real-address
                    // mode the instruction is an architectural NOP (no TLB /
                    // paging here); GPRs and CR0 are unchanged.
                    if m.mod_ == 3 {
                        return Err(ExecError::ArchFault(6));
                    }
                    cpu.set_ip16(next_ip);
                    Ok(())
                }
                _ => Err(ExecError::Unsupported(0x01)),
            }
        }
        0x20 => {
            // MOV r32, CR0 — Spec: Intel SDM Vol. 2 "MOV—Move to/from Control
            // Registers"; Vol. 3 §2.5 (CR0). ModRM.reg selects the control
            // register; the mod field is architecturally ignored (decoder
            // never populates SIB/displacement for this opcode). Operand
            // size is always 32 bits regardless of any 0x66 prefix.
            // CR1 → #UD. CR2/CR3/CR4 → explicit Unsupported (out of scope).
            let m = insn.modrm.ok_or(ExecError::Unsupported(0x20))?;
            match m.reg {
                0 => {
                    cpu.set_gpr_u32(m.rm as usize, cpu.cr0 as u32);
                    cpu.set_ip16(next_ip);
                    Ok(())
                }
                1 => real_mode_ud(cpu, bus),
                2..=4 => Err(ExecError::Unsupported(0x20)),
                _ => real_mode_ud(cpu, bus),
            }
        }
        0x22 => {
            // MOV CR0, r32 — Spec: Intel SDM Vol. 2 "MOV—Move to/from Control
            // Registers"; Vol. 3 §2.5 (CR0). Unlike LMSW, this instruction
            // MAY clear PE. Setting/clearing PE here does not switch this
            // emulator's segment execution model in or out of protected
            // mode (segment loads keep using real-mode / sticky-unreal
            // `selector << 4` bases; no descriptor tables are consulted).
            let m = insn.modrm.ok_or(ExecError::Unsupported(0x22))?;
            match m.reg {
                0 => {
                    let src = cpu.gpr_u32(m.rm as usize);
                    cpu.cr0 = u64::from(src);
                    cpu.set_ip16(next_ip);
                    Ok(())
                }
                1 => real_mode_ud(cpu, bus),
                2..=4 => Err(ExecError::Unsupported(0x22)),
                _ => real_mode_ud(cpu, bus),
            }
        }
        0xAF => {
            // IMUL r16, r/m16 / IMUL r32, r/m32 — Spec: Intel SDM Vol. 2 "IMUL".
            // Dest = ModRM.reg := ModRM.reg * r/m (signed).
            // Unsupported here: REX.W r64 form; LOCK #UD.
            let m = insn.modrm.ok_or(ExecError::Unsupported(0xAF))?;
            if opsz32(insn) {
                let src = read_rm_u32(cpu, bus, insn)?;
                let dst = cpu.gpr_u32(m.reg as usize);
                let prod = i64::from(dst as i32).wrapping_mul(i64::from(src as i32));
                cpu.set_gpr_u32(m.reg as usize, prod as u32);
                set_imul_flags_i32(cpu, prod);
            } else {
                let src = read_rm_u16(cpu, bus, insn)?;
                let dst = cpu.gpr_u16(m.reg as usize);
                let prod = i32::from(dst as i16).wrapping_mul(i32::from(src as i16));
                cpu.set_gpr_u16(m.reg as usize, prod as u16);
                set_imul_flags_i16(cpu, prod);
            }
            cpu.set_ip16(next_ip);
            Ok(())
        }
        op => Err(ExecError::Unsupported(op)),
    }
}

/// Execute a single instruction at CS:IP.
///
/// Services latched `#NMI` (vector 2, not gated by `IF`) before maskable IRQs,
/// then when `IF=1` services a latched/polled external IRQ before fetch/decode
/// so non-REP instructions are interruptible (REP also polls between iterations).
/// Spec: Intel SDM Vol. 3 §6.3.3 / §6.7 (NMI); §6.8.1 (maskable when IF=1).
pub fn step(cpu: &mut CpuState, bus: &mut dyn Bus) -> Result<(), ExecError> {
    // Platform `#NMI` outranks maskable IRQs and can wake HLT.
    if service_pending_nmi(cpu, bus)? {
        return Ok(());
    }
    if cpu.halted {
        return Ok(());
    }
    // Per-instruction external IRQ poll (PIC stub via pending_irq / Bus).
    if service_pending_external_interrupt(cpu, bus)? {
        return Ok(());
    }
    match step_inner(cpu, bus) {
        Err(ExecError::ArchFault(vector)) => real_mode_exception(cpu, bus, vector),
        other => other,
    }
}

fn step_inner(cpu: &mut CpuState, bus: &mut dyn Bus) -> Result<(), ExecError> {
    let insn = fetch_decode(cpu, bus)?;
    let next_ip = cpu.ip16().wrapping_add(insn.length as u16);
    let op = insn.opcode;

    if insn.two_byte {
        return step_two_byte(cpu, bus, &insn, next_ip);
    }

    match op {
        0x06 => {
            // PUSH ES — Spec: Intel SDM Vol. 2 "PUSH".
            push16(cpu, bus, cpu.es.selector)?;
            cpu.set_ip16(next_ip);
        }
        0x07 => {
            // POP ES — Spec: Intel SDM Vol. 2 "POP".
            let sel = pop16(cpu, bus)?;
            cpu.es.load_real_mode_selector(sel);
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
            cpu.ss.load_real_mode_selector(sel);
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
            cpu.ds.load_real_mode_selector(sel);
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
            // XCHG AX/EAX, r16/r32 — Spec: Intel SDM Vol. 2 "XCHG"; Ch. 2 (66H).
            // Opcode 90 is NOP (XCHG AX/EAX,AX/EAX). Unsupported: REX.W (XCHG RAX,r64).
            let idx = (op - 0x90) as usize;
            if opsz32(&insn) {
                let eax = cpu.eax();
                let other = cpu.gpr_u32(idx);
                cpu.set_eax(other);
                cpu.set_gpr_u32(idx, eax);
            } else {
                let ax = cpu.ax();
                let other = cpu.gpr_u16(idx);
                cpu.set_ax(other);
                cpu.set_gpr_u16(idx, ax);
            }
            cpu.set_ip16(next_ip);
        }
        0x98 => {
            // CBW/CWDE — Spec: Intel SDM Vol. 2 "CBW/CWDE/CDQE"; Ch. 2 (66H).
            // Unsupported here: CDQE (REX.W).
            if opsz32(&insn) {
                // CWDE: sign-extend AX into EAX.
                let eax = cpu.ax() as i16 as i32 as u32;
                cpu.set_eax(eax);
            } else {
                // CBW: sign-extend AL into AX.
                let al = cpu.al() as i8 as i16 as u16;
                cpu.set_ax(al);
            }
            cpu.set_ip16(next_ip);
        }
        0x99 => {
            // CWD/CDQ — Spec: Intel SDM Vol. 2 "CWD/CDQ/CQO"; Ch. 2 (66H).
            // Unsupported here: CQO (REX.W).
            if opsz32(&insn) {
                // CDQ: sign-extend EAX into EDX:EAX.
                let edx = if cpu.eax() & 0x8000_0000 != 0 {
                    0xFFFF_FFFFu32
                } else {
                    0
                };
                cpu.set_gpr_u32(CpuState::RDX, edx);
            } else {
                let dx = if cpu.ax() & 0x8000 != 0 { 0xFFFFu16 } else { 0 };
                cpu.set_gpr_u16(CpuState::RDX, dx);
            }
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
            // Address-size selects CX (16) or ECX (32). Unsupported: asize 64 (RCX).
            let zf = cpu.rflags & (1 << 6) != 0;
            let take = if asize32(&insn) {
                let ecx = cpu.gpr_u32(CpuState::RCX).wrapping_sub(1);
                cpu.set_gpr_u32(CpuState::RCX, ecx);
                match op {
                    0xE0 => ecx != 0 && !zf, // LOOPNE / LOOPNZ
                    0xE1 => ecx != 0 && zf,  // LOOPE / LOOPZ
                    0xE2 => ecx != 0,        // LOOP
                    _ => unreachable!("matched 0xE0..=0xE2"),
                }
            } else {
                let cx = cpu.gpr_u16(CpuState::RCX).wrapping_sub(1);
                cpu.set_gpr_u16(CpuState::RCX, cx);
                match op {
                    0xE0 => cx != 0 && !zf,
                    0xE1 => cx != 0 && zf,
                    0xE2 => cx != 0,
                    _ => unreachable!("matched 0xE0..=0xE2"),
                }
            };
            if take {
                cpu.set_ip16(next_ip.wrapping_add(insn.immediate as i16 as u16));
            } else {
                cpu.set_ip16(next_ip);
            }
        }
        0xE3 => {
            // JCXZ/JECXZ rel8 — Spec: Intel SDM Vol. 2 "JCXZ/JECXZ/JRCXZ".
            // Address-size selects CX (16) or ECX (32). Unsupported: JRCXZ (asize 64).
            let zero = if asize32(&insn) {
                cpu.gpr_u32(CpuState::RCX) == 0
            } else {
                cpu.gpr_u16(CpuState::RCX) == 0
            };
            if zero {
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
            // JMP near rel16/rel32 — Spec: Intel SDM Vol. 2 "JMP"; Ch. 2 (66H).
            // Code fetch still uses IP16 (CS:IP); target truncated to 16 bits.
            if opsz32(&insn) {
                let eip = u32::from(next_ip).wrapping_add(insn.immediate as u32);
                cpu.set_ip16(eip as u16);
            } else {
                let target = next_ip.wrapping_add(insn.immediate as i16 as u16);
                cpu.set_ip16(target);
            }
        }
        0xEA => {
            // JMP far ptr16:16 / ptr16:32 — real-address mode.
            // Spec: Intel SDM Vol. 2 "JMP"; Ch. 2 (66H).
            // Unsupported here: protected-mode / task-gate forms.
            // Code fetch still uses IP16 (CS:IP); offset truncated to 16 bits.
            let offset = if opsz32(&insn) {
                insn.immediate as u32
            } else {
                u32::from(insn.immediate as u16)
            };
            let selector = insn.displacement as u16;
            cpu.cs = x86_core::SegmentReg::real_mode_code(selector);
            cpu.set_ip16(offset as u16);
        }
        0xE8 => {
            // CALL near rel16/rel32 — Spec: Intel SDM Vol. 2 "CALL"; Ch. 2 (66H).
            // Opsize 32: push 32-bit return EIP; code fetch still IP16-truncated.
            if opsz32(&insn) {
                push32(cpu, bus, u32::from(next_ip))?;
                let eip = u32::from(next_ip).wrapping_add(insn.immediate as u32);
                cpu.set_ip16(eip as u16);
            } else {
                push16(cpu, bus, next_ip)?;
                let target = next_ip.wrapping_add(insn.immediate as i16 as u16);
                cpu.set_ip16(target);
            }
        }
        0xC2 => {
            // RET iw — near return with stack release.
            // Spec: Intel SDM Vol. 2 "RET" (near, imm16). Imm16 release always;
            // opsize selects pop IP16 vs EIP32.
            let release = insn.immediate as u16;
            if opsz32(&insn) {
                let eip = pop32(cpu, bus)?;
                let sp = cpu.gpr_u16(CpuState::RSP).wrapping_add(release);
                cpu.set_gpr_u16(CpuState::RSP, sp);
                cpu.set_ip16(eip as u16);
            } else {
                let ip = pop16(cpu, bus)?;
                let sp = cpu.gpr_u16(CpuState::RSP).wrapping_add(release);
                cpu.set_gpr_u16(CpuState::RSP, sp);
                cpu.set_ip16(ip);
            }
        }
        0xC3 => {
            // RET near — Spec: Intel SDM Vol. 2 "RET".
            if opsz32(&insn) {
                let eip = pop32(cpu, bus)?;
                cpu.set_ip16(eip as u16);
            } else {
                let ip = pop16(cpu, bus)?;
                cpu.set_ip16(ip);
            }
        }
        0xC4 => {
            // LES r16/r32, m16:16/m16:32 — load offset into r and selector into ES.
            // Spec: Intel SDM Vol. 2 "LES"; Ch. 2 (66H).
            // Register form (mod=11) → #UD (Vol. 3 §6.15).
            // Unsupported here: protected-mode descriptor checks.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if m.mod_ == 3 {
                return real_mode_ud(cpu, bus);
            }
            if opsz32(&insn) {
                let (offset, selector) = read_far_ptr32(cpu, bus, &insn)?;
                cpu.set_gpr_u32(m.reg as usize, offset);
                cpu.es.load_real_mode_selector(selector);
            } else {
                let (offset, selector) = read_far_ptr16(cpu, bus, &insn)?;
                cpu.set_gpr_u16(m.reg as usize, offset);
                cpu.es.load_real_mode_selector(selector);
            }
            cpu.set_ip16(next_ip);
        }
        0xC5 => {
            // LDS r16/r32, m16:16/m16:32 — load offset into r and selector into DS.
            // Spec: Intel SDM Vol. 2 "LDS"; Ch. 2 (66H).
            // Register form (mod=11) → #UD (Vol. 3 §6.15).
            // Unsupported here: protected-mode descriptor checks.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if m.mod_ == 3 {
                return real_mode_ud(cpu, bus);
            }
            if opsz32(&insn) {
                let (offset, selector) = read_far_ptr32(cpu, bus, &insn)?;
                cpu.set_gpr_u32(m.reg as usize, offset);
                cpu.ds.load_real_mode_selector(selector);
            } else {
                let (offset, selector) = read_far_ptr16(cpu, bus, &insn)?;
                cpu.set_gpr_u16(m.reg as usize, offset);
                cpu.ds.load_real_mode_selector(selector);
            }
            cpu.set_ip16(next_ip);
        }
        0x9A => {
            // CALL far ptr16:16 / ptr16:32 — real-address mode.
            // Spec: Intel SDM Vol. 2 "CALL"; Ch. 2 (66H).
            // Real-address OperandSize=32: push CS (16) then EIP (32) — 6-byte frame.
            // Unsupported here: protected-mode privilege / gate transfer.
            let selector = insn.displacement as u16;
            if opsz32(&insn) {
                let offset = insn.immediate as u32;
                push16(cpu, bus, cpu.cs.selector)?;
                push32(cpu, bus, u32::from(next_ip))?;
                cpu.cs = x86_core::SegmentReg::real_mode_code(selector);
                cpu.set_ip16(offset as u16);
            } else {
                let offset = insn.immediate as u16;
                push16(cpu, bus, cpu.cs.selector)?;
                push16(cpu, bus, next_ip)?;
                cpu.cs = x86_core::SegmentReg::real_mode_code(selector);
                cpu.set_ip16(offset);
            }
        }
        0xCA => {
            // RETF iw — far return with stack release.
            // Spec: Intel SDM Vol. 2 "RET" (far, imm16); Ch. 2 (66H).
            // Opsize 32: pop EIP32 then CS16; Imm16 release always.
            // Unsupported here: protected-mode privilege checks.
            let release = insn.immediate as u16;
            if opsz32(&insn) {
                let eip = pop32(cpu, bus)?;
                let cs_sel = pop16(cpu, bus)?;
                let sp = cpu.gpr_u16(CpuState::RSP).wrapping_add(release);
                cpu.set_gpr_u16(CpuState::RSP, sp);
                cpu.cs = x86_core::SegmentReg::real_mode_code(cs_sel);
                cpu.set_ip16(eip as u16);
            } else {
                let ip = pop16(cpu, bus)?;
                let cs_sel = pop16(cpu, bus)?;
                let sp = cpu.gpr_u16(CpuState::RSP).wrapping_add(release);
                cpu.set_gpr_u16(CpuState::RSP, sp);
                cpu.cs = x86_core::SegmentReg::real_mode_code(cs_sel);
                cpu.set_ip16(ip);
            }
        }
        0xCB => {
            // RETF — far return.
            // Spec: Intel SDM Vol. 2 "RET" (far); Ch. 2 (66H).
            // Opsize 16: pop IP then CS; opsize 32: pop EIP then CS (6-byte frame).
            if opsz32(&insn) {
                let eip = pop32(cpu, bus)?;
                let cs_sel = pop16(cpu, bus)?;
                cpu.cs = x86_core::SegmentReg::real_mode_code(cs_sel);
                cpu.set_ip16(eip as u16);
            } else {
                let ip = pop16(cpu, bus)?;
                let cs_sel = pop16(cpu, bus)?;
                cpu.cs = x86_core::SegmentReg::real_mode_code(cs_sel);
                cpu.set_ip16(ip);
            }
        }
        0xC8 => {
            // ENTER/ENTERD iw, ib — nesting level = imm8 mod 32.
            // Spec: Intel SDM Vol. 2 "ENTER"; Vol. 1 §6.5 / §3.6; Ch. 2 (66H).
            // Address-size 16: SP/BP used for stack walks; opsize selects word vs dword.
            // Unsupported here: address-size 32 (0x67 → ESP/EBP stack; push helpers
            // are SP-only); asize 64; protected-mode.
            if asize32(&insn) {
                return Err(ExecError::Unsupported(op));
            }
            let alloc = insn.immediate as u16;
            let nesting = (insn.displacement as u8) & 0x1F;
            if opsz32(&insn) {
                push32(cpu, bus, cpu.gpr_u32(CpuState::RBP))?;
                let frame_temp = cpu.gpr_u32(CpuState::RSP);
                if nesting > 0 {
                    for _ in 1..nesting {
                        let bp = cpu.gpr_u16(CpuState::RBP).wrapping_sub(4);
                        cpu.set_gpr_u16(CpuState::RBP, bp);
                        let addr = seg_linear_checked(&cpu.ss, u64::from(bp), 4, true)?;
                        let display = bus
                            .read_u32(addr)
                            .map_err(|e| classify_mem_fault(e, true))?;
                        push32(cpu, bus, display)?;
                    }
                    push32(cpu, bus, frame_temp)?;
                }
                cpu.set_gpr_u32(CpuState::RBP, frame_temp);
                let sp = cpu.gpr_u16(CpuState::RSP).wrapping_sub(alloc);
                cpu.set_gpr_u16(CpuState::RSP, sp);
            } else {
                push16(cpu, bus, cpu.gpr_u16(CpuState::RBP))?;
                let frame_temp = cpu.gpr_u16(CpuState::RSP);
                if nesting > 0 {
                    // Copy nesting-1 display pointers from the caller's frame, then
                    // push frame_temp (current procedure's frame pointer for LEAVE).
                    for _ in 1..nesting {
                        let bp = cpu.gpr_u16(CpuState::RBP).wrapping_sub(2);
                        cpu.set_gpr_u16(CpuState::RBP, bp);
                        let addr = seg_linear_checked(&cpu.ss, u64::from(bp), 2, true)?;
                        let display = bus
                            .read_u16(addr)
                            .map_err(|e| classify_mem_fault(e, true))?;
                        push16(cpu, bus, display)?;
                    }
                    push16(cpu, bus, frame_temp)?;
                }
                cpu.set_gpr_u16(CpuState::RBP, frame_temp);
                let sp = cpu.gpr_u16(CpuState::RSP).wrapping_sub(alloc);
                cpu.set_gpr_u16(CpuState::RSP, sp);
            }
            cpu.set_ip16(next_ip);
        }
        0xC9 => {
            // LEAVE — Spec: Intel SDM Vol. 2 "LEAVE"; Ch. 2 (66H).
            // Address-size 16: SP ← BP; opsize selects BP vs EBP pop width.
            // Unsupported here: address-size 32 (0x67 → ESP←EBP); asize 64.
            if asize32(&insn) {
                return Err(ExecError::Unsupported(op));
            }
            let bp = cpu.gpr_u16(CpuState::RBP);
            cpu.set_gpr_u16(CpuState::RSP, bp);
            if opsz32(&insn) {
                let v = pop32(cpu, bus)?;
                cpu.set_gpr_u32(CpuState::RBP, v);
            } else {
                let v = pop16(cpu, bus)?;
                cpu.set_gpr_u16(CpuState::RBP, v);
            }
            cpu.set_ip16(next_ip);
        }
        0x9C => {
            // PUSHF/PUSHFD — Spec: Intel SDM Vol. 2 "PUSHF/PUSHFD/PUSHFQ"; Ch. 2 (66H).
            // Real-address mode: push FLAGS (16) or EFLAGS (32). Unsupported: PUSHFQ.
            if opsz32(&insn) {
                push32(cpu, bus, cpu.rflags as u32)?;
            } else {
                push16(cpu, bus, cpu.rflags as u16)?;
            }
            cpu.set_ip16(next_ip);
        }
        0x9D => {
            // POPF/POPFD — Spec: Intel SDM Vol. 2 "POPF/POPFD/POPFQ"; Ch. 2 (66H).
            // Real-address mode: VM and RF unaffected; reserved bit 1 stays set.
            // Unsupported here: IOPL/VIP/VIF privilege masking (protected / V86); POPFQ.
            if opsz32(&insn) {
                let flags = pop32(cpu, bus)?;
                let vm_rf = cpu.rflags & ((1 << 16) | (1 << 17));
                cpu.rflags = (cpu.rflags & !0xFFFF_FFFF) | u64::from(flags) | 2;
                cpu.rflags = (cpu.rflags & !((1 << 16) | (1 << 17))) | vm_rf;
            } else {
                let flags = pop16(cpu, bus)?;
                cpu.rflags = (cpu.rflags & !0xFFFF) | u64::from(flags) | 2;
            }
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
        0xCE => {
            // INTO — if OF=1, #OF (vector 4) trap via IVT; else fall through.
            // Spec: Intel SDM Vol. 2 "INT n/INTO/INT3/INT1"; Vol. 3 §6.15 (#OF — trap).
            // Saved IP is the following instruction (trap class).
            // Unsupported here: 64-bit mode (#UD); protected-mode privilege checks.
            if cpu.rflags & (1 << 11) != 0 {
                real_mode_software_interrupt(cpu, bus, 4, next_ip)?;
            } else {
                cpu.set_ip16(next_ip);
            }
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
        0xD4 => {
            // AAM — ASCII Adjust AX After Multiply. Spec: Intel SDM Vol. 2 "AAM".
            // imm8=0 → #DE (Vol. 3 §6.15). OF/AF/CF undefined (left unchanged).
            // Unsupported here: 64-bit mode (#UD).
            let base = insn.immediate as u8;
            if base == 0 {
                return real_mode_exception(cpu, bus, 0);
            }
            let temp_al = cpu.al();
            cpu.set_ah(temp_al / base);
            let al = temp_al % base;
            cpu.set_al(al);
            set_bcd_szp_flags_u8(cpu, al);
            cpu.set_ip16(next_ip);
        }
        0xD5 => {
            // AAD — ASCII Adjust AX Before Division. Spec: Intel SDM Vol. 2 "AAD".
            // OF/AF/CF undefined (left unchanged). Unsupported here: 64-bit mode (#UD).
            let base = insn.immediate as u8;
            let temp_al = cpu.al();
            let temp_ah = cpu.ah();
            let al = temp_al.wrapping_add(temp_ah.wrapping_mul(base));
            cpu.set_al(al);
            cpu.set_ah(0);
            set_bcd_szp_flags_u8(cpu, al);
            cpu.set_ip16(next_ip);
        }
        0xD7 => {
            // XLAT/XLATB — AL ← [rBX + AL] (segment overrideable).
            // Spec: Intel SDM Vol. 2 "XLAT/XLATB"; Vol. 1 §3.6 (address-size).
            // Address-size 16 → BX; 0x67 → EBX. Opsize does not apply. Unsupported: asize 64.
            let off = if asize32(&insn) {
                u64::from(cpu.gpr_u32(CpuState::RBX).wrapping_add(u32::from(cpu.al())))
            } else {
                u64::from(cpu.gpr_u16(CpuState::RBX).wrapping_add(u16::from(cpu.al())))
            };
            let seg = data_seg_for_string_src(cpu, &insn);
            let uses_ss = matches!(insn.prefixes.segment_override, Some(0x36));
            let addr = seg_linear_checked(seg, off, 1, uses_ss)?;
            let v = bus
                .read_u8(addr)
                .map_err(|e| classify_mem_fault(e, uses_ss))?;
            cpu.set_al(v);
            cpu.set_ip16(next_ip);
        }
        0xD0 => {
            // Group 2 r/m8, 1 — Spec: Intel SDM Vol. 2 ROL/ROR/RCL/RCR/SHL/SHR/SAR.
            // /6 reserved → #UD (Vol. 3 §6.15).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if m.reg == 6 {
                return real_mode_ud(cpu, bus);
            }
            let v = read_rm_u8(cpu, bus, &insn)?;
            let r = grp2_u8(cpu, m.reg, v, 1)?;
            write_rm_u8(cpu, bus, &insn, r)?;
            cpu.set_ip16(next_ip);
        }
        0xD1 => {
            // Group 2 r/m16|32, 1 — Spec: Intel SDM Vol. 2 ROL/ROR/RCL/RCR/SHL/SHR/SAR; Ch. 2.
            // /6 reserved → #UD (Vol. 3 §6.15).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if m.reg == 6 {
                return real_mode_ud(cpu, bus);
            }
            if opsz32(&insn) {
                let v = read_rm_u32(cpu, bus, &insn)?;
                let r = grp2_u32(cpu, m.reg, v, 1)?;
                write_rm_u32(cpu, bus, &insn, r)?;
            } else {
                let v = read_rm_u16(cpu, bus, &insn)?;
                let r = grp2_u16(cpu, m.reg, v, 1)?;
                write_rm_u16(cpu, bus, &insn, r)?;
            }
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
            // Group 1 r/m16|32, imm16|32 — Spec: Intel SDM Vol. 2; Ch. 2 (66H).
            // Unsupported here: LOCK.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if opsz32(&insn) {
                let a = read_rm_u32(cpu, bus, &insn)?;
                let b = insn.immediate as u32;
                if let Some(r) = grp1_u32(cpu, m.reg, a, b)? {
                    write_rm_u32(cpu, bus, &insn, r)?;
                }
            } else {
                let a = read_rm_u16(cpu, bus, &insn)?;
                let b = insn.immediate as u16;
                if let Some(r) = grp1_u16(cpu, m.reg, a, b)? {
                    write_rm_u16(cpu, bus, &insn, r)?;
                }
            }
            cpu.set_ip16(next_ip);
        }
        0x83 => {
            // Group 1 r/m16|32, imm8 (sign-extended) — Spec: Intel SDM Vol. 2; Ch. 2.
            // Unsupported here: LOCK.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if opsz32(&insn) {
                let a = read_rm_u32(cpu, bus, &insn)?;
                let b = insn.immediate as i8 as i32 as u32;
                if let Some(r) = grp1_u32(cpu, m.reg, a, b)? {
                    write_rm_u32(cpu, bus, &insn, r)?;
                }
            } else {
                let a = read_rm_u16(cpu, bus, &insn)?;
                let b = insn.immediate as i8 as i16 as u16;
                if let Some(r) = grp1_u16(cpu, m.reg, a, b)? {
                    write_rm_u16(cpu, bus, &insn, r)?;
                }
            }
            cpu.set_ip16(next_ip);
        }
        0xC0 => {
            // Group 2 r/m8, imm8 — Spec: Intel SDM Vol. 2 (COUNT masked to 5 bits).
            // /6 reserved → #UD (Vol. 3 §6.15).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if m.reg == 6 {
                return real_mode_ud(cpu, bus);
            }
            let v = read_rm_u8(cpu, bus, &insn)?;
            let r = grp2_u8(cpu, m.reg, v, insn.immediate as u8)?;
            write_rm_u8(cpu, bus, &insn, r)?;
            cpu.set_ip16(next_ip);
        }
        0xC1 => {
            // Group 2 r/m16|32, imm8 — Spec: Intel SDM Vol. 2 (COUNT masked to 5 bits); Ch. 2.
            // /6 reserved → #UD (Vol. 3 §6.15).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if m.reg == 6 {
                return real_mode_ud(cpu, bus);
            }
            if opsz32(&insn) {
                let v = read_rm_u32(cpu, bus, &insn)?;
                let r = grp2_u32(cpu, m.reg, v, insn.immediate as u8)?;
                write_rm_u32(cpu, bus, &insn, r)?;
            } else {
                let v = read_rm_u16(cpu, bus, &insn)?;
                let r = grp2_u16(cpu, m.reg, v, insn.immediate as u8)?;
                write_rm_u16(cpu, bus, &insn, r)?;
            }
            cpu.set_ip16(next_ip);
        }
        0xD2 => {
            // Group 2 r/m8, CL — Spec: Intel SDM Vol. 2 (COUNT = CL, masked to 5 bits).
            // /6 reserved → #UD (Vol. 3 §6.15).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if m.reg == 6 {
                return real_mode_ud(cpu, bus);
            }
            let v = read_rm_u8(cpu, bus, &insn)?;
            let r = grp2_u8(cpu, m.reg, v, cpu.gpr_u8_low(CpuState::RCX))?;
            write_rm_u8(cpu, bus, &insn, r)?;
            cpu.set_ip16(next_ip);
        }
        0xD3 => {
            // Group 2 r/m16|32, CL — Spec: Intel SDM Vol. 2 (COUNT = CL, masked to 5 bits); Ch. 2.
            // /6 reserved → #UD (Vol. 3 §6.15).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if m.reg == 6 {
                return real_mode_ud(cpu, bus);
            }
            let count = cpu.gpr_u8_low(CpuState::RCX);
            if opsz32(&insn) {
                let v = read_rm_u32(cpu, bus, &insn)?;
                let r = grp2_u32(cpu, m.reg, v, count)?;
                write_rm_u32(cpu, bus, &insn, r)?;
            } else {
                let v = read_rm_u16(cpu, bus, &insn)?;
                let r = grp2_u16(cpu, m.reg, v, count)?;
                write_rm_u16(cpu, bus, &insn, r)?;
            }
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
                        return real_mode_exception(cpu, bus, 0);
                    }
                    let dividend = u32::from(cpu.ax());
                    let quot = dividend / u32::from(v);
                    let rem = dividend % u32::from(v);
                    if quot > 0xFF {
                        return real_mode_exception(cpu, bus, 0);
                    }
                    cpu.set_ax(((rem as u16) << 8) | (quot as u16));
                }
                7 => {
                    // IDIV r/m8 — signed AX / r/m8 → AL=quot, AH=rem. #DE on 0 or quot∉i8.
                    if v == 0 {
                        return real_mode_exception(cpu, bus, 0);
                    }
                    let dividend = cpu.ax() as i16;
                    let divisor = i16::from(v as i8);
                    let Some(quot) = dividend.checked_div(divisor) else {
                        return real_mode_exception(cpu, bus, 0);
                    };
                    if !(i16::from(i8::MIN)..=i16::from(i8::MAX)).contains(&quot) {
                        return real_mode_exception(cpu, bus, 0);
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
            // Group 3 r/m16|32 — TEST/NOT/NEG/MUL/IMUL/DIV/IDIV (/0–/7).
            // Spec: Intel SDM Vol. 2 "TEST"/"NOT"/"NEG"/"MUL"/"IMUL"/"DIV"/"IDIV"; Ch. 2 (66H).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if opsz32(&insn) {
                let v = read_rm_u32(cpu, bus, &insn)?;
                match m.reg {
                    0 | 1 => {
                        set_logic_flags_u32(cpu, v & (insn.immediate as u32));
                    }
                    2 => {
                        write_rm_u32(cpu, bus, &insn, !v)?;
                    }
                    3 => {
                        let r = v.wrapping_neg();
                        write_rm_u32(cpu, bus, &insn, r)?;
                        set_sub_flags_u32(cpu, 0, v, r);
                    }
                    4 => {
                        // MUL r/m32 — EDX:EAX = EAX * r/m32. CF=OF=1 iff EDX != 0.
                        let prod = u64::from(cpu.eax()).wrapping_mul(u64::from(v));
                        cpu.set_eax(prod as u32);
                        cpu.set_gpr_u32(CpuState::RDX, (prod >> 32) as u32);
                        let hi_nz = (prod >> 32) != 0;
                        cpu.set_cf(hi_nz);
                        cpu.set_of(hi_nz);
                    }
                    5 => {
                        // IMUL r/m32 — EDX:EAX = EAX * r/m32 (signed). CF=OF=1 iff not in EAX.
                        let prod = i64::from(cpu.eax() as i32).wrapping_mul(i64::from(v as i32));
                        cpu.set_eax(prod as u32);
                        cpu.set_gpr_u32(CpuState::RDX, (prod >> 32) as u32);
                        set_imul_flags_i32(cpu, prod);
                    }
                    6 => {
                        // DIV r/m32 — EDX:EAX / r/m32 → EAX=quot, EDX=rem. #DE on 0 or quot>u32::MAX.
                        if v == 0 {
                            return real_mode_exception(cpu, bus, 0);
                        }
                        let dividend =
                            (u64::from(cpu.gpr_u32(CpuState::RDX)) << 32) | u64::from(cpu.eax());
                        let quot = dividend / u64::from(v);
                        let rem = dividend % u64::from(v);
                        if quot > u64::from(u32::MAX) {
                            return real_mode_exception(cpu, bus, 0);
                        }
                        cpu.set_eax(quot as u32);
                        cpu.set_gpr_u32(CpuState::RDX, rem as u32);
                    }
                    7 => {
                        // IDIV r/m32 — signed EDX:EAX / r/m32 → EAX=quot, EDX=rem.
                        if v == 0 {
                            return real_mode_exception(cpu, bus, 0);
                        }
                        let dividend = ((u64::from(cpu.gpr_u32(CpuState::RDX)) << 32)
                            | u64::from(cpu.eax())) as i64;
                        let divisor = i64::from(v as i32);
                        let Some(quot) = dividend.checked_div(divisor) else {
                            return real_mode_exception(cpu, bus, 0);
                        };
                        if !(i64::from(i32::MIN)..=i64::from(i32::MAX)).contains(&quot) {
                            return real_mode_exception(cpu, bus, 0);
                        }
                        let rem = dividend.wrapping_rem(divisor);
                        cpu.set_eax(quot as u32);
                        cpu.set_gpr_u32(CpuState::RDX, rem as u32);
                    }
                    _ => return Err(ExecError::Unsupported(op)),
                }
            } else {
                let v = read_rm_u16(cpu, bus, &insn)?;
                match m.reg {
                    0 | 1 => {
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
                        let prod = u32::from(cpu.ax()).wrapping_mul(u32::from(v));
                        cpu.set_ax(prod as u16);
                        cpu.set_gpr_u16(CpuState::RDX, (prod >> 16) as u16);
                        let hi_nz = (prod >> 16) != 0;
                        cpu.set_cf(hi_nz);
                        cpu.set_of(hi_nz);
                    }
                    5 => {
                        let prod = i32::from(cpu.ax() as i16).wrapping_mul(i32::from(v as i16));
                        cpu.set_ax(prod as u16);
                        cpu.set_gpr_u16(CpuState::RDX, (prod >> 16) as u16);
                        set_imul_flags_i16(cpu, prod);
                    }
                    6 => {
                        if v == 0 {
                            return real_mode_exception(cpu, bus, 0);
                        }
                        let dividend =
                            (u32::from(cpu.gpr_u16(CpuState::RDX)) << 16) | u32::from(cpu.ax());
                        let quot = dividend / u32::from(v);
                        let rem = dividend % u32::from(v);
                        if quot > 0xFFFF {
                            return real_mode_exception(cpu, bus, 0);
                        }
                        cpu.set_ax(quot as u16);
                        cpu.set_gpr_u16(CpuState::RDX, rem as u16);
                    }
                    7 => {
                        if v == 0 {
                            return real_mode_exception(cpu, bus, 0);
                        }
                        let dividend = ((u32::from(cpu.gpr_u16(CpuState::RDX)) << 16)
                            | u32::from(cpu.ax())) as i32;
                        let divisor = i32::from(v as i16);
                        let Some(quot) = dividend.checked_div(divisor) else {
                            return real_mode_exception(cpu, bus, 0);
                        };
                        if !(i32::from(i16::MIN)..=i32::from(i16::MAX)).contains(&quot) {
                            return real_mode_exception(cpu, bus, 0);
                        }
                        let rem = dividend.wrapping_rem(divisor);
                        cpu.set_ax(quot as u16);
                        cpu.set_gpr_u16(CpuState::RDX, rem as u16);
                    }
                    _ => return Err(ExecError::Unsupported(op)),
                }
            }
            cpu.set_ip16(next_ip);
        }
        0xFE => {
            // Group 4 r/m8 — INC (/0) / DEC (/1). Spec: Intel SDM Vol. 2 "INC"/"DEC".
            // /2–/7 reserved → #UD (Vol. 3 §6.15).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if m.reg > 1 {
                return real_mode_ud(cpu, bus);
            }
            let v = read_rm_u8(cpu, bus, &insn)?;
            let saved_cf = cpu.rflags & 1 != 0;
            if m.reg == 0 {
                let r = v.wrapping_add(1);
                write_rm_u8(cpu, bus, &insn, r)?;
                set_add_flags_u8(cpu, v, 1, r);
            } else {
                let r = v.wrapping_sub(1);
                write_rm_u8(cpu, bus, &insn, r)?;
                set_sub_flags_u8(cpu, v, 1, r);
            }
            // INC/DEC do not modify CF (Intel SDM Vol. 2, INC/DEC).
            cpu.set_cf(saved_cf);
            cpu.set_ip16(next_ip);
        }
        0xFF => {
            // Group 5 r/m16|32 — INC/DEC/CALL/JMP/PUSH.
            // Spec: Intel SDM Vol. 2 "INC"/"DEC"/"CALL"/"JMP"/"PUSH"; opcode map Group 5;
            // Ch. 2 (66H). /7 reserved and far CALL/JMP register forms → #UD (Vol. 3 §6.15).
            // Unsupported here: protected-mode transfers.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let op32 = opsz32(&insn);
            match m.reg {
                0 | 1 => {
                    let saved_cf = cpu.rflags & 1 != 0;
                    if op32 {
                        let v = read_rm_u32(cpu, bus, &insn)?;
                        if m.reg == 0 {
                            let r = v.wrapping_add(1);
                            write_rm_u32(cpu, bus, &insn, r)?;
                            set_add_flags_u32(cpu, v, 1, r);
                        } else {
                            let r = v.wrapping_sub(1);
                            write_rm_u32(cpu, bus, &insn, r)?;
                            set_sub_flags_u32(cpu, v, 1, r);
                        }
                    } else {
                        let v = read_rm_u16(cpu, bus, &insn)?;
                        if m.reg == 0 {
                            let r = v.wrapping_add(1);
                            write_rm_u16(cpu, bus, &insn, r)?;
                            set_add_flags_u16(cpu, v, 1, r);
                        } else {
                            let r = v.wrapping_sub(1);
                            write_rm_u16(cpu, bus, &insn, r)?;
                            set_sub_flags_u16(cpu, v, 1, r);
                        }
                    }
                    // INC/DEC do not modify CF (Intel SDM Vol. 2, INC/DEC).
                    cpu.set_cf(saved_cf);
                    cpu.set_ip16(next_ip);
                }
                2 => {
                    // CALL r/m16|32 near absolute indirect.
                    if op32 {
                        let target = read_rm_u32(cpu, bus, &insn)?;
                        push32(cpu, bus, u32::from(next_ip))?;
                        cpu.set_ip16(target as u16);
                    } else {
                        let target = read_rm_u16(cpu, bus, &insn)?;
                        push16(cpu, bus, next_ip)?;
                        cpu.set_ip16(target);
                    }
                }
                3 => {
                    // CALL FAR m16:16 / m16:32 — absolute indirect far (memory only).
                    // Spec: Intel SDM Vol. 2 "CALL"; opcode map Group 5 /3; Ch. 2 (66H).
                    // Register form is invalid (#UD). Unsupported: protected-mode gates.
                    if m.mod_ == 3 {
                        return real_mode_ud(cpu, bus);
                    }
                    if op32 {
                        let (addr, _, uses_ss) = ea(cpu, &insn, 6)?;
                        let offset = bus
                            .read_u32(addr)
                            .map_err(|e| classify_mem_fault(e, uses_ss))?;
                        let selector = bus
                            .read_u16(addr.wrapping_add(4))
                            .map_err(|e| classify_mem_fault(e, uses_ss))?;
                        push16(cpu, bus, cpu.cs.selector)?;
                        push32(cpu, bus, u32::from(next_ip))?;
                        cpu.cs = x86_core::SegmentReg::real_mode_code(selector);
                        cpu.set_ip16(offset as u16);
                    } else {
                        let (addr, _, uses_ss) = ea(cpu, &insn, 4)?;
                        let offset = bus
                            .read_u16(addr)
                            .map_err(|e| classify_mem_fault(e, uses_ss))?;
                        let selector = bus
                            .read_u16(addr.wrapping_add(2))
                            .map_err(|e| classify_mem_fault(e, uses_ss))?;
                        push16(cpu, bus, cpu.cs.selector)?;
                        push16(cpu, bus, next_ip)?;
                        cpu.cs = x86_core::SegmentReg::real_mode_code(selector);
                        cpu.set_ip16(offset);
                    }
                }
                4 => {
                    // JMP r/m16|32 near absolute indirect.
                    if op32 {
                        let target = read_rm_u32(cpu, bus, &insn)?;
                        cpu.set_ip16(target as u16);
                    } else {
                        let target = read_rm_u16(cpu, bus, &insn)?;
                        cpu.set_ip16(target);
                    }
                }
                5 => {
                    // JMP FAR m16:16 / m16:32 — absolute indirect far (memory only).
                    // Spec: Intel SDM Vol. 2 "JMP"; opcode map Group 5 /5; Ch. 2 (66H).
                    // Register form is invalid (#UD). Unsupported: protected-mode gates.
                    if m.mod_ == 3 {
                        return real_mode_ud(cpu, bus);
                    }
                    if op32 {
                        let (addr, _, uses_ss) = ea(cpu, &insn, 6)?;
                        let offset = bus
                            .read_u32(addr)
                            .map_err(|e| classify_mem_fault(e, uses_ss))?;
                        let selector = bus
                            .read_u16(addr.wrapping_add(4))
                            .map_err(|e| classify_mem_fault(e, uses_ss))?;
                        cpu.cs = x86_core::SegmentReg::real_mode_code(selector);
                        cpu.set_ip16(offset as u16);
                    } else {
                        let (addr, _, uses_ss) = ea(cpu, &insn, 4)?;
                        let offset = bus
                            .read_u16(addr)
                            .map_err(|e| classify_mem_fault(e, uses_ss))?;
                        let selector = bus
                            .read_u16(addr.wrapping_add(2))
                            .map_err(|e| classify_mem_fault(e, uses_ss))?;
                        cpu.cs = x86_core::SegmentReg::real_mode_code(selector);
                        cpu.set_ip16(offset);
                    }
                }
                6 => {
                    // PUSH r/m16|32 — value is read before SP decrement (incl. PUSH SP).
                    if op32 {
                        let v = read_rm_u32(cpu, bus, &insn)?;
                        push32(cpu, bus, v)?;
                    } else {
                        let v = read_rm_u16(cpu, bus, &insn)?;
                        push16(cpu, bus, v)?;
                    }
                    cpu.set_ip16(next_ip);
                }
                _ => return real_mode_ud(cpu, bus), // /7 reserved
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
            // INC r16/r32 — Spec: Intel SDM Vol. 2 "INC"; Ch. 2 (66H).
            let idx = (op - 0x40) as usize;
            let saved_cf = cpu.rflags & 1 != 0;
            if opsz32(&insn) {
                let old = cpu.gpr_u32(idx);
                let v = old.wrapping_add(1);
                cpu.set_gpr_u32(idx, v);
                set_add_flags_u32(cpu, old, 1, v);
            } else {
                let old = cpu.gpr_u16(idx);
                let v = old.wrapping_add(1);
                cpu.set_gpr_u16(idx, v);
                set_add_flags_u16(cpu, old, 1, v);
            }
            // INC does not modify CF (Intel SDM Vol. 2, INC).
            cpu.set_cf(saved_cf);
            cpu.set_ip16(next_ip);
        }
        0x48..=0x4F => {
            // DEC r16/r32 — Spec: Intel SDM Vol. 2 "DEC"; Ch. 2 (66H).
            let idx = (op - 0x48) as usize;
            let saved_cf = cpu.rflags & 1 != 0;
            if opsz32(&insn) {
                let old = cpu.gpr_u32(idx);
                let v = old.wrapping_sub(1);
                cpu.set_gpr_u32(idx, v);
                set_sub_flags_u32(cpu, old, 1, v);
            } else {
                let old = cpu.gpr_u16(idx);
                let v = old.wrapping_sub(1);
                cpu.set_gpr_u16(idx, v);
                set_sub_flags_u16(cpu, old, 1, v);
            }
            // DEC does not modify CF (Intel SDM Vol. 2, DEC).
            cpu.set_cf(saved_cf);
            cpu.set_ip16(next_ip);
        }
        0x50..=0x57 => {
            // PUSH r16/r32 — Spec: Intel SDM Vol. 2 "PUSH"; Ch. 2 (66H).
            let idx = (op - 0x50) as usize;
            if opsz32(&insn) {
                push32(cpu, bus, cpu.gpr_u32(idx))?;
            } else {
                push16(cpu, bus, cpu.gpr_u16(idx))?;
            }
            cpu.set_ip16(next_ip);
        }
        0x58..=0x5F => {
            // POP r16/r32 — Spec: Intel SDM Vol. 2 "POP"; Ch. 2 (66H).
            let idx = (op - 0x58) as usize;
            if opsz32(&insn) {
                let v = pop32(cpu, bus)?;
                cpu.set_gpr_u32(idx, v);
            } else {
                let v = pop16(cpu, bus)?;
                cpu.set_gpr_u16(idx, v);
            }
            cpu.set_ip16(next_ip);
        }
        0x60 => {
            // PUSHA/PUSHAD — push AX…DI / EAX…EDI; Temp = SP/ESP before pushes.
            // Spec: Intel SDM Vol. 2 "PUSHA/PUSHAD"; Ch. 2 (66H).
            // Address-size 16: Temp ← SP (zero-extended into dword slot for PUSHAD).
            // Unsupported here: address-size 32 (0x67 → ESP stack / Temp←ESP);
            // asize 64. Stack push helpers remain SP-only in this slice.
            if asize32(&insn) {
                return Err(ExecError::Unsupported(op));
            }
            if opsz32(&insn) {
                let temp = u32::from(cpu.gpr_u16(CpuState::RSP));
                push32(cpu, bus, cpu.gpr_u32(CpuState::RAX))?;
                push32(cpu, bus, cpu.gpr_u32(CpuState::RCX))?;
                push32(cpu, bus, cpu.gpr_u32(CpuState::RDX))?;
                push32(cpu, bus, cpu.gpr_u32(CpuState::RBX))?;
                push32(cpu, bus, temp)?;
                push32(cpu, bus, cpu.gpr_u32(CpuState::RBP))?;
                push32(cpu, bus, cpu.gpr_u32(CpuState::RSI))?;
                push32(cpu, bus, cpu.gpr_u32(CpuState::RDI))?;
            } else {
                let sp0 = cpu.gpr_u16(CpuState::RSP);
                push16(cpu, bus, cpu.gpr_u16(CpuState::RAX))?;
                push16(cpu, bus, cpu.gpr_u16(CpuState::RCX))?;
                push16(cpu, bus, cpu.gpr_u16(CpuState::RDX))?;
                push16(cpu, bus, cpu.gpr_u16(CpuState::RBX))?;
                push16(cpu, bus, sp0)?;
                push16(cpu, bus, cpu.gpr_u16(CpuState::RBP))?;
                push16(cpu, bus, cpu.gpr_u16(CpuState::RSI))?;
                push16(cpu, bus, cpu.gpr_u16(CpuState::RDI))?;
            }
            cpu.set_ip16(next_ip);
        }
        0x61 => {
            // POPA/POPAD — pop DI…AX / EDI…EAX; discard saved SP/ESP slot.
            // Spec: Intel SDM Vol. 2 "POPA/POPAD"; Ch. 2 (66H).
            // Unsupported here: address-size 32 (0x67 → ESP stack); asize 64.
            if asize32(&insn) {
                return Err(ExecError::Unsupported(op));
            }
            if opsz32(&insn) {
                let di = pop32(cpu, bus)?;
                let si = pop32(cpu, bus)?;
                let bp = pop32(cpu, bus)?;
                let _discard_esp = pop32(cpu, bus)?;
                let bx = pop32(cpu, bus)?;
                let dx = pop32(cpu, bus)?;
                let cx = pop32(cpu, bus)?;
                let ax = pop32(cpu, bus)?;
                cpu.set_gpr_u32(CpuState::RDI, di);
                cpu.set_gpr_u32(CpuState::RSI, si);
                cpu.set_gpr_u32(CpuState::RBP, bp);
                cpu.set_gpr_u32(CpuState::RBX, bx);
                cpu.set_gpr_u32(CpuState::RDX, dx);
                cpu.set_gpr_u32(CpuState::RCX, cx);
                cpu.set_gpr_u32(CpuState::RAX, ax);
            } else {
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
            }
            cpu.set_ip16(next_ip);
        }
        0x62 => {
            // BOUND r16/r32, m16&16 / m32&32 — signed index vs lower/upper bounds.
            // Spec: Intel SDM Vol. 2 "BOUND"; Vol. 3 §6.15 (#BR — fault, vector 5); Ch. 2.
            // Register form (mod=11) → #UD. #BR saved IP = BOUND instruction.
            // Unsupported here: protected mode; 64-bit (#UD).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if m.mod_ == 3 {
                return real_mode_ud(cpu, bus);
            }
            if opsz32(&insn) {
                let (addr, _, uses_ss) = ea(cpu, &insn, 8)?;
                let lower =
                    bus.read_u32(addr)
                        .map_err(|e| classify_mem_fault(e, uses_ss))? as i32;
                let upper =
                    bus.read_u32(addr.wrapping_add(4))
                        .map_err(|e| classify_mem_fault(e, uses_ss))? as i32;
                let index = cpu.gpr_u32(m.reg as usize) as i32;
                if index < lower || index > upper {
                    return real_mode_exception(cpu, bus, 5);
                }
            } else {
                let (addr, _, uses_ss) = ea(cpu, &insn, 4)?;
                let lower =
                    bus.read_u16(addr)
                        .map_err(|e| classify_mem_fault(e, uses_ss))? as i16;
                let upper =
                    bus.read_u16(addr.wrapping_add(2))
                        .map_err(|e| classify_mem_fault(e, uses_ss))? as i16;
                let index = cpu.gpr_u16(m.reg as usize) as i16;
                if index < lower || index > upper {
                    return real_mode_exception(cpu, bus, 5);
                }
            }
            cpu.set_ip16(next_ip);
        }
        0x8F => {
            // POP r/m16|32 — Group /0 only.
            // Spec: Intel SDM Vol. 2 "POP"; Ch. 2 (66H).
            // /1–/7 reserved → #UD (Vol. 3 §6.15).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if m.reg != 0 {
                return real_mode_ud(cpu, bus);
            }
            if opsz32(&insn) {
                let v = pop32(cpu, bus)?;
                write_rm_u32(cpu, bus, &insn, v)?;
            } else {
                let v = pop16(cpu, bus)?;
                write_rm_u16(cpu, bus, &insn, v)?;
            }
            cpu.set_ip16(next_ip);
        }
        0x68 => {
            // PUSH imm16/imm32 — Spec: Intel SDM Vol. 2 "PUSH"; Ch. 2 (66H).
            if opsz32(&insn) {
                push32(cpu, bus, insn.immediate as u32)?;
            } else {
                push16(cpu, bus, insn.immediate as u16)?;
            }
            cpu.set_ip16(next_ip);
        }
        0x69 | 0x6B => {
            // IMUL r16/r32, r/m16/32, imm — Spec: Intel SDM Vol. 2 "IMUL"; Ch. 2 (66H).
            // Dest = ModRM.reg; src = r/m; 6B imm8 sign-extended; 69 imm follows OsZ.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if opsz32(&insn) {
                let src = read_rm_u32(cpu, bus, &insn)?;
                let imm = if op == 0x6B {
                    i32::from(insn.immediate as i8)
                } else {
                    insn.immediate
                };
                let prod = i64::from(src as i32).wrapping_mul(i64::from(imm));
                cpu.set_gpr_u32(m.reg as usize, prod as u32);
                set_imul_flags_i32(cpu, prod);
            } else {
                let src = read_rm_u16(cpu, bus, &insn)?;
                let imm = if op == 0x6B {
                    i32::from(insn.immediate as i8)
                } else {
                    i32::from(insn.immediate as u16 as i16)
                };
                let prod = i32::from(src as i16).wrapping_mul(imm);
                cpu.set_gpr_u16(m.reg as usize, prod as u16);
                set_imul_flags_i16(cpu, prod);
            }
            cpu.set_ip16(next_ip);
        }
        0x6A => {
            // PUSH imm8 (sign-extended to opsize) — Spec: Intel SDM Vol. 2 "PUSH".
            if opsz32(&insn) {
                let v = insn.immediate as i8 as i32 as u32;
                push32(cpu, bus, v)?;
            } else {
                let v = insn.immediate as i8 as i16 as u16;
                push16(cpu, bus, v)?;
            }
            cpu.set_ip16(next_ip);
        }
        0x6C => {
            // INSB — Spec: Intel SDM Vol. 2 "INS/INSB/INSW/INSD"
            // and "REP/REPE/REPNE/REPZ/REPNZ".
            // Port = DX; dest = ES:DI. F2/F3 act as unconditional REP (count = CX).
            // Unsupported here: asize 64; IOPL/CPL checks.
            exec_string_op(cpu, bus, &insn, next_ip, None, |cpu, bus, insn| {
                insb_once(cpu, bus, insn)
            })?;
        }
        0x6D => {
            // INSW/INSD — Spec: Intel SDM Vol. 2 "INS/INSB/INSW/INSD"
            // and "REP/REPE/REPNE/REPZ/REPNZ".
            // Operand-size 16 → word; 0x66 → dword. Unsupported: asize 64; IOPL.
            if opsz32(&insn) {
                exec_string_op(cpu, bus, &insn, next_ip, None, |cpu, bus, insn| {
                    insd_once(cpu, bus, insn)
                })?;
            } else {
                exec_string_op(cpu, bus, &insn, next_ip, None, |cpu, bus, insn| {
                    insw_once(cpu, bus, insn)
                })?;
            }
        }
        0x6E => {
            // OUTSB — Spec: Intel SDM Vol. 2 "OUTS/OUTSB/OUTSW/OUTSD"
            // and "REP/REPE/REPNE/REPZ/REPNZ".
            // Port = DX; src = DS:SI (segment override allowed).
            // Unsupported here: asize 64; IOPL/CPL checks.
            exec_string_op(cpu, bus, &insn, next_ip, None, |cpu, bus, insn| {
                outsb_once(cpu, bus, insn)
            })?;
        }
        0x6F => {
            // OUTSW/OUTSD — Spec: Intel SDM Vol. 2 "OUTS/OUTSB/OUTSW/OUTSD"
            // and "REP/REPE/REPNE/REPZ/REPNZ".
            // Operand-size 16 → word; 0x66 → dword. Unsupported: asize 64; IOPL.
            if opsz32(&insn) {
                exec_string_op(cpu, bus, &insn, next_ip, None, |cpu, bus, insn| {
                    outsd_once(cpu, bus, insn)
                })?;
            } else {
                exec_string_op(cpu, bus, &insn, next_ip, None, |cpu, bus, insn| {
                    outsw_once(cpu, bus, insn)
                })?;
            }
        }
        0xA4 => {
            // MOVSB — Spec: Intel SDM Vol. 2 "MOVS/MOVSB/MOVSW/MOVSD/MOVSQ"
            // and "REP/REPE/REPNE/REPZ/REPNZ".
            // Unsupported here: MOVSQ; asize 64.
            // F2/F3 both act as unconditional REP for MOVS (count = (E)CX).
            exec_string_op(cpu, bus, &insn, next_ip, None, |cpu, bus, insn| {
                movsb_once(cpu, bus, insn)
            })?;
        }
        0xA5 => {
            // MOVSW/MOVSD — Spec: Intel SDM Vol. 2 "MOVS/MOVSB/MOVSW/MOVSD/MOVSQ"
            // and "REP/REPE/REPNE/REPZ/REPNZ".
            // Operand-size 16 → word; 0x66 → dword. Unsupported: MOVSQ; asize 64.
            if opsz32(&insn) {
                exec_string_op(cpu, bus, &insn, next_ip, None, |cpu, bus, insn| {
                    movsd_once(cpu, bus, insn)
                })?;
            } else {
                exec_string_op(cpu, bus, &insn, next_ip, None, |cpu, bus, insn| {
                    movsw_once(cpu, bus, insn)
                })?;
            }
        }
        0xAA => {
            // STOSB — Spec: Intel SDM Vol. 2 "STOS/STOSB/STOSW/STOSD/STOSQ"
            // and "REP/REPE/REPNE/REPZ/REPNZ".
            // Unsupported here: STOSQ; asize 64.
            exec_string_op(cpu, bus, &insn, next_ip, None, |cpu, bus, insn| {
                stosb_once(cpu, bus, insn)
            })?;
        }
        0xAB => {
            // STOSW/STOSD — Spec: Intel SDM Vol. 2 "STOS/STOSB/STOSW/STOSD/STOSQ"
            // and "REP/REPE/REPNE/REPZ/REPNZ".
            // Unsupported here: STOSQ; asize 64.
            if opsz32(&insn) {
                exec_string_op(cpu, bus, &insn, next_ip, None, |cpu, bus, insn| {
                    stosd_once(cpu, bus, insn)
                })?;
            } else {
                exec_string_op(cpu, bus, &insn, next_ip, None, |cpu, bus, insn| {
                    stosw_once(cpu, bus, insn)
                })?;
            }
        }
        0xAC => {
            // LODSB — Spec: Intel SDM Vol. 2 "LODS/LODSB/LODSW/LODSD/LODSQ"
            // and "REP/REPE/REPNE/REPZ/REPNZ".
            // Unsupported here: LODSQ; asize 64.
            exec_string_op(cpu, bus, &insn, next_ip, None, |cpu, bus, insn| {
                lodsb_once(cpu, bus, insn)
            })?;
        }
        0xAD => {
            // LODSW/LODSD — Spec: Intel SDM Vol. 2 "LODS/LODSB/LODSW/LODSD/LODSQ"
            // and "REP/REPE/REPNE/REPZ/REPNZ".
            // Unsupported here: LODSQ; asize 64.
            if opsz32(&insn) {
                exec_string_op(cpu, bus, &insn, next_ip, None, |cpu, bus, insn| {
                    lodsd_once(cpu, bus, insn)
                })?;
            } else {
                exec_string_op(cpu, bus, &insn, next_ip, None, |cpu, bus, insn| {
                    lodsw_once(cpu, bus, insn)
                })?;
            }
        }
        0xA6 => {
            // CMPSB — Spec: Intel SDM Vol. 2 "CMPS/CMPSB/CMPSW/CMPSD/CMPSQ"
            // and "REP/REPE/REPNE/REPZ/REPNZ".
            // F3 = REPE (while ZF=1); F2 = REPNE (while ZF=0).
            // Unsupported here: CMPSQ; asize 64.
            let zf_term = if insn.prefixes.repne {
                Some(false)
            } else if insn.prefixes.rep {
                Some(true)
            } else {
                None
            };
            exec_string_op(cpu, bus, &insn, next_ip, zf_term, |cpu, bus, insn| {
                cmpsb_once(cpu, bus, insn)
            })?;
        }
        0xA7 => {
            // CMPSW/CMPSD — Spec: Intel SDM Vol. 2 "CMPS/CMPSB/CMPSW/CMPSD/CMPSQ"
            // and "REP/REPE/REPNE/REPZ/REPNZ".
            // F3 = REPE; F2 = REPNE. Unsupported: CMPSQ; asize 64.
            let zf_term = if insn.prefixes.repne {
                Some(false)
            } else if insn.prefixes.rep {
                Some(true)
            } else {
                None
            };
            if opsz32(&insn) {
                exec_string_op(cpu, bus, &insn, next_ip, zf_term, |cpu, bus, insn| {
                    cmpsd_once(cpu, bus, insn)
                })?;
            } else {
                exec_string_op(cpu, bus, &insn, next_ip, zf_term, |cpu, bus, insn| {
                    cmpsw_once(cpu, bus, insn)
                })?;
            }
        }
        0xAE => {
            // SCASB — Spec: Intel SDM Vol. 2 "SCAS/SCASB/SCASW/SCASD/SCASQ"
            // and "REP/REPE/REPNE/REPZ/REPNZ".
            // F3 = REPE (while ZF=1); F2 = REPNE (while ZF=0).
            // Unsupported here: SCASQ; asize 64.
            let zf_term = if insn.prefixes.repne {
                Some(false)
            } else if insn.prefixes.rep {
                Some(true)
            } else {
                None
            };
            exec_string_op(cpu, bus, &insn, next_ip, zf_term, |cpu, bus, insn| {
                scasb_once(cpu, bus, insn)
            })?;
        }
        0xAF => {
            // SCASW/SCASD — Spec: Intel SDM Vol. 2 "SCAS/SCASB/SCASW/SCASD/SCASQ"
            // and "REP/REPE/REPNE/REPZ/REPNZ".
            // F3 = REPE; F2 = REPNE. Unsupported: SCASQ; asize 64.
            let zf_term = if insn.prefixes.repne {
                Some(false)
            } else if insn.prefixes.rep {
                Some(true)
            } else {
                None
            };
            if opsz32(&insn) {
                exec_string_op(cpu, bus, &insn, next_ip, zf_term, |cpu, bus, insn| {
                    scasd_once(cpu, bus, insn)
                })?;
            } else {
                exec_string_op(cpu, bus, &insn, next_ip, zf_term, |cpu, bus, insn| {
                    scasw_once(cpu, bus, insn)
                })?;
            }
        }
        0xA0 => {
            // MOV AL, moffs8 — Spec: Intel SDM Vol. 2 "MOV".
            // Address-size 16 → moffs16; 0x67 → moffs32 (unreal high offsets).
            let off = moffs_offset(&insn);
            let seg = data_seg_for_string_src(cpu, &insn);
            let uses_ss = string_src_uses_ss(&insn);
            let addr = seg_linear_checked(seg, off, 1, uses_ss)?;
            let v = bus
                .read_u8(addr)
                .map_err(|e| classify_mem_fault(e, uses_ss))?;
            cpu.set_al(v);
            cpu.set_ip16(next_ip);
        }
        0xA1 => {
            // MOV AX/EAX, moffs — Spec: Intel SDM Vol. 2 "MOV"; Ch. 2 (66H).
            // Address-size selects moffs16/moffs32; operand-size selects AX/EAX.
            let off = moffs_offset(&insn);
            let seg = data_seg_for_string_src(cpu, &insn);
            let uses_ss = string_src_uses_ss(&insn);
            if opsz32(&insn) {
                let addr = seg_linear_checked(seg, off, 4, uses_ss)?;
                let v = bus
                    .read_u32(addr)
                    .map_err(|e| classify_mem_fault(e, uses_ss))?;
                cpu.set_eax(v);
            } else {
                let addr = seg_linear_checked(seg, off, 2, uses_ss)?;
                let v = bus
                    .read_u16(addr)
                    .map_err(|e| classify_mem_fault(e, uses_ss))?;
                cpu.set_ax(v);
            }
            cpu.set_ip16(next_ip);
        }
        0xA2 => {
            // MOV moffs8, AL — Spec: Intel SDM Vol. 2 "MOV".
            let off = moffs_offset(&insn);
            let seg = data_seg_for_string_src(cpu, &insn);
            let uses_ss = string_src_uses_ss(&insn);
            let addr = seg_linear_checked(seg, off, 1, uses_ss)?;
            bus.write_u8(addr, cpu.al())
                .map_err(|e| classify_mem_fault(e, uses_ss))?;
            cpu.set_ip16(next_ip);
        }
        0xA3 => {
            // MOV moffs, AX/EAX — Spec: Intel SDM Vol. 2 "MOV"; Ch. 2 (66H).
            let off = moffs_offset(&insn);
            let seg = data_seg_for_string_src(cpu, &insn);
            let uses_ss = string_src_uses_ss(&insn);
            if opsz32(&insn) {
                let addr = seg_linear_checked(seg, off, 4, uses_ss)?;
                bus.write_u32(addr, cpu.eax())
                    .map_err(|e| classify_mem_fault(e, uses_ss))?;
            } else {
                let addr = seg_linear_checked(seg, off, 2, uses_ss)?;
                bus.write_u16(addr, cpu.ax())
                    .map_err(|e| classify_mem_fault(e, uses_ss))?;
            }
            cpu.set_ip16(next_ip);
        }
        0xA8 => {
            // TEST AL, imm8 — Spec: Intel SDM Vol. 2 "TEST".
            // Flags: CF=OF=0; SF/ZF/PF from (AL & imm); AF undefined (cleared).
            set_logic_flags_u8(cpu, cpu.al() & insn.immediate as u8);
            cpu.set_ip16(next_ip);
        }
        0xA9 => {
            // TEST AX/EAX, imm16/imm32 — Spec: Intel SDM Vol. 2 "TEST"; Ch. 2 (66H).
            // Flags: CF=OF=0; SF/ZF/PF from (AX/EAX & imm); AF undefined (cleared).
            if opsz32(&insn) {
                set_logic_flags_u32(cpu, cpu.eax() & insn.immediate as u32);
            } else {
                set_logic_flags_u16(cpu, cpu.ax() & insn.immediate as u16);
            }
            cpu.set_ip16(next_ip);
        }
        0xC6 => {
            // Group 11 MOV r/m8, imm8 — Spec: Intel SDM Vol. 2 "MOV" / opcode map.
            // Only /0 is defined; /1–/7 → #UD (Vol. 3 §6.15).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if m.reg != 0 {
                return real_mode_ud(cpu, bus);
            }
            write_rm_u8(cpu, bus, &insn, insn.immediate as u8)?;
            cpu.set_ip16(next_ip);
        }
        0xC7 => {
            // Group 11 MOV r/m16|32, imm16|32 — Spec: Intel SDM Vol. 2 "MOV"; Ch. 2.
            // Only /0 is defined; /1–/7 → #UD (Vol. 3 §6.15).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if m.reg != 0 {
                return real_mode_ud(cpu, bus);
            }
            if opsz32(&insn) {
                write_rm_u32(cpu, bus, &insn, insn.immediate as u32)?;
            } else {
                write_rm_u16(cpu, bus, &insn, insn.immediate as u16)?;
            }
            cpu.set_ip16(next_ip);
        }
        0xB0..=0xB7 => {
            // MOV r8, imm8 - B0-B3 AL/CL/DL/BL; B4-B7 AH/CH/DH/BH (SDM Vol. 2 MOV).
            write_reg_u8(cpu, op - 0xB0, insn.immediate as u8);
            cpu.set_ip16(next_ip);
        }
        0xB8..=0xBF => {
            // MOV r16/r32, imm16/imm32 — Spec: Intel SDM Vol. 2 "MOV"; Ch. 2 (66H).
            let idx = (op - 0xB8) as usize;
            if opsz32(&insn) {
                cpu.set_gpr_u32(idx, insn.immediate as u32);
            } else {
                cpu.set_gpr_u16(idx, insn.immediate as u16);
            }
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
            // MOV r16/r32, r/m16|32 — Spec: Intel SDM Vol. 2 "MOV"; Ch. 2 (66H).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if opsz32(&insn) {
                let v = read_rm_u32(cpu, bus, &insn)?;
                cpu.set_gpr_u32(m.reg as usize, v);
            } else {
                let v = read_rm_u16(cpu, bus, &insn)?;
                cpu.set_gpr_u16(m.reg as usize, v);
            }
            cpu.set_ip16(next_ip);
        }
        0x89 => {
            // MOV r/m16|32, r16/r32 — Spec: Intel SDM Vol. 2 "MOV"; Ch. 2 (66H).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if opsz32(&insn) {
                let v = cpu.gpr_u32(m.reg as usize);
                write_rm_u32(cpu, bus, &insn, v)?;
            } else {
                let v = cpu.gpr_u16(m.reg as usize);
                write_rm_u16(cpu, bus, &insn, v)?;
            }
            cpu.set_ip16(next_ip);
        }
        0x8C => {
            // MOV r/m16|r32, Sreg — Spec: Intel SDM Vol. 2 "MOV" (r/m16, Sreg); Ch. 2.
            // OsZ32 + register dest: zero-extend selector into r32.
            // Memory dest always stores 16 bits (selector width), even with 0x66.
            // Reserved Sreg encodings (reg=6,7) → #UD (Vol. 3 §6.15).
            // Unsupported here: protected-mode side effects.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let Some(sreg) = sreg_from_modrm_reg(m.reg) else {
                return real_mode_ud(cpu, bus);
            };
            let v = read_sreg_selector(cpu, sreg);
            if opsz32(&insn) && m.mod_ == 3 {
                cpu.set_gpr_u32(m.rm as usize, u32::from(v));
            } else {
                write_rm_u16(cpu, bus, &insn, v)?;
            }
            cpu.set_ip16(next_ip);
        }
        0x8D => {
            // LEA r16/r32, m — load effective address (offset only; no memory read).
            // Spec: Intel SDM Vol. 2 "LEA"; Vol. 1 §3.6 (address-/operand-size).
            // Register source (mod=11) → #UD (Vol. 3 §6.15).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if m.mod_ == 3 {
                return real_mode_ud(cpu, bus);
            }
            if asize32(&insn) {
                let off = calc_ea32(cpu, &insn)?;
                if opsz32(&insn) {
                    cpu.set_gpr_u32(m.reg as usize, off);
                } else {
                    cpu.set_gpr_u16(m.reg as usize, off as u16);
                }
            } else {
                let off = calc_ea16(cpu, m.mod_, m.rm, insn.displacement)?;
                if opsz32(&insn) {
                    cpu.set_gpr_u32(m.reg as usize, u32::from(off));
                } else {
                    cpu.set_gpr_u16(m.reg as usize, off);
                }
            }
            cpu.set_ip16(next_ip);
        }
        0x8E => {
            // MOV Sreg, r/m16 — real-address mode load (base = selector << 4).
            // Spec: Intel SDM Vol. 2 "MOV" (Sreg, r/m16); Vol. 3 §3.4.2.
            // MOV to CS and reserved Sreg encodings → #UD (Vol. 3 §6.15).
            // Unsupported here: protected-mode descriptor checks; IRQ inhibit after MOV SS.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let Some(sreg) = sreg_from_modrm_reg(m.reg) else {
                return real_mode_ud(cpu, bus);
            };
            if sreg == 1 {
                // MOV to CS is invalid (#UD). Spec: Intel SDM Vol. 2 "MOV".
                return real_mode_ud(cpu, bus);
            }
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
            if opsz32(&insn) {
                if op == 0x31 {
                    let a = read_rm_u32(cpu, bus, &insn)?;
                    let b = cpu.gpr_u32(m.reg as usize);
                    let r = a ^ b;
                    write_rm_u32(cpu, bus, &insn, r)?;
                    set_logic_flags_u32(cpu, r);
                } else {
                    let a = cpu.gpr_u32(m.reg as usize);
                    let b = read_rm_u32(cpu, bus, &insn)?;
                    let r = a ^ b;
                    cpu.set_gpr_u32(m.reg as usize, r);
                    set_logic_flags_u32(cpu, r);
                }
            } else if op == 0x31 {
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
            if opsz32(&insn) {
                if op == 0x01 {
                    let a = read_rm_u32(cpu, bus, &insn)?;
                    let b = cpu.gpr_u32(m.reg as usize);
                    let r = a.wrapping_add(b);
                    write_rm_u32(cpu, bus, &insn, r)?;
                    set_add_flags_u32(cpu, a, b, r);
                } else {
                    let a = cpu.gpr_u32(m.reg as usize);
                    let b = read_rm_u32(cpu, bus, &insn)?;
                    let r = a.wrapping_add(b);
                    cpu.set_gpr_u32(m.reg as usize, r);
                    set_add_flags_u32(cpu, a, b, r);
                }
            } else if op == 0x01 {
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
            if opsz32(&insn) {
                if op == 0x29 {
                    let a = read_rm_u32(cpu, bus, &insn)?;
                    let b = cpu.gpr_u32(m.reg as usize);
                    let r = a.wrapping_sub(b);
                    write_rm_u32(cpu, bus, &insn, r)?;
                    set_sub_flags_u32(cpu, a, b, r);
                } else {
                    let a = cpu.gpr_u32(m.reg as usize);
                    let b = read_rm_u32(cpu, bus, &insn)?;
                    let r = a.wrapping_sub(b);
                    cpu.set_gpr_u32(m.reg as usize, r);
                    set_sub_flags_u32(cpu, a, b, r);
                }
            } else if op == 0x29 {
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
            if opsz32(&insn) {
                if op == 0x39 {
                    let a = read_rm_u32(cpu, bus, &insn)?;
                    let b = cpu.gpr_u32(m.reg as usize);
                    set_sub_flags_u32(cpu, a, b, a.wrapping_sub(b));
                } else {
                    let a = cpu.gpr_u32(m.reg as usize);
                    let b = read_rm_u32(cpu, bus, &insn)?;
                    set_sub_flags_u32(cpu, a, b, a.wrapping_sub(b));
                }
            } else if op == 0x39 {
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
        // ADD AX/EAX,imm — Spec: Intel SDM Vol. 2 "ADD" (05 iw/id); Ch. 2 (66H).
        0x05 => {
            if opsz32(&insn) {
                let a = cpu.eax();
                let b = insn.immediate as u32;
                let r = a.wrapping_add(b);
                cpu.set_eax(r);
                set_add_flags_u32(cpu, a, b, r);
            } else {
                let a = cpu.ax();
                let b = insn.immediate as u16;
                let r = a.wrapping_add(b);
                cpu.set_ax(r);
                set_add_flags_u16(cpu, a, b, r);
            }
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
            if opsz32(&insn) {
                let r = cpu.eax() | (insn.immediate as u32);
                cpu.set_eax(r);
                set_logic_flags_u32(cpu, r);
            } else {
                let r = cpu.ax() | (insn.immediate as u16);
                cpu.set_ax(r);
                set_logic_flags_u16(cpu, r);
            }
            cpu.set_ip16(next_ip);
        }
        0x24 => {
            let r = cpu.al() & (insn.immediate as u8);
            cpu.set_al(r);
            set_logic_flags_u8(cpu, r);
            cpu.set_ip16(next_ip);
        }
        0x25 => {
            if opsz32(&insn) {
                let r = cpu.eax() & (insn.immediate as u32);
                cpu.set_eax(r);
                set_logic_flags_u32(cpu, r);
            } else {
                let r = cpu.ax() & (insn.immediate as u16);
                cpu.set_ax(r);
                set_logic_flags_u16(cpu, r);
            }
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
            let cf_in = cpu.rflags & 1 != 0;
            if opsz32(&insn) {
                let a = cpu.eax();
                let b = insn.immediate as u32;
                let r = a.wrapping_add(b).wrapping_add(u32::from(cf_in));
                cpu.set_eax(r);
                set_adc_flags_u32(cpu, a, b, cf_in, r);
            } else {
                let a = cpu.ax();
                let b = insn.immediate as u16;
                let r = a.wrapping_add(b).wrapping_add(u16::from(cf_in));
                cpu.set_ax(r);
                set_adc_flags_u16(cpu, a, b, cf_in, r);
            }
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
            let cf_in = cpu.rflags & 1 != 0;
            if opsz32(&insn) {
                let a = cpu.eax();
                let b = insn.immediate as u32;
                let r = a.wrapping_sub(b).wrapping_sub(u32::from(cf_in));
                cpu.set_eax(r);
                set_sbb_flags_u32(cpu, a, b, cf_in, r);
            } else {
                let a = cpu.ax();
                let b = insn.immediate as u16;
                let r = a.wrapping_sub(b).wrapping_sub(u16::from(cf_in));
                cpu.set_ax(r);
                set_sbb_flags_u16(cpu, a, b, cf_in, r);
            }
            cpu.set_ip16(next_ip);
        }
        // SUB/XOR/CMP AL/AX/EAX,imm — Spec: Intel SDM Vol. 2 accumulator forms; Ch. 2.
        // BCD adjust — Spec: Intel SDM Vol. 2 DAA/DAS/AAA/AAS.
        // Unsupported here: 64-bit mode (#UD); INTO/BOUND (separate opcodes).
        0x27 => {
            exec_daa(cpu);
            cpu.set_ip16(next_ip);
        }
        0x2F => {
            exec_das(cpu);
            cpu.set_ip16(next_ip);
        }
        0x37 => {
            exec_aaa(cpu);
            cpu.set_ip16(next_ip);
        }
        0x3F => {
            exec_aas(cpu);
            cpu.set_ip16(next_ip);
        }
        0x2C => {
            let a = cpu.al();
            let b = insn.immediate as u8;
            let r = a.wrapping_sub(b);
            cpu.set_al(r);
            set_sub_flags_u8(cpu, a, b, r);
            cpu.set_ip16(next_ip);
        }
        0x2D => {
            if opsz32(&insn) {
                let a = cpu.eax();
                let b = insn.immediate as u32;
                let r = a.wrapping_sub(b);
                cpu.set_eax(r);
                set_sub_flags_u32(cpu, a, b, r);
            } else {
                let a = cpu.ax();
                let b = insn.immediate as u16;
                let r = a.wrapping_sub(b);
                cpu.set_ax(r);
                set_sub_flags_u16(cpu, a, b, r);
            }
            cpu.set_ip16(next_ip);
        }
        0x34 => {
            let r = cpu.al() ^ (insn.immediate as u8);
            cpu.set_al(r);
            set_logic_flags_u8(cpu, r);
            cpu.set_ip16(next_ip);
        }
        0x35 => {
            if opsz32(&insn) {
                let r = cpu.eax() ^ (insn.immediate as u32);
                cpu.set_eax(r);
                set_logic_flags_u32(cpu, r);
            } else {
                let r = cpu.ax() ^ (insn.immediate as u16);
                cpu.set_ax(r);
                set_logic_flags_u16(cpu, r);
            }
            cpu.set_ip16(next_ip);
        }
        0x3C => {
            let a = cpu.al();
            let b = insn.immediate as u8;
            set_sub_flags_u8(cpu, a, b, a.wrapping_sub(b));
            cpu.set_ip16(next_ip);
        }
        0x3D => {
            if opsz32(&insn) {
                let a = cpu.eax();
                let b = insn.immediate as u32;
                set_sub_flags_u32(cpu, a, b, a.wrapping_sub(b));
            } else {
                let a = cpu.ax();
                let b = insn.immediate as u16;
                set_sub_flags_u16(cpu, a, b, a.wrapping_sub(b));
            }
            cpu.set_ip16(next_ip);
        }
        // ADC/SBB ModRM — Spec: Intel SDM Vol. 2 "ADC" / "SBB"; Ch. 2 (66H).
        // Unsupported here: LOCK; segment-limit faults.
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
            let cf_in = cpu.rflags & 1 != 0;
            if opsz32(&insn) {
                let a = read_rm_u32(cpu, bus, &insn)?;
                let b = cpu.gpr_u32(m.reg as usize);
                let r = a.wrapping_add(b).wrapping_add(u32::from(cf_in));
                write_rm_u32(cpu, bus, &insn, r)?;
                set_adc_flags_u32(cpu, a, b, cf_in, r);
            } else {
                let a = read_rm_u16(cpu, bus, &insn)?;
                let b = cpu.gpr_u16(m.reg as usize);
                let r = a.wrapping_add(b).wrapping_add(u16::from(cf_in));
                write_rm_u16(cpu, bus, &insn, r)?;
                set_adc_flags_u16(cpu, a, b, cf_in, r);
            }
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
            let cf_in = cpu.rflags & 1 != 0;
            if opsz32(&insn) {
                let a = cpu.gpr_u32(m.reg as usize);
                let b = read_rm_u32(cpu, bus, &insn)?;
                let r = a.wrapping_add(b).wrapping_add(u32::from(cf_in));
                cpu.set_gpr_u32(m.reg as usize, r);
                set_adc_flags_u32(cpu, a, b, cf_in, r);
            } else {
                let a = cpu.gpr_u16(m.reg as usize);
                let b = read_rm_u16(cpu, bus, &insn)?;
                let r = a.wrapping_add(b).wrapping_add(u16::from(cf_in));
                cpu.set_gpr_u16(m.reg as usize, r);
                set_adc_flags_u16(cpu, a, b, cf_in, r);
            }
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
            let cf_in = cpu.rflags & 1 != 0;
            if opsz32(&insn) {
                let a = read_rm_u32(cpu, bus, &insn)?;
                let b = cpu.gpr_u32(m.reg as usize);
                let r = a.wrapping_sub(b).wrapping_sub(u32::from(cf_in));
                write_rm_u32(cpu, bus, &insn, r)?;
                set_sbb_flags_u32(cpu, a, b, cf_in, r);
            } else {
                let a = read_rm_u16(cpu, bus, &insn)?;
                let b = cpu.gpr_u16(m.reg as usize);
                let r = a.wrapping_sub(b).wrapping_sub(u16::from(cf_in));
                write_rm_u16(cpu, bus, &insn, r)?;
                set_sbb_flags_u16(cpu, a, b, cf_in, r);
            }
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
            let cf_in = cpu.rflags & 1 != 0;
            if opsz32(&insn) {
                let a = cpu.gpr_u32(m.reg as usize);
                let b = read_rm_u32(cpu, bus, &insn)?;
                let r = a.wrapping_sub(b).wrapping_sub(u32::from(cf_in));
                cpu.set_gpr_u32(m.reg as usize, r);
                set_sbb_flags_u32(cpu, a, b, cf_in, r);
            } else {
                let a = cpu.gpr_u16(m.reg as usize);
                let b = read_rm_u16(cpu, bus, &insn)?;
                let r = a.wrapping_sub(b).wrapping_sub(u16::from(cf_in));
                cpu.set_gpr_u16(m.reg as usize, r);
                set_sbb_flags_u16(cpu, a, b, cf_in, r);
            }
            cpu.set_ip16(next_ip);
        }
        // OR/AND ModRM — Spec: Intel SDM Vol. 2 "OR" / "AND"; Ch. 2 (66H).
        // Unsupported here: LOCK; segment-limit faults.
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
            if opsz32(&insn) {
                let a = read_rm_u32(cpu, bus, &insn)?;
                let b = cpu.gpr_u32(m.reg as usize);
                let r = a | b;
                write_rm_u32(cpu, bus, &insn, r)?;
                set_logic_flags_u32(cpu, r);
            } else {
                let a = read_rm_u16(cpu, bus, &insn)?;
                let b = cpu.gpr_u16(m.reg as usize);
                let r = a | b;
                write_rm_u16(cpu, bus, &insn, r)?;
                set_logic_flags_u16(cpu, r);
            }
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
            if opsz32(&insn) {
                let a = cpu.gpr_u32(m.reg as usize);
                let b = read_rm_u32(cpu, bus, &insn)?;
                let r = a | b;
                cpu.set_gpr_u32(m.reg as usize, r);
                set_logic_flags_u32(cpu, r);
            } else {
                let a = cpu.gpr_u16(m.reg as usize);
                let b = read_rm_u16(cpu, bus, &insn)?;
                let r = a | b;
                cpu.set_gpr_u16(m.reg as usize, r);
                set_logic_flags_u16(cpu, r);
            }
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
            if opsz32(&insn) {
                let a = read_rm_u32(cpu, bus, &insn)?;
                let b = cpu.gpr_u32(m.reg as usize);
                let r = a & b;
                write_rm_u32(cpu, bus, &insn, r)?;
                set_logic_flags_u32(cpu, r);
            } else {
                let a = read_rm_u16(cpu, bus, &insn)?;
                let b = cpu.gpr_u16(m.reg as usize);
                let r = a & b;
                write_rm_u16(cpu, bus, &insn, r)?;
                set_logic_flags_u16(cpu, r);
            }
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
            if opsz32(&insn) {
                let a = cpu.gpr_u32(m.reg as usize);
                let b = read_rm_u32(cpu, bus, &insn)?;
                let r = a & b;
                cpu.set_gpr_u32(m.reg as usize, r);
                set_logic_flags_u32(cpu, r);
            } else {
                let a = cpu.gpr_u16(m.reg as usize);
                let b = read_rm_u16(cpu, bus, &insn)?;
                let r = a & b;
                cpu.set_gpr_u16(m.reg as usize, r);
                set_logic_flags_u16(cpu, r);
            }
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

    /// Test bus: latch an external IRQ after N successful `write_u8` calls.
    /// Used to exercise REP interruptibility between iterations (PIC stub).
    struct IrqAfterWritesBus {
        mem: Vec<u8>,
        ports: Vec<u8>,
        writes: usize,
        inject_after_writes: usize,
        inject_vector: u8,
        latched: Option<u8>,
    }

    impl Bus for IrqAfterWritesBus {
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
            self.writes = self.writes.saturating_add(1);
            if self.writes == self.inject_after_writes {
                self.latched = Some(self.inject_vector);
            }
            Ok(())
        }
        fn port_in_u8(&mut self, _port: u16) -> Result<u8, ExecError> {
            Ok(0xFF)
        }
        fn port_out_u8(&mut self, _port: u16, val: u8) -> Result<(), ExecError> {
            self.ports.push(val);
            Ok(())
        }
        fn poll_external_irq(&mut self) -> Option<u8> {
            self.latched.take()
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

    /// `#NMI` vector 2 via IVT; not gated by IF (SDM Vol. 3 §6.3.3 / §6.7 / §6.4).
    #[test]
    fn nmi_delivers_vector_2_ignoring_if() {
        let mut mem = vec![0u8; 0x10000];
        // IVT[2] at offset 8 → handler 0000:0x0800
        mem[8] = 0x00;
        mem[9] = 0x08;
        mem[10] = 0x00;
        mem[11] = 0x00;
        mem[0x800] = 0xF4; // HLT
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0x1000;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(false);
        cpu.request_nmi();
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert!(!cpu.pending_nmi);
        assert_eq!(cpu.cs.selector, 0);
        assert_eq!(cpu.ip16(), 0x0800);
        assert!(!cpu.interrupt_flag());
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF8);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0x1000); // return IP
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

    /// MOV CS, r/m16 is invalid (#UD) — delivered via IVT vector 6 (SDM Vol. 2 MOV; Vol. 3 §6.15).
    #[test]
    fn mov_to_cs_ud_via_ivt() {
        let mut mem = vec![0u8; 0x10000];
        // IVT[6] → 0000:0B00
        mem[24] = 0x00;
        mem[25] = 0x0B;
        mem[26] = 0x00;
        mem[27] = 0x00;
        // 8E C8 = MOV CS, AX
        mem[0] = 0x8E;
        mem[1] = 0xC8;
        mem[0xB00] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_gpr_u16(CpuState::RAX, 0x1000);
        cpu.set_interrupt_flag(true);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.cs.selector, 0);
        assert_eq!(cpu.ip16(), 0x0B00);
        assert!(!cpu.interrupt_flag());
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0); // fault IP
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

    /// REP/REPE/REPNE on string byte ops (SDM Vol. 2 REP/REPE/REPNE + MOVS/STOS/LODS/SCAS/CMPS).
    #[test]
    fn rep_stosb_cx_zero_is_nop() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0xAA; // REP STOSB
        mem[2] = 0xF4;
        mem[0x2000] = 0x55;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_al(0xAA);
        cpu.set_gpr_u16(CpuState::RCX, 0);
        cpu.set_gpr_u16(CpuState::RDI, 0x2000);
        cpu.set_direction_flag(false);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x2000).unwrap(), 0x55); // unchanged
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x2000);
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.ip16(), 2);
    }

    #[test]
    fn rep_stosb_fills_and_clears_cx() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0xAA; // REP STOSB
        mem[2] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_al(b'Z');
        cpu.set_gpr_u16(CpuState::RCX, 3);
        cpu.set_gpr_u16(CpuState::RDI, 0x3000);
        cpu.set_direction_flag(false);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x3000).unwrap(), b'Z');
        assert_eq!(bus.read_u8(0x3001).unwrap(), b'Z');
        assert_eq!(bus.read_u8(0x3002).unwrap(), b'Z');
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x3003);
        assert_eq!(cpu.ip16(), 2);
    }

    /// REP is interruptible between iterations when IF=1.
    /// Spec: Intel SDM Vol. 2 "REP/REPE/REPNE" — service pending interrupts
    /// before each string iteration; saved IP points at the string insn;
    /// CX/SI/DI reflect the last completed iteration.
    #[test]
    fn rep_stosb_external_irq_before_first_iteration() {
        let mut mem = vec![0u8; 0x10000];
        // IVT[0x20] → 0000:0E00
        mem[0x20 * 4] = 0x00;
        mem[0x20 * 4 + 1] = 0x0E;
        mem[0x20 * 4 + 2] = 0x00;
        mem[0x20 * 4 + 3] = 0x00;
        mem[0] = 0xF3;
        mem[1] = 0xAA; // REP STOSB
        mem[0xE00] = 0xF4; // handler HLT
        mem[0x3000] = 0x11;
        mem[0x3001] = 0x22;
        mem[0x3002] = 0x33;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_al(b'Z');
        cpu.set_gpr_u16(CpuState::RCX, 3);
        cpu.set_gpr_u16(CpuState::RDI, 0x3000);
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_direction_flag(false);
        cpu.set_interrupt_flag(true);
        cpu.request_interrupt(0x20);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();

        // Interrupted before any store (SDM poll at iteration start).
        assert_eq!(bus.read_u8(0x3000).unwrap(), 0x11);
        assert_eq!(bus.read_u8(0x3001).unwrap(), 0x22);
        assert_eq!(bus.read_u8(0x3002).unwrap(), 0x33);
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 3);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x3000);
        assert_eq!(cpu.ip16(), 0x0E00);
        assert!(!cpu.interrupt_flag());
        assert_eq!(cpu.pending_irq, None);
        // Saved IP = REP STOSB start.
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0);
    }

    /// IF=0: pending IRQ stays latched; REP runs to completion.
    #[test]
    fn rep_stosb_pending_irq_ignored_when_if_clear() {
        let mut mem = vec![0u8; 0x10000];
        mem[0x20 * 4] = 0x00;
        mem[0x20 * 4 + 1] = 0x0E;
        mem[0x20 * 4 + 2] = 0x00;
        mem[0x20 * 4 + 3] = 0x00;
        mem[0] = 0xF3;
        mem[1] = 0xAA; // REP STOSB
        mem[2] = 0xF4;
        mem[0xE00] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_al(b'Q');
        cpu.set_gpr_u16(CpuState::RCX, 2);
        cpu.set_gpr_u16(CpuState::RDI, 0x3000);
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_direction_flag(false);
        cpu.set_interrupt_flag(false);
        cpu.request_interrupt(0x20);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(bus.read_u8(0x3000).unwrap(), b'Q');
        assert_eq!(bus.read_u8(0x3001).unwrap(), b'Q');
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.ip16(), 2);
        assert_eq!(cpu.pending_irq, Some(0x20));
    }

    /// Bus-latched IRQ after first STOS write → suspend before second iteration.
    /// Spec: SDM Vol. 2 REP — CX/DI reflect last successful iteration; IP = string insn.
    #[test]
    fn rep_stosb_irq_between_iterations_via_bus_poll() {
        let mut mem = vec![0u8; 0x10000];
        mem[0x21 * 4] = 0x00;
        mem[0x21 * 4 + 1] = 0x0F;
        mem[0x21 * 4 + 2] = 0x00;
        mem[0x21 * 4 + 3] = 0x00;
        mem[0] = 0xF3;
        mem[1] = 0xAA; // REP STOSB
        mem[0xF00] = 0xCF; // IRET — resume REP

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_al(b'Z');
        cpu.set_gpr_u16(CpuState::RCX, 3);
        cpu.set_gpr_u16(CpuState::RDI, 0x3000);
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_direction_flag(false);
        cpu.set_interrupt_flag(true);

        let mut bus = IrqAfterWritesBus {
            mem,
            ports: vec![],
            writes: 0,
            inject_after_writes: 1,
            inject_vector: 0x21,
            latched: None,
        };
        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(bus.mem[0x3000], b'Z');
        assert_eq!(bus.mem[0x3001], 0); // not yet
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 2);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x3001);
        assert_eq!(cpu.ip16(), 0x0F00);
        assert!(!cpu.interrupt_flag());
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0); // resume at REP STOSB

        // IRET then finish remaining two stores.
        step(&mut cpu, &mut bus).unwrap(); // IRET
        assert!(cpu.interrupt_flag());
        assert_eq!(cpu.ip16(), 0);
        step(&mut cpu, &mut bus).unwrap(); // remaining REP
        assert_eq!(bus.mem[0x3001], b'Z');
        assert_eq!(bus.mem[0x3002], b'Z');
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x3003);
        assert_eq!(cpu.ip16(), 2);
    }

    #[test]
    fn rep_movsb_df_backward() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0xA4; // REP MOVSB
        mem[2] = 0xF4;
        mem[0x1010] = b'A';
        mem[0x100F] = b'B';
        mem[0x100E] = b'C';

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RCX, 3);
        cpu.set_gpr_u16(CpuState::RSI, 0x1010);
        cpu.set_gpr_u16(CpuState::RDI, 0x2010);
        cpu.set_direction_flag(true);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x2010).unwrap(), b'A');
        assert_eq!(bus.read_u8(0x200F).unwrap(), b'B');
        assert_eq!(bus.read_u8(0x200E).unwrap(), b'C');
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x100D);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x200D);
    }

    #[test]
    fn rep_lodsb_loads_last_byte_into_al() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0xAC; // REP LODSB
        mem[2] = 0xF4;
        mem[0x4000] = 0x11;
        mem[0x4001] = 0x22;
        mem[0x4002] = 0x33;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RCX, 3);
        cpu.set_gpr_u16(CpuState::RSI, 0x4000);
        cpu.set_direction_flag(false);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x33);
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x4003);
    }

    #[test]
    fn repe_scasb_stops_on_mismatch() {
        // REPE SCASB: repeat while ZF=1; stop early on first mismatch.
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0xAE; // REPE SCASB
        mem[2] = 0xF4;
        mem[0x5000] = b'x';
        mem[0x5001] = b'x';
        mem[0x5002] = b'y'; // mismatch
        mem[0x5003] = b'x';

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_al(b'x');
        cpu.set_gpr_u16(CpuState::RCX, 4);
        cpu.set_gpr_u16(CpuState::RDI, 0x5000);
        cpu.set_direction_flag(false);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 1); // 4→3→2→1 after mismatch at third
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x5003);
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF=0
    }

    #[test]
    fn repne_scasb_stops_on_match() {
        // REPNE SCASB: repeat while ZF=0; stop when equal found.
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF2;
        mem[1] = 0xAE; // REPNE SCASB
        mem[2] = 0xF4;
        mem[0x6000] = b'a';
        mem[0x6001] = b'b';
        mem[0x6002] = b'Q'; // match AL
        mem[0x6003] = b'c';

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_al(b'Q');
        cpu.set_gpr_u16(CpuState::RCX, 4);
        cpu.set_gpr_u16(CpuState::RDI, 0x6000);
        cpu.set_direction_flag(false);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 1);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x6003);
        assert_ne!(cpu.rflags & (1 << 6), 0); // ZF=1
    }

    #[test]
    fn repe_cmpsb_compares_strings() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0xA6; // REPE CMPSB
        mem[2] = 0xF4;
        mem[0x7000] = 1;
        mem[0x7001] = 2;
        mem[0x7002] = 3;
        mem[0x8000] = 1;
        mem[0x8001] = 2;
        mem[0x8002] = 9; // mismatch

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RCX, 3);
        cpu.set_gpr_u16(CpuState::RSI, 0x7000);
        cpu.set_gpr_u16(CpuState::RDI, 0x8000);
        cpu.set_direction_flag(false);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x7003);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x8003);
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF=0 after mismatch
    }

    /// LODSW/STOSW/MOVSW advance SI/DI by ±2 per DF (SDM Vol. 2 LODS/STOS/MOVS).
    #[test]
    fn string_word_ops_df_forward() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xAD; // LODSW
        mem[1] = 0xAB; // STOSW
        mem[2] = 0xA5; // MOVSW
        mem[3] = 0xF4;
        // little-endian words at DS:1000
        mem[0x1000] = 0x34;
        mem[0x1001] = 0x12; // 0x1234
        mem[0x1002] = 0x78;
        mem[0x1003] = 0x56; // 0x5678

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_direction_flag(false);
        cpu.set_gpr_u16(CpuState::RSI, 0x1000);
        cpu.set_gpr_u16(CpuState::RDI, 0x2000);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap(); // LODSW
        assert_eq!(cpu.ax(), 0x1234);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x1002);

        step(&mut cpu, &mut bus).unwrap(); // STOSW
        assert_eq!(bus.read_u16(0x2000).unwrap(), 0x1234);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x2002);

        // MOVSW: DS:[SI]=0x5678 → ES:[DI]
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x2002).unwrap(), 0x5678);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x1004);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x2004);
    }

    #[test]
    fn lodsw_df_backward() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xAD;
        mem[1] = 0xF4;
        mem[0x1000] = 0xCD;
        mem[0x1001] = 0xAB; // 0xABCD

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_direction_flag(true);
        cpu.set_gpr_u16(CpuState::RSI, 0x1000);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0xABCD);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x0FFE);
    }

    #[test]
    fn rep_stosw_fills_and_clears_cx() {
        // Spec: Intel SDM Vol. 2 STOS + REP/REPE/REPNE
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0xAB; // REP STOSW
        mem[2] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_ax(0xBEEF);
        cpu.set_gpr_u16(CpuState::RCX, 3);
        cpu.set_gpr_u16(CpuState::RDI, 0x3000);
        cpu.set_direction_flag(false);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x3000).unwrap(), 0xBEEF);
        assert_eq!(bus.read_u16(0x3002).unwrap(), 0xBEEF);
        assert_eq!(bus.read_u16(0x3004).unwrap(), 0xBEEF);
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x3006);
        assert_eq!(cpu.ip16(), 2);
    }

    #[test]
    fn rep_movsw_df_backward() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0xA5; // REP MOVSW
        mem[2] = 0xF4;
        // Words at SI=0x1010, 0x100E, 0x100C (DF=1 steps −2).
        mem[0x1010] = 0xAA;
        mem[0x1011] = 0x11; // 0x11AA
        mem[0x100E] = 0xBB;
        mem[0x100F] = 0x22; // 0x22BB
        mem[0x100C] = 0xCC;
        mem[0x100D] = 0x33; // 0x33CC

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RCX, 3);
        cpu.set_gpr_u16(CpuState::RSI, 0x1010);
        cpu.set_gpr_u16(CpuState::RDI, 0x2010);
        cpu.set_direction_flag(true);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x2010).unwrap(), 0x11AA);
        assert_eq!(bus.read_u16(0x200E).unwrap(), 0x22BB);
        assert_eq!(bus.read_u16(0x200C).unwrap(), 0x33CC);
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x100A);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x200A);
    }

    #[test]
    fn rep_lodsw_loads_last_word_into_ax() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0xAD; // REP LODSW
        mem[2] = 0xF4;
        mem[0x4000] = 0x11;
        mem[0x4001] = 0x11;
        mem[0x4002] = 0x22;
        mem[0x4003] = 0x22;
        mem[0x4004] = 0x33;
        mem[0x4005] = 0x33;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RCX, 3);
        cpu.set_gpr_u16(CpuState::RSI, 0x4000);
        cpu.set_direction_flag(false);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x3333);
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x4006);
    }

    #[test]
    fn repe_scasw_stops_on_mismatch() {
        // Spec: Intel SDM Vol. 2 SCAS + REPE
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0xAF; // REPE SCASW
        mem[2] = 0xF4;
        mem[0x5000] = 0x78;
        mem[0x5001] = 0x56; // 0x5678 match
        mem[0x5002] = 0x78;
        mem[0x5003] = 0x56; // match
        mem[0x5004] = 0x00;
        mem[0x5005] = 0x00; // mismatch
        mem[0x5006] = 0x78;
        mem[0x5007] = 0x56;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_ax(0x5678);
        cpu.set_gpr_u16(CpuState::RCX, 4);
        cpu.set_gpr_u16(CpuState::RDI, 0x5000);
        cpu.set_direction_flag(false);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 1);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x5006);
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF=0
    }

    #[test]
    fn repne_scasw_stops_on_match() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF2;
        mem[1] = 0xAF; // REPNE SCASW
        mem[2] = 0xF4;
        mem[0x6000] = 0x01;
        mem[0x6001] = 0x00;
        mem[0x6002] = 0x02;
        mem[0x6003] = 0x00;
        mem[0x6004] = 0x51;
        mem[0x6005] = 0x51; // match AX
        mem[0x6006] = 0x03;
        mem[0x6007] = 0x00;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_ax(0x5151);
        cpu.set_gpr_u16(CpuState::RCX, 4);
        cpu.set_gpr_u16(CpuState::RDI, 0x6000);
        cpu.set_direction_flag(false);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 1);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x6006);
        assert_ne!(cpu.rflags & (1 << 6), 0); // ZF=1
    }

    #[test]
    fn repe_cmpsw_compares_words() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0xA7; // REPE CMPSW
        mem[2] = 0xF4;
        mem[0x7000] = 0x01;
        mem[0x7001] = 0x00;
        mem[0x7002] = 0x02;
        mem[0x7003] = 0x00;
        mem[0x7004] = 0x03;
        mem[0x7005] = 0x00;
        mem[0x8000] = 0x01;
        mem[0x8001] = 0x00;
        mem[0x8002] = 0x02;
        mem[0x8003] = 0x00;
        mem[0x8004] = 0x09;
        mem[0x8005] = 0x00; // mismatch

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RCX, 3);
        cpu.set_gpr_u16(CpuState::RSI, 0x7000);
        cpu.set_gpr_u16(CpuState::RDI, 0x8000);
        cpu.set_direction_flag(false);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x7006);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x8006);
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF=0 after mismatch
    }

    /// 0x66 A5 = MOVSD — dword element, SI/DI ±4 (SDM Vol. 2 MOVS + opsize).
    #[test]
    fn rep_movsd_opsize32() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0x66;
        mem[2] = 0xA5; // REP MOVSD
        mem[3] = 0xF4;
        // two dwords at 0x4000
        mem[0x4000] = 0x01;
        mem[0x4001] = 0x02;
        mem[0x4002] = 0x03;
        mem[0x4003] = 0x04; // 0x04030201
        mem[0x4004] = 0x11;
        mem[0x4005] = 0x22;
        mem[0x4006] = 0x33;
        mem[0x4007] = 0x44; // 0x44332211

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RCX, 2);
        cpu.set_gpr_u16(CpuState::RSI, 0x4000);
        cpu.set_gpr_u16(CpuState::RDI, 0x5000);
        cpu.set_direction_flag(false);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u32(0x5000).unwrap(), 0x0403_0201);
        assert_eq!(bus.read_u32(0x5004).unwrap(), 0x4433_2211);
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x4008);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x5008);
    }

    #[test]
    fn stosd_opsize32_writes_eax() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x66;
        mem[1] = 0xAB; // STOSD
        mem[2] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_eax(0xDEAD_BEEF);
        cpu.set_gpr_u16(CpuState::RDI, 0x2100);
        cpu.set_direction_flag(false);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u32(0x2100).unwrap(), 0xDEAD_BEEF);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x2104);
    }

    /// Port bus with sequenced IN bytes and recorded OUT traffic for INS/OUTS tests.
    struct PortSeqBus {
        mem: Vec<u8>,
        in_bytes: Vec<u8>,
        in_idx: usize,
        /// Recorded (port, size, value) outs.
        outs: Vec<(u16, u8, u32)>,
    }

    impl Bus for PortSeqBus {
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
            if self.in_idx >= self.in_bytes.len() {
                return Ok(0xFF);
            }
            let v = self.in_bytes[self.in_idx];
            self.in_idx += 1;
            Ok(v)
        }
        fn port_out_u8(&mut self, port: u16, val: u8) -> Result<(), ExecError> {
            self.outs.push((port, 1, u32::from(val)));
            Ok(())
        }
        fn port_in_u16(&mut self, port: u16) -> Result<u16, ExecError> {
            let lo = self.port_in_u8(port)?;
            let hi = self.port_in_u8(port.wrapping_add(1))?;
            Ok(u16::from_le_bytes([lo, hi]))
        }
        fn port_out_u16(&mut self, port: u16, val: u16) -> Result<(), ExecError> {
            self.outs.push((port, 2, u32::from(val)));
            Ok(())
        }
        fn port_in_u32(&mut self, port: u16) -> Result<u32, ExecError> {
            let lo = u32::from(self.port_in_u16(port)?);
            let hi = u32::from(self.port_in_u16(port.wrapping_add(2))?);
            Ok(lo | (hi << 16))
        }
        fn port_out_u32(&mut self, port: u16, val: u32) -> Result<(), ExecError> {
            self.outs.push((port, 4, val));
            Ok(())
        }
    }

    /// INSB: DX port → ES:[DI], DI ±1 by DF (SDM Vol. 2 INS/INSB/INSW/INSD).
    #[test]
    fn insb_df_forward() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x6C; // INSB
        mem[1] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RDX, 0x3F8);
        cpu.set_gpr_u16(CpuState::RDI, 0x2000);
        cpu.set_direction_flag(false);

        let mut bus = PortSeqBus {
            mem,
            in_bytes: vec![0x41],
            in_idx: 0,
            outs: vec![],
        };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x2000).unwrap(), 0x41);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x2001);
        assert_eq!(cpu.ip16(), 1);
    }

    #[test]
    fn insb_df_backward() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x6C;
        mem[1] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RDX, 0x60);
        cpu.set_gpr_u16(CpuState::RDI, 0x2000);
        cpu.set_direction_flag(true);

        let mut bus = PortSeqBus {
            mem,
            in_bytes: vec![0xAB],
            in_idx: 0,
            outs: vec![],
        };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x2000).unwrap(), 0xAB);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x1FFF);
    }

    /// OUTSB: DS:[SI] → DX port, SI ±1 by DF (SDM Vol. 2 OUTS/OUTSB/OUTSW/OUTSD).
    #[test]
    fn outsb_df_forward() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x6E; // OUTSB
        mem[1] = 0xF4;
        mem[0x1000] = b'Z';

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RDX, 0x3F8);
        cpu.set_gpr_u16(CpuState::RSI, 0x1000);
        cpu.set_direction_flag(false);

        let mut bus = PortSeqBus {
            mem,
            in_bytes: vec![],
            in_idx: 0,
            outs: vec![],
        };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.outs, [(0x3F8, 1, u32::from(b'Z'))]);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x1001);
        assert_eq!(cpu.ip16(), 1);
    }

    #[test]
    fn outsb_segment_override_es() {
        // Spec: SDM Vol. 2 OUTS — source may use segment override; dest port is DX.
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x26; // ES:
        mem[1] = 0x6E; // OUTSB
        mem[2] = 0xF4;
        mem[0x3000] = 0x55;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        // Put source data under ES base ≠ DS: use es.base via selector.
        cpu.es = x86_core::SegmentReg::real_mode(0x0300); // base 0x3000
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RDX, 0x402);
        cpu.set_gpr_u16(CpuState::RSI, 0); // ES:0 → linear 0x3000
        cpu.set_direction_flag(false);

        let mut bus = PortSeqBus {
            mem,
            in_bytes: vec![],
            in_idx: 0,
            outs: vec![],
        };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.outs, [(0x402, 1, 0x55)]);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 1);
    }

    #[test]
    fn rep_insb_fills_and_clears_cx() {
        // Spec: SDM Vol. 2 INS + REP/REPE/REPNE (count = CX in asize 16).
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0x6C; // REP INSB
        mem[2] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RDX, 0x60);
        cpu.set_gpr_u16(CpuState::RCX, 3);
        cpu.set_gpr_u16(CpuState::RDI, 0x4000);
        cpu.set_direction_flag(false);

        let mut bus = PortSeqBus {
            mem,
            in_bytes: vec![0x11, 0x22, 0x33],
            in_idx: 0,
            outs: vec![],
        };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x4000).unwrap(), 0x11);
        assert_eq!(bus.read_u8(0x4001).unwrap(), 0x22);
        assert_eq!(bus.read_u8(0x4002).unwrap(), 0x33);
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x4003);
        assert_eq!(cpu.ip16(), 2);
    }

    #[test]
    fn rep_outsb_cx_zero_is_nop() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0x6E; // REP OUTSB
        mem[2] = 0xF4;
        mem[0x1000] = 0x99;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RDX, 0x3F8);
        cpu.set_gpr_u16(CpuState::RCX, 0);
        cpu.set_gpr_u16(CpuState::RSI, 0x1000);
        cpu.set_direction_flag(false);

        let mut bus = PortSeqBus {
            mem,
            in_bytes: vec![],
            in_idx: 0,
            outs: vec![],
        };
        step(&mut cpu, &mut bus).unwrap();
        assert!(bus.outs.is_empty());
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x1000);
        assert_eq!(cpu.ip16(), 2);
    }

    #[test]
    fn rep_outsb_writes_and_clears_cx() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0x6E; // REP OUTSB
        mem[2] = 0xF4;
        mem[0x1000] = b'A';
        mem[0x1001] = b'B';
        mem[0x1002] = b'C';

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RDX, 0x3F8);
        cpu.set_gpr_u16(CpuState::RCX, 3);
        cpu.set_gpr_u16(CpuState::RSI, 0x1000);
        cpu.set_direction_flag(false);

        let mut bus = PortSeqBus {
            mem,
            in_bytes: vec![],
            in_idx: 0,
            outs: vec![],
        };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(
            bus.outs,
            [
                (0x3F8, 1, u32::from(b'A')),
                (0x3F8, 1, u32::from(b'B')),
                (0x3F8, 1, u32::from(b'C')),
            ]
        );
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x1003);
    }

    /// INSW/OUTSW: word port I/O, SI/DI ±2 (SDM Vol. 2 INS/OUTS).
    #[test]
    fn insw_outsw_df_forward() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x6D; // INSW
        mem[1] = 0x6F; // OUTSW
        mem[2] = 0xF4;
        // OUTSW source after INSW wrote 0x1234 at ES:2000; point SI there.
        // We'll set SI=0x2000 before OUTSW via separate setup — run step by step.

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RDX, 0x1F0);
        cpu.set_gpr_u16(CpuState::RDI, 0x2000);
        cpu.set_direction_flag(false);

        let mut bus = PortSeqBus {
            mem,
            // little-endian word 0x1234 via default port_in_u16 (port, port+1)
            in_bytes: vec![0x34, 0x12],
            in_idx: 0,
            outs: vec![],
        };
        step(&mut cpu, &mut bus).unwrap(); // INSW
        assert_eq!(bus.read_u16(0x2000).unwrap(), 0x1234);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x2002);

        cpu.set_gpr_u16(CpuState::RSI, 0x2000);
        step(&mut cpu, &mut bus).unwrap(); // OUTSW
        assert_eq!(bus.outs, [(0x1F0, 2, 0x1234)]);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x2002);
    }

    #[test]
    fn rep_insw_fills_words() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0x6D; // REP INSW
        mem[2] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RDX, 0x1F0);
        cpu.set_gpr_u16(CpuState::RCX, 2);
        cpu.set_gpr_u16(CpuState::RDI, 0x3000);
        cpu.set_direction_flag(false);

        let mut bus = PortSeqBus {
            mem,
            in_bytes: vec![0xEE, 0xBE, 0xAD, 0xDE], // 0xBEEE, 0xDEAD
            in_idx: 0,
            outs: vec![],
        };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x3000).unwrap(), 0xBEEE);
        assert_eq!(bus.read_u16(0x3002).unwrap(), 0xDEAD);
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x3004);
    }

    /// 0x66 6D/6F = INSD/OUTSD — dword element, DI/SI ±4 (SDM Vol. 2 INS/OUTS + opsize).
    #[test]
    fn rep_insd_outsd_opsize32() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0x66;
        mem[2] = 0x6D; // REP INSD
        mem[3] = 0x66;
        mem[4] = 0x6F; // OUTSD (single)
        mem[5] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RDX, 0x1F0);
        cpu.set_gpr_u16(CpuState::RCX, 1);
        cpu.set_gpr_u16(CpuState::RDI, 0x5000);
        cpu.set_direction_flag(false);

        let mut bus = PortSeqBus {
            mem,
            in_bytes: vec![0x01, 0x02, 0x03, 0x04], // 0x04030201
            in_idx: 0,
            outs: vec![],
        };
        step(&mut cpu, &mut bus).unwrap(); // REP INSD
        assert_eq!(bus.read_u32(0x5000).unwrap(), 0x0403_0201);
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x5004);

        cpu.set_gpr_u16(CpuState::RSI, 0x5000);
        step(&mut cpu, &mut bus).unwrap(); // OUTSD
        assert_eq!(bus.outs, [(0x1F0, 4, 0x0403_0201)]);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x5004);
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

    /// INTO: OF=0 falls through; OF=1 delivers #OF (vector 4) as a trap (return IP = next).
    /// Spec: Intel SDM Vol. 2 "INT n/INTO/INT3/INT1"; Vol. 3 §6.15 (#OF — trap).
    #[test]
    fn into_overflow_trap_via_ivt() {
        let mut mem = vec![0u8; 0x10000];
        // IVT[4] → 0000:0A00
        mem[0x10] = 0x00;
        mem[0x11] = 0x0A;
        mem[0x12] = 0x00;
        mem[0x13] = 0x00;
        mem[0] = 0xCE; // INTO
        mem[1] = 0xF4; // fall-through HLT when OF clear
        mem[0xA00] = 0xF4; // #OF handler

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_of(false);
        cpu.set_interrupt_flag(true);
        let flags_clear = cpu.rflags;
        let mut bus = VecBus { mem, ports: vec![] };

        // OF clear → no vectoring; IP advances past INTO
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 1);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE);
        assert_eq!(cpu.rflags, flags_clear);
        assert!(cpu.interrupt_flag());

        // OF set → vector 4; saved IP = next (trap), IF cleared
        cpu.rip = 0;
        cpu.set_of(true);
        cpu.set_interrupt_flag(true);
        let saved_flags = cpu.rflags as u16;
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.cs.selector, 0);
        assert_eq!(cpu.ip16(), 0x0A00);
        assert!(!cpu.interrupt_flag());
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF8);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 1); // return IP after INTO
        assert_eq!(bus.read_u16(0xFFFA).unwrap(), 0);
        assert_eq!(bus.read_u16(0xFFFC).unwrap(), saved_flags);
    }

    /// BOUND checks signed index against m16&16; #BR (vector 5) is a fault (IP = BOUND).
    /// Spec: Intel SDM Vol. 2 "BOUND"; Vol. 3 §6.15 (#BR — fault).
    #[test]
    fn bound_index_check_and_br_via_ivt() {
        let mut mem = vec![0u8; 0x10000];
        // IVT[5] → 0000:0B00
        mem[0x14] = 0x00;
        mem[0x15] = 0x0B;
        mem[0x16] = 0x00;
        mem[0x17] = 0x00;
        // Bounds at DS:0x2000 — lower=0x0010, upper=0x0020 (signed)
        mem[0x2000] = 0x10;
        mem[0x2001] = 0x00;
        mem[0x2002] = 0x20;
        mem[0x2003] = 0x00;
        // 62 06 00 20 = BOUND AX, [0x2000]
        mem[0] = 0x62;
        mem[1] = 0x06;
        mem[2] = 0x00;
        mem[3] = 0x20;
        mem[4] = 0xF4;
        mem[0xB00] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_ax(0x0015); // inside [0x10, 0x20]
        cpu.rflags = 0x246;
        let flags_before = cpu.rflags;
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 4);
        assert_eq!(cpu.ax(), 0x0015);
        assert_eq!(cpu.rflags, flags_before);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE);

        // Below lower bound → #BR; fault IP = 0
        cpu.rip = 0;
        cpu.set_ax(0x000F);
        cpu.set_interrupt_flag(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0B00);
        assert!(!cpu.interrupt_flag());
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0);
        assert_eq!(cpu.ax(), 0x000F); // index unchanged

        // Above upper bound → #BR
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_ax(0x0021);
        cpu.set_interrupt_flag(true);
        cpu.halted = false;
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0B00);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0);

        // Inclusive endpoints succeed
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_ax(0x0010);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 4);
        cpu.rip = 0;
        cpu.set_ax(0x0020);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 4);
    }

    /// BOUND register form is #UD via IVT (SDM Vol. 2 BOUND; Vol. 3 §6.15).
    #[test]
    fn bound_register_source_ud_via_ivt() {
        let mut mem = vec![0u8; 0x10000];
        // IVT[6] → 0000:0C00
        mem[24] = 0x00;
        mem[25] = 0x0C;
        mem[26] = 0x00;
        mem[27] = 0x00;
        mem[0] = 0x62;
        mem[1] = 0xC0; // BOUND AX, AX — mod=11 → #UD
        mem[0xC00] = 0xF4;
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0C00);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0); // fault IP
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
    fn grp2_reserved_slash6_ud_via_ivt() {
        let mut mem = vec![0u8; 0x10000];
        // IVT[6] → 0000:0B00
        mem[24] = 0x00;
        mem[25] = 0x0B;
        mem[26] = 0x00;
        mem[27] = 0x00;
        mem[0] = 0xD0;
        mem[1] = 0xF0; // /6 AL
        mem[0xB00] = 0xF4;
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0B00);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0);
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

    /// BCD adjust: DAA/DAS/AAA/AAS/AAM/AAD results + flags (Intel SDM Vol. 2).
    #[test]
    fn bcd_adjust_daa_das_aaa_aas_aam_aad_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 0: DAA  1: DAS  2: AAA  3: AAS  4-5: AAM 0Ah  6-7: AAD 0Ah  8-9: AAM 10h  10-11: AAD 10h
        mem[0] = 0x27;
        mem[1] = 0x2F;
        mem[2] = 0x37;
        mem[3] = 0x3F;
        mem[4] = 0xD4;
        mem[5] = 0x0A;
        mem[6] = 0xD5;
        mem[7] = 0x0A;
        mem[8] = 0xD4;
        mem[9] = 0x10;
        mem[10] = 0xD5;
        mem[11] = 0x10;
        mem[12] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };

        // DAA: AL=0x0A, AF=0, CF=0 → AL=0x10, AF=1, CF=0; SF/ZF/PF from AL.
        // Spec: Intel SDM Vol. 2 "DAA".
        cpu.set_al(0x0A);
        cpu.set_af(false);
        cpu.set_cf(false);
        cpu.set_of(true); // OF undefined — left unchanged
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x10);
        assert!(cpu.rflags & (1 << 4) != 0); // AF
        assert!(cpu.rflags & 1 == 0); // CF
        assert!(cpu.rflags & (1 << 6) == 0); // ZF
        assert!(cpu.rflags & (1 << 7) == 0); // SF
                                             // PF: 0x10 has one set bit (odd) → PF clear
        assert!(cpu.rflags & (1 << 2) == 0);
        assert!(cpu.rflags & (1 << 11) != 0); // OF preserved

        // DAA: AL=0x9A → low adjust then +60H → AL=0x00, AF=1, CF=1, ZF=1.
        cpu.rip = 0;
        cpu.set_al(0x9A);
        cpu.set_af(false);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x00);
        assert!(cpu.rflags & (1 << 4) != 0);
        assert!(cpu.rflags & 1 != 0);
        assert!(cpu.rflags & (1 << 6) != 0);

        // DAA: AL=0x15, AF=1 → +6 → 0x1B; no high adjust.
        cpu.rip = 0;
        cpu.set_al(0x15);
        cpu.set_af(true);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x1B);
        assert!(cpu.rflags & (1 << 4) != 0);
        assert!(cpu.rflags & 1 == 0);

        // DAS: AL=0x10, AF=0, CF=0 → no adjust (nibble ok, high ok).
        // Spec: Intel SDM Vol. 2 "DAS".
        cpu.rip = 1;
        cpu.set_al(0x10);
        cpu.set_af(false);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x10);
        assert!(cpu.rflags & (1 << 4) == 0);
        assert!(cpu.rflags & 1 == 0);

        // DAS: AL=0x05, AF=1 → AL−6 = 0xFF, AF=1; high adjust off → CF=0.
        cpu.rip = 1;
        cpu.set_al(0x05);
        cpu.set_af(true);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xFF);
        assert!(cpu.rflags & (1 << 4) != 0);
        assert!(cpu.rflags & 1 == 0);
        assert!(cpu.rflags & (1 << 7) != 0); // SF

        // DAS: AL=0xA0 → high adjust −60H → AL=0x40, CF=1.
        cpu.rip = 1;
        cpu.set_al(0xA0);
        cpu.set_af(false);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x40);
        assert!(cpu.rflags & 1 != 0);

        // AAA: AL=0x0A → AX+=0x106, AL&=0x0F → AX=0x0100, AF=CF=1.
        // Spec: Intel SDM Vol. 2 "AAA". OF/SF/ZF/PF undefined (left unchanged).
        cpu.rip = 2;
        cpu.set_ax(0x000A);
        cpu.set_af(false);
        cpu.set_cf(false);
        cpu.set_zf(true);
        cpu.set_sf(true);
        cpu.set_of(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0100);
        assert!(cpu.rflags & (1 << 4) != 0);
        assert!(cpu.rflags & 1 != 0);
        assert!(cpu.rflags & (1 << 6) != 0); // ZF preserved
        assert!(cpu.rflags & (1 << 7) != 0); // SF preserved
        assert!(cpu.rflags & (1 << 11) != 0); // OF preserved

        // AAA: AL=0x05, AF=0 → no adjust; AL&=0x0F stays 5; AF=CF=0.
        cpu.rip = 2;
        cpu.set_ax(0x1205);
        cpu.set_af(false);
        cpu.set_cf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x1205);
        assert!(cpu.rflags & (1 << 4) == 0);
        assert!(cpu.rflags & 1 == 0);

        // AAS: AL=0x0A → AX−=0x106, AL&=0x0F → AX=0xFF04? Wait: 0x000A - 0x106 = 0xFF04, then AL&=0x0F → 0xFF04.
        // Spec: Intel SDM Vol. 2 "AAS".
        cpu.rip = 3;
        cpu.set_ax(0x000A);
        cpu.set_af(false);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0xFF04);
        assert!(cpu.rflags & (1 << 4) != 0);
        assert!(cpu.rflags & 1 != 0);

        // AAS: AL=0x03, AF=0 → AL&=0x0F; AF=CF=0.
        cpu.rip = 3;
        cpu.set_ax(0x5503);
        cpu.set_af(false);
        cpu.set_cf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x5503);
        assert!(cpu.rflags & (1 << 4) == 0);
        assert!(cpu.rflags & 1 == 0);

        // AAM base 10: AL=0x0F → AH=1, AL=5; SF/ZF/PF from AL.
        // Spec: Intel SDM Vol. 2 "AAM".
        cpu.rip = 4;
        cpu.set_ax(0x000F);
        cpu.set_cf(true); // undefined — left unchanged
        cpu.set_of(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0105);
        assert!(cpu.rflags & (1 << 6) == 0); // ZF
        assert!(cpu.rflags & (1 << 7) == 0); // SF
        assert!(cpu.rflags & (1 << 2) != 0); // PF even(5)=true? 5=101b two bits → even → PF=1
        assert!(cpu.rflags & 1 != 0); // CF preserved
        assert!(cpu.rflags & (1 << 11) != 0); // OF preserved

        // AAD base 10: AH=2, AL=3 → AL=23=0x17, AH=0.
        // Spec: Intel SDM Vol. 2 "AAD".
        cpu.rip = 6;
        cpu.set_ax(0x0203);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0017);
        assert!(cpu.rflags & (1 << 6) == 0);
        assert!(cpu.rflags & (1 << 7) == 0);

        // AAM base 16: AL=0x2A → AH=2, AL=0x0A.
        cpu.rip = 8;
        cpu.set_ax(0x002A);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x020A);

        // AAD base 16: AH=1, AL=5 → AL = 5 + 16 = 0x15.
        cpu.rip = 10;
        cpu.set_ax(0x0105);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0015);
        // PF for 0x15: three set bits → odd → PF clear
        assert!(cpu.rflags & (1 << 2) == 0);
    }

    /// AAM imm8=0 raises #DE via IVT vector 0 (SDM Vol. 2 AAM; Vol. 3 §6.15).
    #[test]
    fn aam_base_zero_de_via_ivt() {
        let mut mem = vec![0u8; 0x10000];
        // IVT[0] → 0000:0900
        mem[0] = 0x00;
        mem[1] = 0x09;
        mem[2] = 0x00;
        mem[3] = 0x00;
        mem[0x1000] = 0xD4;
        mem[0x1001] = 0x00; // AAM 0
        mem[0x900] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0x0100);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        cpu.set_ax(0x0010);
        let ax_before = cpu.ax();
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.cs.selector, 0);
        assert_eq!(cpu.ip16(), 0x0900);
        assert!(!cpu.interrupt_flag());
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0); // fault IP
        assert_eq!(bus.read_u16(0xFFFA).unwrap(), 0x0100);
        assert_eq!(cpu.ax(), ax_before); // no partial update
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
    fn lea_register_source_ud_via_ivt() {
        let mut mem = vec![0u8; 0x10000];
        // IVT[6] → 0000:0B00
        mem[24] = 0x00;
        mem[25] = 0x0B;
        mem[26] = 0x00;
        mem[27] = 0x00;
        mem[0] = 0x8D;
        mem[1] = 0xC0; // LEA AX, AX — mod=11 → #UD
        mem[0xB00] = 0xF4;
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0B00);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0);
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

    /// Decode-miss #UD policy (sparse primary table).
    ///
    /// - Architecturally invalid in real-address mode (e.g. ARPL 0x63) → IVT vector 6.
    /// - Valid-but-unimplemented (x87, WAIT, IN/OUT EAX, unimplemented 0F map, …) stay host Decode errors.
    /// - D6/F1 are reserved/undefined but do **not** generate #UD (SDM Vol. 3 §6.15).
    ///
    /// Spec: Intel SDM Vol. 3 §6.15 (#UD); Vol. 2 ARPL (real-address mode).
    #[test]
    fn decode_miss_ud_via_ivt_only_for_architectural_ud() {
        let mut mem = vec![0u8; 0x10000];
        // IVT[6] → 0000:0B00
        mem[6 * 4] = 0x00;
        mem[6 * 4 + 1] = 0x0B;
        mem[6 * 4 + 2] = 0x00;
        mem[6 * 4 + 3] = 0x00;
        mem[0x1000] = 0x63; // ARPL — #UD in real-address mode
        mem[0x1001] = 0xC0;
        mem[0xB00] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0x0100);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.cs.selector, 0);
        assert_eq!(cpu.ip16(), 0x0B00);
        assert!(!cpu.interrupt_flag());
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0); // fault IP = insn start
        assert_eq!(bus.read_u16(0xFFFA).unwrap(), 0x0100);

        // Sparse-table misses that are valid-but-unimplemented must NOT become #UD.
        for &op in &[0x9Bu8, 0xD8, 0xED, 0xD6, 0xF1] {
            let mut mem = vec![0u8; 0x10000];
            mem[6 * 4] = 0x00;
            mem[6 * 4 + 1] = 0x0B;
            mem[6 * 4 + 2] = 0x00;
            mem[6 * 4 + 3] = 0x00;
            mem[0] = op;
            mem[1] = 0x90;
            let mut cpu = CpuState::reset();
            cpu.cs = x86_core::SegmentReg::real_mode_code(0);
            cpu.ss = x86_core::SegmentReg::real_mode(0);
            cpu.rip = 0;
            cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
            let mut bus = VecBus { mem, ports: vec![] };
            let err = step(&mut cpu, &mut bus).unwrap_err();
            assert!(
                matches!(err, ExecError::Decode(DecodeError::UnsupportedOpcode(o)) if o == op),
                "opcode {op:#x} should remain Decode/UnsupportedOpcode, got {err:?}"
            );
            assert_eq!(cpu.ip16(), 0, "IP must not advance on host decode miss");
            assert_eq!(cpu.cs.selector, 0);
        }

        // 0F is a real escape (IMUL 0F AF is implemented); unimplemented secondaries
        // report UnsupportedOpcode(secondary) and must not vector #UD.
        {
            let mut mem = vec![0u8; 0x10000];
            mem[6 * 4] = 0x00;
            mem[6 * 4 + 1] = 0x0B;
            mem[6 * 4 + 2] = 0x00;
            mem[6 * 4 + 3] = 0x00;
            mem[0] = 0x0F;
            mem[1] = 0x90; // not in 0F map
            let mut cpu = CpuState::reset();
            cpu.cs = x86_core::SegmentReg::real_mode_code(0);
            cpu.ss = x86_core::SegmentReg::real_mode(0);
            cpu.rip = 0;
            cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
            let mut bus = VecBus { mem, ports: vec![] };
            let err = step(&mut cpu, &mut bus).unwrap_err();
            assert!(
                matches!(err, ExecError::Decode(DecodeError::UnsupportedOpcode(0x90))),
                "unimplemented 0F map entry should remain Decode/UnsupportedOpcode(secondary), got {err:?}"
            );
            assert_eq!(cpu.ip16(), 0, "IP must not advance on host decode miss");
            assert_eq!(cpu.cs.selector, 0);
        }
    }

    /// Bus that returns MemoryFault once for a poisoned linear address, then allows it.
    /// Needed when the faulting stack write address overlaps the later IVT frame pushes.
    struct PoisonBus {
        mem: Vec<u8>,
        poison: u64,
        tripped: bool,
    }

    impl Bus for PoisonBus {
        fn read_u8(&mut self, addr: u64) -> Result<u8, ExecError> {
            if addr == self.poison && !self.tripped {
                self.tripped = true;
                return Err(ExecError::MemoryFault(addr));
            }
            let i = addr as usize;
            if i >= self.mem.len() {
                return Err(ExecError::MemoryFault(addr));
            }
            Ok(self.mem[i])
        }
        fn write_u8(&mut self, addr: u64, val: u8) -> Result<(), ExecError> {
            if addr == self.poison && !self.tripped {
                self.tripped = true;
                return Err(ExecError::MemoryFault(addr));
            }
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
        fn port_out_u8(&mut self, _port: u16, _val: u8) -> Result<(), ExecError> {
            Ok(())
        }
    }

    /// Real-mode MemoryFault → #SS (vector 12) when the access uses SS; #GP (13) otherwise.
    /// Spec: Intel SDM Vol. 3 §6.4, §6.15 (#SS/#GP).
    /// Remaining host MemoryFault: IVT delivery stack/IVT bus errors (unchecked pushes).
    #[test]
    fn memory_fault_ss_gp_via_ivt() {
        // --- #SS: PUSH AX writes SS:SP-2 at poisoned linear address ---
        {
            let mut mem = vec![0u8; 0x10000];
            mem[12 * 4] = 0x00;
            mem[12 * 4 + 1] = 0x0C;
            mem[12 * 4 + 2] = 0x00;
            mem[12 * 4 + 3] = 0x00;
            mem[0] = 0x50; // PUSH AX
            mem[0xC00] = 0xF4;
            let poison = 0xFFFC; // first byte of PUSH write at SP=0xFFFE → SP-2=0xFFFC
            let mut cpu = CpuState::reset();
            cpu.cs = x86_core::SegmentReg::real_mode_code(0);
            cpu.ss = x86_core::SegmentReg::real_mode(0);
            cpu.rip = 0;
            cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
            cpu.set_interrupt_flag(true);
            let mut bus = PoisonBus {
                mem,
                poison,
                tripped: false,
            };
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.cs.selector, 0);
            assert_eq!(cpu.ip16(), 0x0C00);
            assert!(!cpu.interrupt_flag());
            // After SP restore + 3× push16: SP = 0xFFFE - 6 = 0xFFF8; saved IP at 0xFFF8
            assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0);
            assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF8);
        }

        // --- #GP: MOV AX,[BX] DS-relative read at poisoned address ---
        {
            let mut mem = vec![0u8; 0x10000];
            mem[13 * 4] = 0x00;
            mem[13 * 4 + 1] = 0x0D;
            mem[13 * 4 + 2] = 0x00;
            mem[13 * 4 + 3] = 0x00;
            // 8B 07 = MOV AX, [BX]
            mem[0] = 0x8B;
            mem[1] = 0x07;
            mem[0xD00] = 0xF4;
            let poison = 0x3000;
            let mut cpu = CpuState::reset();
            cpu.cs = x86_core::SegmentReg::real_mode_code(0);
            cpu.ss = x86_core::SegmentReg::real_mode(0);
            cpu.ds = x86_core::SegmentReg::real_mode(0);
            cpu.rip = 0;
            cpu.set_gpr_u16(CpuState::RBX, 0x3000);
            cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
            cpu.set_interrupt_flag(true);
            let mut bus = PoisonBus {
                mem,
                poison,
                tripped: false,
            };
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.ip16(), 0x0D00);
            assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0); // fault IP
            assert!(!cpu.interrupt_flag());
        }

        // --- #SS: MOV AX,[BP] default segment is SS ---
        {
            let mut mem = vec![0u8; 0x10000];
            mem[12 * 4] = 0x00;
            mem[12 * 4 + 1] = 0x0C;
            mem[12 * 4 + 2] = 0x00;
            mem[12 * 4 + 3] = 0x00;
            mem[0] = 0x8B;
            mem[1] = 0x46;
            mem[2] = 0x00; // MOV AX, [BP+0]
            mem[0xC00] = 0xF4;
            let poison = 0x4000;
            let mut cpu = CpuState::reset();
            cpu.cs = x86_core::SegmentReg::real_mode_code(0);
            cpu.ss = x86_core::SegmentReg::real_mode(0);
            cpu.rip = 0;
            cpu.set_gpr_u16(CpuState::RBP, 0x4000);
            cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
            cpu.set_interrupt_flag(true);
            let mut bus = PoisonBus {
                mem,
                poison,
                tripped: false,
            };
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.ip16(), 0x0C00);
            assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0);
        }
    }

    /// String / moffs MemoryFault → #GP/#SS via IVT (same classify as ModRM).
    /// Spec: Intel SDM Vol. 3 §6.15 (#SS/#GP); Vol. 2 MOVS/STOS/LODS/MOV moffs.
    #[test]
    fn string_moffs_memory_fault_ss_gp_via_ivt() {
        // --- #GP: STOSB ES:DI write at poisoned address ---
        {
            let mut mem = vec![0u8; 0x10000];
            mem[13 * 4] = 0x00;
            mem[13 * 4 + 1] = 0x0D;
            mem[13 * 4 + 2] = 0x00;
            mem[13 * 4 + 3] = 0x00;
            mem[0] = 0xAA; // STOSB
            mem[0xD00] = 0xF4;
            let poison = 0x3000;
            let mut cpu = CpuState::reset();
            cpu.cs = x86_core::SegmentReg::real_mode_code(0);
            cpu.es = x86_core::SegmentReg::real_mode(0);
            cpu.ss = x86_core::SegmentReg::real_mode(0);
            cpu.rip = 0;
            cpu.set_al(0x5A);
            cpu.set_gpr_u16(CpuState::RDI, 0x3000);
            cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
            cpu.set_interrupt_flag(true);
            let mut bus = PoisonBus {
                mem,
                poison,
                tripped: false,
            };
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.ip16(), 0x0D00);
            assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0); // fault IP = STOSB
            assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x3000); // no index update on fault
            assert!(!cpu.interrupt_flag());
        }

        // --- #SS: LODSB with SS override, SI at poison ---
        {
            let mut mem = vec![0u8; 0x10000];
            mem[12 * 4] = 0x00;
            mem[12 * 4 + 1] = 0x0C;
            mem[12 * 4 + 2] = 0x00;
            mem[12 * 4 + 3] = 0x00;
            // 36 AC = SS: LODSB
            mem[0] = 0x36;
            mem[1] = 0xAC;
            mem[0xC00] = 0xF4;
            let poison = 0x4000;
            let mut cpu = CpuState::reset();
            cpu.cs = x86_core::SegmentReg::real_mode_code(0);
            cpu.ss = x86_core::SegmentReg::real_mode(0);
            cpu.rip = 0;
            cpu.set_gpr_u16(CpuState::RSI, 0x4000);
            cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
            cpu.set_interrupt_flag(true);
            let mut bus = PoisonBus {
                mem,
                poison,
                tripped: false,
            };
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.ip16(), 0x0C00);
            assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0);
            assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x4000);
        }

        // --- #GP: MOV AL, moffs8 (A0) DS-relative ---
        {
            let mut mem = vec![0u8; 0x10000];
            mem[13 * 4] = 0x00;
            mem[13 * 4 + 1] = 0x0D;
            mem[13 * 4 + 2] = 0x00;
            mem[13 * 4 + 3] = 0x00;
            // A0 00 50 = MOV AL, [0x5000]
            mem[0] = 0xA0;
            mem[1] = 0x00;
            mem[2] = 0x50;
            mem[0xD00] = 0xF4;
            let poison = 0x5000;
            let mut cpu = CpuState::reset();
            cpu.cs = x86_core::SegmentReg::real_mode_code(0);
            cpu.ds = x86_core::SegmentReg::real_mode(0);
            cpu.ss = x86_core::SegmentReg::real_mode(0);
            cpu.rip = 0;
            cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
            cpu.set_interrupt_flag(true);
            let mut bus = PoisonBus {
                mem,
                poison,
                tripped: false,
            };
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.ip16(), 0x0D00);
            assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0);
        }

        // --- #SS: MOV AL, moffs8 with SS override ---
        {
            let mut mem = vec![0u8; 0x10000];
            mem[12 * 4] = 0x00;
            mem[12 * 4 + 1] = 0x0C;
            mem[12 * 4 + 2] = 0x00;
            mem[12 * 4 + 3] = 0x00;
            // 36 A0 00 60 = SS: MOV AL, [0x6000]
            mem[0] = 0x36;
            mem[1] = 0xA0;
            mem[2] = 0x00;
            mem[3] = 0x60;
            mem[0xC00] = 0xF4;
            let poison = 0x6000;
            let mut cpu = CpuState::reset();
            cpu.cs = x86_core::SegmentReg::real_mode_code(0);
            cpu.ss = x86_core::SegmentReg::real_mode(0);
            cpu.rip = 0;
            cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
            cpu.set_interrupt_flag(true);
            let mut bus = PoisonBus {
                mem,
                poison,
                tripped: false,
            };
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.ip16(), 0x0C00);
            assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0);
        }
    }

    /// #UD (vector 6) via real-mode IVT for reserved / invalid encodings.
    /// Spec: Intel SDM Vol. 3 §6.15 (#UD); Vol. 2 opcode map (Group 2 /6, Group 5 /7, …).
    /// Faulting IP = instruction start (same frame shape as software INT / #DE).
    #[test]
    fn ud_exception_via_ivt_reserved_encodings() {
        let mut mem = vec![0u8; 0x10000];
        // IVT[6] → handler at 0000:0A00
        mem[6 * 4] = 0x00;
        mem[6 * 4 + 1] = 0x0A;
        mem[6 * 4 + 2] = 0x00;
        mem[6 * 4 + 3] = 0x00;
        // CS base 0x1000 (selector 0x0100), IP 0:
        // 0: D0 F0         Group 2 /6 AL (reserved)
        // 2: FF F8         Group 5 /7 AX (reserved)
        // 4: 8D C0         LEA AX, AX (register source)
        // 6: 8E C8         MOV CS, AX
        // 8: C6 C8 00      MOV r/m8,imm /1 (Group 11 reserved)
        // B: FE D0         Group 4 /2 AL (reserved)
        // D: FF D8         Group 5 /3 CALL far reg (#UD)
        // F: 8F C0         POP r/m /0 would be valid; 8F C8 = /1 AX (#UD)
        mem[0x1000] = 0xD0;
        mem[0x1001] = 0xF0;
        mem[0x1002] = 0xFF;
        mem[0x1003] = 0xF8;
        mem[0x1004] = 0x8D;
        mem[0x1005] = 0xC0;
        mem[0x1006] = 0x8E;
        mem[0x1007] = 0xC8;
        mem[0x1008] = 0xC6;
        mem[0x1009] = 0xC8;
        mem[0x100A] = 0x00;
        mem[0x100B] = 0xFE;
        mem[0x100C] = 0xD0;
        mem[0x100D] = 0xFF;
        mem[0x100E] = 0xD8;
        mem[0x100F] = 0x8F;
        mem[0x1010] = 0xC8; // POP /1 AX
        mem[0xA00] = 0xF4; // handler HLT

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0x0100);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        let mut bus = VecBus { mem, ports: vec![] };

        let cases: &[(u16, u16)] = &[
            (0, 0),     // Group 2 /6
            (2, 2),     // Group 5 /7
            (4, 4),     // LEA reg
            (6, 6),     // MOV CS
            (8, 8),     // C6 /1
            (0xB, 0xB), // FE /2
            (0xD, 0xD), // FF /3 far CALL reg
            (0xF, 0xF), // 8F /1
        ];
        for &(ip, expect_saved_ip) in cases {
            cpu.cs = x86_core::SegmentReg::real_mode_code(0x0100);
            cpu.rip = u64::from(ip);
            cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
            cpu.set_interrupt_flag(true);
            cpu.halted = false;
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.cs.selector, 0, "handler CS at IP {ip:#x}");
            assert_eq!(cpu.ip16(), 0x0A00, "handler IP at fault IP {ip:#x}");
            assert!(!cpu.interrupt_flag());
            assert_eq!(
                bus.read_u16(0xFFF8).unwrap(),
                expect_saved_ip,
                "saved fault IP"
            );
            assert_eq!(bus.read_u16(0xFFFA).unwrap(), 0x0100); // CS
        }
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
        // FE /2 #UD covered by ud_exception_via_ivt_reserved_encodings
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
        mem[26] = 0xF4;
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
        // FF /3 far CALL reg #UD covered by ud_exception_via_ivt_reserved_encodings
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
    }

    /// FF Group 5 far CALL/JMP m16:16 — /3 CALL far, /5 JMP far (SDM Vol. 2).
    #[test]
    fn grp5_call_jmp_far_real_mode() {
        let mut mem = vec![0u8; 0x20000];
        // 0: FF 1E 00 40    CALL FAR [0x4000]
        // 4: F4             HLT (return landing after RETF)
        // 5: FF 2E 00 40    JMP FAR [0x4000]
        // 9: F4             HLT (should not reach after JMP)
        // Far CALL/JMP register #UD covered by ud_exception_via_ivt_reserved_encodings
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
        // C6 /1 #UD covered by ud_exception_via_ivt_reserved_encodings
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
        mem[18] = 0xF4;

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

    /// ENTER nesting level 1 pushes old BP and the new frame pointer (display).
    /// Spec: Intel SDM Vol. 2 "ENTER"; Vol. 1 §6.5.
    #[test]
    fn enter_nesting_level1_display() {
        let mut mem = vec![0u8; 0x10000];
        // ENTER 4, 1
        mem[0] = 0xC8;
        mem[1] = 0x04;
        mem[2] = 0x00;
        mem[3] = 0x01;
        mem[4] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let sp0 = 0xFFFE_u16;
        cpu.set_gpr_u16(CpuState::RSP, sp0);
        cpu.set_gpr_u16(CpuState::RBP, 0xABCD);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        // Push old BP; frame_temp = SP; Push(frame_temp); BP = frame_temp; SP -= 4.
        assert_eq!(bus.read_u16(u64::from(sp0 - 2)).unwrap(), 0xABCD);
        let frame = sp0 - 2;
        assert_eq!(bus.read_u16(u64::from(sp0 - 4)).unwrap(), frame);
        assert_eq!(cpu.gpr_u16(CpuState::RBP), frame);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), frame.wrapping_sub(2 + 4));
    }

    /// ENTER nesting level 2 copies one display word from the caller's frame.
    /// Spec: Intel SDM Vol. 2 "ENTER"; Vol. 1 §6.5.
    #[test]
    fn enter_nesting_level2_copies_display() {
        let mut mem = vec![0u8; 0x10000];
        // ENTER 0, 1 then ENTER 0, 2
        mem[0] = 0xC8;
        mem[1] = 0x00;
        mem[2] = 0x00;
        mem[3] = 0x01;
        mem[4] = 0xC8;
        mem[5] = 0x00;
        mem[6] = 0x00;
        mem[7] = 0x02;
        mem[8] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let sp0 = 0xFFFE_u16;
        cpu.set_gpr_u16(CpuState::RSP, sp0);
        cpu.set_gpr_u16(CpuState::RBP, 0x1111);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap(); // ENTER 0,1
        let parent_bp = cpu.gpr_u16(CpuState::RBP);
        assert_eq!(parent_bp, sp0 - 2);
        // Parent display: [BP]=old BP, [BP-2]=frame_temp (= parent_bp).
        assert_eq!(bus.read_u16(u64::from(parent_bp)).unwrap(), 0x1111);
        assert_eq!(bus.read_u16(u64::from(parent_bp - 2)).unwrap(), parent_bp);

        step(&mut cpu, &mut bus).unwrap(); // ENTER 0,2
        let child_bp = cpu.gpr_u16(CpuState::RBP);
        // [BP] = parent frame pointer (pushed at start).
        assert_eq!(bus.read_u16(u64::from(child_bp)).unwrap(), parent_bp);
        // [BP-2] = copied display entry from parent [parent_bp-2] (= parent_bp).
        assert_eq!(bus.read_u16(u64::from(child_bp - 2)).unwrap(), parent_bp);
        // [BP-4] = child's frame_temp (= child_bp).
        assert_eq!(bus.read_u16(u64::from(child_bp - 4)).unwrap(), child_bp);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), child_bp - 4);
    }

    /// ENTERD (0x66 ENTER) nesting 0 + LEAVE opsize-32 round-trip.
    /// Spec: Intel SDM Vol. 2 "ENTER"/"LEAVE"; Ch. 2 (66H); Vol. 1 §3.6 / §6.5.
    #[test]
    fn enterd_level0_leave_round_trip() {
        let mut mem = vec![0u8; 0x10000];
        // 66 C8 08 00 00 = ENTERD 8, 0
        mem[0] = 0x66;
        mem[1] = 0xC8;
        mem[2] = 0x08;
        mem[3] = 0x00;
        mem[4] = 0x00;
        // 66 C9 = LEAVE (opsize 32)
        mem[5] = 0x66;
        mem[6] = 0xC9;
        mem[7] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let sp0 = 0xFFFE_u16;
        cpu.set_gpr_u16(CpuState::RSP, sp0);
        cpu.set_gpr_u32(CpuState::RBP, 0xAAAA_ABCD);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        // Push EBP (4); frame = SP; EBP = frame; SP = frame - 8.
        assert_eq!(bus.read_u32(u64::from(sp0 - 4)).unwrap(), 0xAAAA_ABCD);
        let frame = u32::from(sp0 - 4);
        assert_eq!(cpu.gpr_u32(CpuState::RBP), frame);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), (frame as u16).wrapping_sub(8));

        step(&mut cpu, &mut bus).unwrap(); // LEAVE opsize32
        assert_eq!(cpu.gpr_u32(CpuState::RBP), 0xAAAA_ABCD);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), sp0);
    }

    /// ENTERD nesting level 1: push EBP, push frame_temp (dword display).
    /// Spec: Intel SDM Vol. 2 "ENTER"; Vol. 1 §6.5; Ch. 2 (66H).
    #[test]
    fn enterd_nesting_level1_display() {
        let mut mem = vec![0u8; 0x10000];
        // 66 C8 04 00 01 = ENTERD 4, 1
        mem[0] = 0x66;
        mem[1] = 0xC8;
        mem[2] = 0x04;
        mem[3] = 0x00;
        mem[4] = 0x01;
        mem[5] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let sp0 = 0xFFFE_u16;
        cpu.set_gpr_u16(CpuState::RSP, sp0);
        cpu.set_gpr_u32(CpuState::RBP, 0x1111_ABCD);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u32(u64::from(sp0 - 4)).unwrap(), 0x1111_ABCD);
        let frame = u32::from(sp0 - 4);
        assert_eq!(bus.read_u32(u64::from(sp0 - 8)).unwrap(), frame);
        assert_eq!(cpu.gpr_u32(CpuState::RBP), frame);
        // frame_temp push (4) + alloc 4.
        assert_eq!(
            cpu.gpr_u16(CpuState::RSP),
            (frame as u16).wrapping_sub(4 + 4)
        );
    }

    /// PUSHAD stack layout then POPAD restores GPRs (discards saved ESP).
    /// Spec: Intel SDM Vol. 2 "PUSHA/PUSHAD", "POPA/POPAD"; Ch. 2 (66H).
    #[test]
    fn pushad_popad_stack_layout_and_round_trip() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x66;
        mem[1] = 0x60; // PUSHAD
        mem[2] = 0x66;
        mem[3] = 0x61; // POPAD
        mem[4] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let sp0 = 0xFFFE_u16;
        cpu.set_gpr_u16(CpuState::RSP, sp0);
        cpu.set_gpr_u32(CpuState::RAX, 0x1111_1111);
        cpu.set_gpr_u32(CpuState::RCX, 0x2222_2222);
        cpu.set_gpr_u32(CpuState::RDX, 0x3333_3333);
        cpu.set_gpr_u32(CpuState::RBX, 0x4444_4444);
        cpu.set_gpr_u32(CpuState::RBP, 0x5555_5555);
        cpu.set_gpr_u32(CpuState::RSI, 0x6666_6666);
        cpu.set_gpr_u32(CpuState::RDI, 0x7777_7777);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RSP), sp0.wrapping_sub(32));
        assert_eq!(bus.read_u32(u64::from(sp0 - 4)).unwrap(), 0x1111_1111); // EAX
        assert_eq!(bus.read_u32(u64::from(sp0 - 8)).unwrap(), 0x2222_2222); // ECX
        assert_eq!(bus.read_u32(u64::from(sp0 - 12)).unwrap(), 0x3333_3333); // EDX
        assert_eq!(bus.read_u32(u64::from(sp0 - 16)).unwrap(), 0x4444_4444); // EBX
        assert_eq!(bus.read_u32(u64::from(sp0 - 20)).unwrap(), u32::from(sp0)); // orig ESP
        assert_eq!(bus.read_u32(u64::from(sp0 - 24)).unwrap(), 0x5555_5555); // EBP
        assert_eq!(bus.read_u32(u64::from(sp0 - 28)).unwrap(), 0x6666_6666); // ESI
        assert_eq!(bus.read_u32(u64::from(sp0 - 32)).unwrap(), 0x7777_7777); // EDI

        cpu.set_gpr_u32(CpuState::RAX, 0);
        cpu.set_gpr_u32(CpuState::RCX, 0);
        cpu.set_gpr_u32(CpuState::RDX, 0);
        cpu.set_gpr_u32(CpuState::RBX, 0);
        cpu.set_gpr_u32(CpuState::RBP, 0);
        cpu.set_gpr_u32(CpuState::RSI, 0);
        cpu.set_gpr_u32(CpuState::RDI, 0);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RAX), 0x1111_1111);
        assert_eq!(cpu.gpr_u32(CpuState::RCX), 0x2222_2222);
        assert_eq!(cpu.gpr_u32(CpuState::RDX), 0x3333_3333);
        assert_eq!(cpu.gpr_u32(CpuState::RBX), 0x4444_4444);
        assert_eq!(cpu.gpr_u32(CpuState::RBP), 0x5555_5555);
        assert_eq!(cpu.gpr_u32(CpuState::RSI), 0x6666_6666);
        assert_eq!(cpu.gpr_u32(CpuState::RDI), 0x7777_7777);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), sp0);
    }

    /// PUSHFD/POPFD round-trip in real-address mode (opsize 32).
    /// Spec: Intel SDM Vol. 2 "PUSHF/PUSHFD/PUSHFQ", "POPF/POPFD/POPFQ"; Ch. 2 (66H).
    /// VM and RF are unaffected by POPFD in real-address mode.
    #[test]
    fn pushfd_popfd_round_trip_preserves_vm_rf() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x66;
        mem[1] = 0x9C; // PUSHFD
        mem[2] = 0x66;
        mem[3] = 0x9D; // POPFD
        mem[4] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let sp0 = 0xFFFE_u16;
        cpu.set_gpr_u16(CpuState::RSP, sp0);
        // CF+PF+AF+ZF+SF+IF+OF + synthetic VM/RF that must survive POPFD.
        cpu.rflags = 0x0002_0AD7 | (1 << 16) | (1 << 17);
        let flags_before = cpu.rflags;
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap(); // PUSHFD
        assert_eq!(cpu.gpr_u16(CpuState::RSP), sp0.wrapping_sub(4));
        assert_eq!(
            bus.read_u32(u64::from(sp0 - 4)).unwrap(),
            (flags_before as u32)
        );

        // Clobber writable flags but keep VM/RF set for the POPFD preserve check.
        cpu.rflags = (1 << 16) | (1 << 17) | 2;
        step(&mut cpu, &mut bus).unwrap(); // POPFD
        assert_eq!(cpu.gpr_u16(CpuState::RSP), sp0);
        // Lower image restored (bit 1 forced); VM/RF unchanged from pre-POPFD.
        assert_eq!(cpu.rflags & 0xFFFF, u64::from(flags_before as u16 | 2));
        assert_ne!(cpu.rflags & (1 << 16), 0); // RF
        assert_ne!(cpu.rflags & (1 << 17), 0); // VM
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

    /// POP r/m16 (8F /0) reg and mem forms.
    /// Spec: Intel SDM Vol. 2 "POP". /1–/7 #UD covered by ud_exception_via_ivt_reserved_encodings.
    #[test]
    fn pop_rm16_reg_mem() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x8F;
        mem[1] = 0xC3; // POP BX
        mem[2] = 0x8F;
        mem[3] = 0x06;
        mem[4] = 0x00;
        mem[5] = 0x40; // POP [0x4000]
        mem[6] = 0xF4;

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
    }

    /// LES/LDS load m16:16 into r16 + ES/DS (SDM Vol. 2 LES/LDS). Real mode only.
    #[test]
    fn les_lds_load_far_pointer() {
        let mut mem = vec![0u8; 0x10000];
        // Far pointer at DS:0x2000 — offset 0x5678, segment 0x1234
        mem[0x2000] = 0x78;
        mem[0x2001] = 0x56;
        mem[0x2002] = 0x34;
        mem[0x2003] = 0x12;
        // Far pointer at DS:0x3000 — offset 0xABCD, segment 0xF000
        mem[0x3000] = 0xCD;
        mem[0x3001] = 0xAB;
        mem[0x3002] = 0x00;
        mem[0x3003] = 0xF0;
        // C4 06 00 20 = LES AX, [0x2000]
        // C5 1E 00 30 = LDS BX, [0x3000]
        mem[0] = 0xC4;
        mem[1] = 0x06;
        mem[2] = 0x00;
        mem[3] = 0x20;
        mem[4] = 0xC5;
        mem[5] = 0x1E;
        mem[6] = 0x00;
        mem[7] = 0x30;
        mem[8] = 0xF4;
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0x9999);
        cpu.rip = 0;
        cpu.rflags = 0x246; // IF+reserved; sticky pattern for "flags unchanged"
        let flags_before = cpu.rflags;
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x5678);
        assert_eq!(cpu.es.selector, 0x1234);
        assert_eq!(cpu.es.base, 0x1234u64 << 4);
        assert_eq!(cpu.rflags, flags_before);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0xABCD);
        assert_eq!(cpu.ds.selector, 0xF000);
        assert_eq!(cpu.ds.base, 0xF000u64 << 4);
        assert_eq!(cpu.rflags, flags_before);
    }

    /// LES/LDS register form is #UD via IVT (SDM Vol. 2 LES/LDS; Vol. 3 §6.15).
    #[test]
    fn les_lds_register_source_ud_via_ivt() {
        let mut mem = vec![0u8; 0x10000];
        // IVT[6] → 0000:0B00
        mem[24] = 0x00;
        mem[25] = 0x0B;
        mem[26] = 0x00;
        mem[27] = 0x00;
        mem[0] = 0xC4;
        mem[1] = 0xC0; // LES AX, AX — mod=11 → #UD
        mem[2] = 0xC5;
        mem[3] = 0xDB; // LDS BX, BX — mod=11 → #UD
        mem[0xB00] = 0xF4;
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0B00);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0);
        // Second case after returning to next insn via fresh RIP setup
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 2;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        cpu.halted = false;
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0B00);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 2);
    }

    /// XLATB: AL ← DS:[BX+AL] (SDM Vol. 2 XLAT/XLATB); segment override honored.
    #[test]
    fn xlat_table_lookup_and_segment_override() {
        let mut mem = vec![0u8; 0x20000];
        // DS=0 table at BX=0x1000: index AL=0x05 → 0xAB
        mem[0x1005] = 0xAB;
        // ES=0x1000 table at BX=0x0200: index AL=0x03 → linear 0x10203
        mem[0x10203] = 0xCD;
        // D7; 26 D7; F4
        mem[0] = 0xD7;
        mem[1] = 0x26;
        mem[2] = 0xD7;
        mem[3] = 0xF4;
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0x1000);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RBX, 0x1000);
        cpu.set_al(0x05);
        cpu.rflags = 0x246;
        let flags_before = cpu.rflags;
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xAB);
        assert_eq!(cpu.rflags, flags_before);
        cpu.set_gpr_u16(CpuState::RBX, 0x0200);
        cpu.set_al(0x03);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xCD);
        assert_eq!(cpu.rflags, flags_before);
    }

    /// IMUL r16, r/m16, imm — opcodes 69/6B (SDM Vol. 2 "IMUL").
    /// CF=OF set iff signed product does not fit in r16; SF/ZF/AF/PF undefined.
    #[test]
    fn imul_imm_69_6b_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 0: 69 D8 02 00     IMUL BX, AX, 2
        // 4: 69 D8 00 01     IMUL BX, AX, 0x100
        // 8: 6B D8 FD        IMUL BX, AX, -3 (imm8)
        // B: 6B DB FF        IMUL BX, BX, -1 (two-op sugar)
        // E: 69 1E 00 40 03 00  IMUL BX, [0x4000], 3
        mem[0] = 0x69;
        mem[1] = 0xD8;
        mem[2] = 0x02;
        mem[3] = 0x00;
        mem[4] = 0x69;
        mem[5] = 0xD8;
        mem[6] = 0x00;
        mem[7] = 0x01;
        mem[8] = 0x6B;
        mem[9] = 0xD8;
        mem[10] = 0xFD;
        mem[11] = 0x6B;
        mem[12] = 0xDB;
        mem[13] = 0xFF;
        mem[14] = 0x69;
        mem[15] = 0x1E;
        mem[16] = 0x00;
        mem[17] = 0x40;
        mem[18] = 0x03;
        mem[19] = 0x00;
        mem[20] = 0xF4;
        mem[0x4000] = 0x05;
        mem[0x4001] = 0x00;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };

        // IMUL BX, AX, 2: 3*2=6 fits → CF=OF=0; AX unchanged
        cpu.set_ax(3);
        cpu.set_gpr_u16(CpuState::RBX, 0xDEAD);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 6);
        assert_eq!(cpu.ax(), 3);
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 11), 0);

        // IMUL BX, AX, 0x100: 0x100*0x100=0x10000 does not fit in i16 → CF=OF=1
        cpu.set_ax(0x0100);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0);
        assert_ne!(cpu.rflags & 1, 0);
        assert_ne!(cpu.rflags & (1 << 11), 0);

        // IMUL BX, AX, -3: (-2)*(-3)=6 fits
        cpu.set_ax(0xFFFE); // -2
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 6);
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 11), 0);

        // IMUL BX, BX, -1: 6*(-1)=-6 fits
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0xFFFA); // -6
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 11), 0);

        // IMUL BX, [0x4000], 3: 5*3=15; memory unchanged
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 15);
        assert_eq!(bus.read_u16(0x4000).unwrap(), 5);
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 11), 0);
    }

    /// IMUL r16/r32, r/m16/r/m32 — opcode 0F AF (SDM Vol. 2 "IMUL").
    /// Dest = ModRM.reg * r/m; CF=OF iff signed product does not fit in dest width.
    #[test]
    fn imul_0f_af_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 0: 0F AF D8          IMUL BX, AX
        // 3: 0F AF D8          IMUL BX, AX (overflow)
        // 6: 0F AF 1E 00 40    IMUL BX, [0x4000]
        // B: 66 0F AF C3       IMUL EAX, EBX
        // F: 66 0F AF C3       IMUL EAX, EBX (overflow)
        // 13: F4               HLT
        mem[0] = 0x0F;
        mem[1] = 0xAF;
        mem[2] = 0xD8;
        mem[3] = 0x0F;
        mem[4] = 0xAF;
        mem[5] = 0xD8;
        mem[6] = 0x0F;
        mem[7] = 0xAF;
        mem[8] = 0x1E;
        mem[9] = 0x00;
        mem[10] = 0x40;
        mem[11] = 0x66;
        mem[12] = 0x0F;
        mem[13] = 0xAF;
        mem[14] = 0xC3;
        mem[15] = 0x66;
        mem[16] = 0x0F;
        mem[17] = 0xAF;
        mem[18] = 0xC3;
        mem[19] = 0xF4;
        mem[0x4000] = 0x05;
        mem[0x4001] = 0x00;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };

        // IMUL BX, AX: 3*2=6 fits → CF=OF=0; AX unchanged
        cpu.set_ax(2);
        cpu.set_gpr_u16(CpuState::RBX, 3);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 6);
        assert_eq!(cpu.ax(), 2);
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 11), 0);

        // IMUL BX, AX: 0x100*0x100=0x10000 does not fit in i16 → CF=OF=1
        cpu.set_ax(0x0100);
        cpu.set_gpr_u16(CpuState::RBX, 0x0100);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0);
        assert_ne!(cpu.rflags & 1, 0);
        assert_ne!(cpu.rflags & (1 << 11), 0);

        // IMUL BX, [0x4000]: 7*5=35; memory unchanged
        cpu.set_gpr_u16(CpuState::RBX, 7);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 35);
        assert_eq!(bus.read_u16(0x4000).unwrap(), 5);
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 11), 0);

        // IMUL EAX, EBX: 0x10 * 0x20 = 0x200 fits → CF=OF=0
        cpu.set_gpr_u32(CpuState::RAX, 0x10);
        cpu.set_gpr_u32(CpuState::RBX, 0x20);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RAX), 0x200);
        assert_eq!(cpu.gpr_u32(CpuState::RBX), 0x20);
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 11), 0);

        // IMUL EAX, EBX: 0x10000 * 0x10000 = 0x1_0000_0000 does not fit in i32
        cpu.set_gpr_u32(CpuState::RAX, 0x0001_0000);
        cpu.set_gpr_u32(CpuState::RBX, 0x0001_0000);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RAX), 0);
        assert_ne!(cpu.rflags & 1, 0);
        assert_ne!(cpu.rflags & (1 << 11), 0);
    }

    /// SMSW/LMSW — opcode 0F 01 /4 and /6 (SDM Vol. 2 SMSW/LMSW; Vol. 3 CR0).
    /// SMSW stores CR0[15:0]; LMSW loads CR0[15:0] and cannot clear PE.
    /// PE bit updates do not enter protected-mode execution here.
    #[test]
    fn smsw_lmsw_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        let code = 0x1000usize;
        // +0: 0F 01 E0         SMSW AX          (mod=11, /4, rm=AX)
        // +3: 0F 01 26 00 40   SMSW [0x4000]    (mem always 16-bit)
        // +8: 66 0F 01 E3      SMSW EBX         (opsize32 zero-extend)
        // +C: B8 01 00         MOV AX, 1        (PE=1)
        // +F: 0F 01 F0         LMSW AX
        // +12: B8 00 00        MOV AX, 0
        // +15: 0F 01 F0        LMSW AX          (must not clear PE)
        // +18: B8 10 00        MOV AX, 0x10     (ET)
        // +1B: 0F 01 F0        LMSW AX          (from CR0 with PE still set → PE stays)
        // +1E: F4              HLT
        mem[code] = 0x0F;
        mem[code + 1] = 0x01;
        mem[code + 2] = 0xE0; // 11_100_000 SMSW AX
        mem[code + 3] = 0x0F;
        mem[code + 4] = 0x01;
        mem[code + 5] = 0x26; // 00_100_110 SMSW [disp16]
        mem[code + 6] = 0x00;
        mem[code + 7] = 0x40;
        mem[code + 8] = 0x66;
        mem[code + 9] = 0x0F;
        mem[code + 10] = 0x01;
        mem[code + 11] = 0xE3; // SMSW EBX
        mem[code + 12] = 0xB8;
        mem[code + 13] = 0x01;
        mem[code + 14] = 0x00;
        mem[code + 15] = 0x0F;
        mem[code + 16] = 0x01;
        mem[code + 17] = 0xF0; // 11_110_000 LMSW AX
        mem[code + 18] = 0xB8;
        mem[code + 19] = 0x00;
        mem[code + 20] = 0x00;
        mem[code + 21] = 0x0F;
        mem[code + 22] = 0x01;
        mem[code + 23] = 0xF0;
        mem[code + 24] = 0xB8;
        mem[code + 25] = 0x10;
        mem[code + 26] = 0x00;
        mem[code + 27] = 0x0F;
        mem[code + 28] = 0x01;
        mem[code + 29] = 0xF0;
        mem[code + 30] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        // Reset CR0 low = 0x0010 (ET). Spec: typical real-mode after RESET.
        assert_eq!(cpu.cr0 as u16, 0x0010);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RAX), 0x0010);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x4000).unwrap(), 0x0010);

        cpu.set_gpr_u32(CpuState::RBX, 0xFFFF_FFFF);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RBX), 0x0000_0010);

        step(&mut cpu, &mut bus).unwrap(); // MOV AX,1
        step(&mut cpu, &mut bus).unwrap(); // LMSW AX → set PE
        assert_eq!(cpu.cr0 & 0xFFFF, 0x0001);
        assert_eq!(cpu.cr0 & 1, 1, "PE set in CR0");

        step(&mut cpu, &mut bus).unwrap(); // MOV AX,0
        step(&mut cpu, &mut bus).unwrap(); // LMSW AX — must not clear PE
        assert_eq!(cpu.cr0 & 1, 1, "LMSW cannot clear PE");
        assert_eq!(cpu.cr0 & 0xFFFF, 0x0001);

        step(&mut cpu, &mut bus).unwrap(); // MOV AX,0x10
        step(&mut cpu, &mut bus).unwrap(); // LMSW AX with PE sticky
        assert_eq!(cpu.cr0 & 0xFFFF, 0x0011, "ET loaded; PE remains set");
    }

    /// LMSW PE sticky keeps real-mode / sticky-unreal segment semantics.
    /// Spec: Intel SDM Vol. 2 "LMSW"; Vol. 3 §2.5 (CR0.PE); §3.4.2–§3.4.3
    /// (real-address `base = selector << 4` + unreal descriptor cache).
    /// Out of scope: GDT descriptor loads, far jump into protected mode, paging.
    #[test]
    fn lmsw_pe_sticky_keeps_real_mode_segment_and_far_jmp() {
        let mut mem = vec![0u8; 0x30000];
        let code = 0x1000usize;
        // +0: B8 01 00         MOV AX, 1
        // +3: 0F 01 F0         LMSW AX            (CR0.PE ← 1)
        // +6: B8 34 12         MOV AX, 0x1234
        // +9: 8E D8            MOV DS, AX         (still selector<<4 + sticky limit)
        // +B: EA 00 02 00 20   JMP 2000:0200      (far JMP still real-mode CS)
        // Target linear = 0x2000<<4 + 0x0200 = 0x20200 → HLT
        mem[code] = 0xB8;
        mem[code + 1] = 0x01;
        mem[code + 2] = 0x00;
        mem[code + 3] = 0x0F;
        mem[code + 4] = 0x01;
        mem[code + 5] = 0xF0; // LMSW AX
        mem[code + 6] = 0xB8;
        mem[code + 7] = 0x34;
        mem[code + 8] = 0x12;
        mem[code + 9] = 0x8E;
        mem[code + 10] = 0xD8; // MOV DS, AX
        mem[code + 11] = 0xEA;
        mem[code + 12] = 0x00;
        mem[code + 13] = 0x02;
        mem[code + 14] = 0x00;
        mem[code + 15] = 0x20; // JMP far 2000:0200
        mem[0x20200] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        // Expanded unreal cached limit must survive MOV DS after LMSW PE=1.
        cpu.ds.limit = 0xFFFF_FFFF;
        cpu.ds.flags = 0x0093;
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        assert_eq!(cpu.cr0 & 1, 0, "reset starts with PE clear");
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap(); // MOV AX, 1
        step(&mut cpu, &mut bus).unwrap(); // LMSW AX → set PE
        assert_eq!(cpu.cr0 & 1, 1, "LMSW sets CR0.PE");

        step(&mut cpu, &mut bus).unwrap(); // MOV AX, 0x1234
        step(&mut cpu, &mut bus).unwrap(); // MOV DS, AX
        assert_eq!(cpu.cr0 & 1, 1, "MOV DS must not clear PE");
        assert_eq!(cpu.ds.selector, 0x1234);
        assert_eq!(
            cpu.ds.base,
            0x1234u64 << 4,
            "after LMSW PE=1, DS base is still selector<<4 (no GDT load)"
        );
        assert_eq!(
            cpu.ds.limit, 0xFFFF_FFFF,
            "sticky unreal DS limit preserved under LMSW PE=1"
        );
        assert_eq!(cpu.ds.flags, 0x0093);

        step(&mut cpu, &mut bus).unwrap(); // JMP far
        assert_eq!(cpu.cr0 & 1, 1, "far JMP must not clear PE");
        assert_eq!(cpu.cs.selector, 0x2000);
        assert_eq!(
            cpu.cs.base,
            0x2000u64 << 4,
            "after LMSW PE=1, far JMP still uses real-mode CS base"
        );
        assert_eq!(cpu.ip16(), 0x0200);
        assert_eq!(
            cpu.gpr_u16(CpuState::RSP),
            0xFFFE,
            "far JMP does not touch stack"
        );
    }

    /// CLTS — opcode 0F 06. Clears CR0.TS (bit 3) only; all other CR0 bits preserved.
    /// Spec: Intel SDM Vol. 2 "CLTS—Clear Task-Switched Flag in CR0"; Vol. 3 §2.5 (CR0.TS).
    /// Real-mode only here; PM CPL/#GP checks are out of scope.
    #[test]
    fn clts_clears_only_cr0_ts() {
        let mut mem = vec![0u8; 0x10000];
        let code = 0x1000usize;
        // +0: 0F 06   CLTS
        // +2: F4      HLT
        mem[code] = 0x0F;
        mem[code + 1] = 0x06;
        mem[code + 2] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        // CD|NW|ET|TS|PE — TS (bit 3) and PE (bit 0) both set; CLTS must clear only TS.
        // Spec: CR0.TS = bit 3; CLTS clears TS without modifying other CR0 bits.
        cpu.cr0 = 0x6000_0019;
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap(); // CLTS
        assert_eq!(cpu.cr0 & (1 << 3), 0, "CR0.TS must be cleared");
        assert_eq!(cpu.cr0 & 1, 1, "PE must be preserved");
        assert_eq!(
            cpu.cr0, 0x6000_0011,
            "only TS (bit 3) cleared; CD|NW|ET|PE remain"
        );
        assert_eq!(cpu.rip, (code + 2) as u64);
    }

    /// MOV r32, CR0 — opcode 0F 20 /r (SDM Vol. 2 "MOV—Move to/from Control
    /// Registers"; Vol. 3 §2.5 CR0). Reads the full 32-bit CR0 into a GPR.
    #[test]
    fn mov_r32_cr0_reads_reset_value() {
        let mut mem = vec![0u8; 0x10000];
        let code = 0x1000usize;
        // +0: 0F 20 C0   MOV EAX, CR0
        // +3: F4         HLT
        mem[code] = 0x0F;
        mem[code + 1] = 0x20;
        mem[code + 2] = 0xC0; // 11_000_000: reg=0 (CR0), rm=0 (EAX)
        mem[code + 3] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        // Reset CR0 = CD|NW|ET (Spec: typical real-mode after RESET).
        assert_eq!(cpu.cr0, 0x6000_0010);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RAX), 0x6000_0010);
        assert_eq!(cpu.gpr_u32(CpuState::RAX) & 0xFFFF, 0x0010, "ET set");
    }

    /// MOV CR0, r32 — opcode 0F 22 /r. Unlike LMSW, this can clear PE.
    /// Setting/clearing PE must not switch the segment-load/execution model —
    /// segment loads keep using `selector << 4` real-mode bases.
    /// Spec: Intel SDM Vol. 2 "MOV—Move to/from Control Registers"; Vol. 3 §2.5.
    #[test]
    fn mov_cr0_r32_sets_and_clears_pe_no_mode_change() {
        let mut mem = vec![0u8; 0x10000];
        let code = 0x1000usize;
        // +0:  66 B8 11 00 00 60   MOV EAX, 0x60000011  (PE=1, plus CD|NW|ET)
        // +6:  0F 22 C0            MOV CR0, EAX
        // +9:  B8 34 12            MOV AX, 0x1234
        // +C:  8E D8               MOV DS, AX           (still real-mode base<<4)
        // +E:  66 B8 10 00 00 60   MOV EAX, 0x60000010  (PE=0)
        // +14: 0F 22 C0            MOV CR0, EAX         (MOV CR0 CAN clear PE)
        // +17: F4                  HLT
        mem[code] = 0x66;
        mem[code + 1] = 0xB8;
        mem[code + 2] = 0x11;
        mem[code + 3] = 0x00;
        mem[code + 4] = 0x00;
        mem[code + 5] = 0x60;
        mem[code + 6] = 0x0F;
        mem[code + 7] = 0x22;
        mem[code + 8] = 0xC0; // reg=0 (CR0), rm=0 (EAX)
        mem[code + 9] = 0xB8;
        mem[code + 10] = 0x34;
        mem[code + 11] = 0x12;
        mem[code + 12] = 0x8E;
        mem[code + 13] = 0xD8; // MOV DS, AX
        mem[code + 14] = 0x66;
        mem[code + 15] = 0xB8;
        mem[code + 16] = 0x10;
        mem[code + 17] = 0x00;
        mem[code + 18] = 0x00;
        mem[code + 19] = 0x60;
        mem[code + 20] = 0x0F;
        mem[code + 21] = 0x22;
        mem[code + 22] = 0xC0;
        mem[code + 23] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap(); // MOV EAX, 0x60000011
        assert_eq!(cpu.gpr_u32(CpuState::RAX), 0x6000_0011);

        step(&mut cpu, &mut bus).unwrap(); // MOV CR0, EAX
        assert_eq!(cpu.cr0, 0x6000_0011);
        assert_eq!(cpu.cr0 & 1, 1, "PE set via MOV CR0");

        step(&mut cpu, &mut bus).unwrap(); // MOV AX, 0x1234
        step(&mut cpu, &mut bus).unwrap(); // MOV DS, AX
        assert_eq!(
            cpu.ds.selector, 0x1234,
            "PE=1 does not change segment-load model"
        );
        assert_eq!(
            cpu.ds.base,
            0x1234u64 << 4,
            "DS base still selector<<4; no protected-mode descriptor lookup"
        );

        step(&mut cpu, &mut bus).unwrap(); // MOV EAX, 0x60000010
        step(&mut cpu, &mut bus).unwrap(); // MOV CR0, EAX — clears PE
        assert_eq!(cpu.cr0 & 1, 0, "MOV CR0 (unlike LMSW) can clear PE");
        assert_eq!(cpu.cr0, 0x6000_0010);
    }

    /// MOV to/from CR1 is architecturally undefined — #UD via the real-mode IVT.
    /// Spec: Intel SDM Vol. 2 "MOV—Move to/from Control Registers"
    /// ("Attempts to reference CR1 ... result in undefined opcode (#UD)").
    #[test]
    fn mov_cr1_is_ud_via_ivt() {
        let mut mem = vec![0u8; 0x10000];
        // IVT #UD vector 6 → handler at 0x0B00.
        mem[6 * 4] = 0x00;
        mem[6 * 4 + 1] = 0x0B;
        mem[6 * 4 + 2] = 0x00;
        mem[6 * 4 + 3] = 0x00;
        let code = 0x1000usize;
        // +0: 0F 20 C8   MOV EAX, CR1  (reg=1) → #UD
        // +3: F4         HLT (unreached)
        mem[code] = 0x0F;
        mem[code + 1] = 0x20;
        mem[code + 2] = 0xC8; // 11_001_000: reg=1 (CR1), rm=0 (EAX)
        mem[code + 3] = 0xF4;
        mem[0xB00] = 0xF4; // handler HLT

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.cs.selector, 0);
        assert_eq!(cpu.ip16(), 0x0B00);
    }

    /// MOV to/from CR2/CR3/CR4 are valid on real hardware but out of scope for
    /// this slice — must fail explicitly rather than silently faking behavior.
    /// Spec: Intel SDM Vol. 2 "MOV—Move to/from Control Registers"; Vol. 3 §2.5.
    #[test]
    fn mov_cr2_cr3_cr4_are_explicitly_unsupported() {
        let mut mem = vec![0u8; 0x10000];
        let code = 0x1000usize;
        // +0: 0F 20 D0   MOV EAX, CR2  (reg=2)
        // +3: 0F 22 D8   MOV CR3, EAX  (reg=3)
        // +6: 0F 20 E0   MOV EAX, CR4  (reg=4)
        mem[code] = 0x0F;
        mem[code + 1] = 0x20;
        mem[code + 2] = 0xD0;
        mem[code + 3] = 0x0F;
        mem[code + 4] = 0x22;
        mem[code + 5] = 0xD8;
        mem[code + 6] = 0x0F;
        mem[code + 7] = 0x20;
        mem[code + 8] = 0xE0;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        assert_eq!(step(&mut cpu, &mut bus), Err(ExecError::Unsupported(0x20)));
        cpu.set_ip16(cpu.ip16().wrapping_add(3));
        assert_eq!(step(&mut cpu, &mut bus), Err(ExecError::Unsupported(0x22)));
        cpu.set_ip16(cpu.ip16().wrapping_add(3));
        assert_eq!(step(&mut cpu, &mut bus), Err(ExecError::Unsupported(0x20)));
    }

    /// LIDT/SIDT m16&32 — opcode 0F 01 /3 and /1 (SDM Vol. 2 LIDT/SIDT; Vol. 3 §2.4.3).
    /// Mirrors LGDT/SGDT opsize and mod=11 #UD rules for IDTR.
    #[test]
    fn lidt_sidt_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // IVT #UD vector 6 → handler at 0x0B00
        mem[6 * 4] = 0x00;
        mem[6 * 4 + 1] = 0x0B;
        mem[6 * 4 + 2] = 0x00;
        mem[6 * 4 + 3] = 0x00;
        let code = 0x1000usize;
        // +0: 0F 01 1E 00 40    LIDT [0x4000]  (opsize 16 → 24-bit base)
        // +5: 0F 01 0E 00 50    SIDT [0x5000]
        // +A: 66 0F 01 1E 00 60 LIDT [0x6000] (opsize 32 → 32-bit base)
        // +10: 66 0F 01 0E 00 70 SIDT [0x7000]
        // +16: 0F 01 C9         SIDT ECX (mod=11, /1) → #UD
        // +19: F4               HLT
        mem[code] = 0x0F;
        mem[code + 1] = 0x01;
        mem[code + 2] = 0x1E;
        mem[code + 3] = 0x00;
        mem[code + 4] = 0x40;
        mem[code + 5] = 0x0F;
        mem[code + 6] = 0x01;
        mem[code + 7] = 0x0E;
        mem[code + 8] = 0x00;
        mem[code + 9] = 0x50;
        mem[code + 10] = 0x66;
        mem[code + 11] = 0x0F;
        mem[code + 12] = 0x01;
        mem[code + 13] = 0x1E;
        mem[code + 14] = 0x00;
        mem[code + 15] = 0x60;
        mem[code + 16] = 0x66;
        mem[code + 17] = 0x0F;
        mem[code + 18] = 0x01;
        mem[code + 19] = 0x0E;
        mem[code + 20] = 0x00;
        mem[code + 21] = 0x70;
        mem[code + 22] = 0x0F;
        mem[code + 23] = 0x01;
        mem[code + 24] = 0xC9; // mod=11, reg=1 (SIDT r/m) — #UD
        mem[code + 25] = 0xF4;

        // Pseudo-descriptor at 0x4000: limit=0x03FF, base=0x12ABCDEF (high byte ignored)
        mem[0x4000] = 0xFF;
        mem[0x4001] = 0x03;
        mem[0x4002] = 0xEF;
        mem[0x4003] = 0xCD;
        mem[0x4004] = 0xAB;
        mem[0x4005] = 0x12;

        // Pseudo-descriptor at 0x6000: limit=0x07FF, base=0xCAFEBABE
        mem[0x6000] = 0xFF;
        mem[0x6001] = 0x07;
        mem[0x6002] = 0xBE;
        mem[0x6003] = 0xBA;
        mem[0x6004] = 0xFE;
        mem[0x6005] = 0xCA;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.idtr.limit, 0x03FF);
        assert_eq!(
            cpu.idtr.base, 0x00AB_CDEF,
            "16-bit opsize truncates base to 24 bits"
        );

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x5000).unwrap(), 0x03FF);
        assert_eq!(bus.read_u32(0x5002).unwrap(), 0x00AB_CDEF);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.idtr.limit, 0x07FF);
        assert_eq!(cpu.idtr.base, 0xCAFE_BABE);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x7000).unwrap(), 0x07FF);
        assert_eq!(bus.read_u32(0x7002).unwrap(), 0xCAFE_BABE);

        // Register form → #UD via IVT (IDTR still at 0xCAFEBABE would miss;
        // restore IVT base first so delivery uses low memory).
        cpu.idtr.base = 0;
        cpu.idtr.limit = 0x03FF;
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0B00);
        assert_eq!(cpu.cs.selector, 0);
    }

    /// INVLPG m — opcode 0F 01 /7 (SDM Vol. 2 "INVLPG—Invalidate TLB Entries").
    /// Real-address mode: memory form is an architectural NOP (TLB-less; no paging).
    /// Register form (mod=11) → #UD via IVT. Does not modify GPRs or CR0 / enable PE/PM.
    #[test]
    fn invlpg_real_mode_nop_and_reg_ud() {
        let mut mem = vec![0u8; 0x10000];
        // IVT #UD vector 6 → handler at 0x0B00
        mem[6 * 4] = 0x00;
        mem[6 * 4 + 1] = 0x0B;
        mem[6 * 4 + 2] = 0x00;
        mem[6 * 4 + 3] = 0x00;
        let code = 0x1000usize;
        // +0: 0F 01 3E 00 40    INVLPG [0x4000]  (memory form → NOP)
        // +5: 0F 01 F8         INVLPG EAX        (mod=11 → #UD)
        // +8: F4               HLT
        mem[code] = 0x0F;
        mem[code + 1] = 0x01;
        mem[code + 2] = 0x3E;
        mem[code + 3] = 0x00;
        mem[code + 4] = 0x40;
        mem[code + 5] = 0x0F;
        mem[code + 6] = 0x01;
        mem[code + 7] = 0xF8; // mod=11, reg=7, rm=EAX
        mem[code + 8] = 0xF4;
        // Sentinel at operand address — INVLPG must not read or write it.
        mem[0x4000] = 0xA5;
        mem[0x4001] = 0x5A;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_gpr_u32(CpuState::RAX, 0x1122_3344);
        cpu.set_gpr_u32(CpuState::RBX, 0x5566_7788);
        let cr0_before = cpu.cr0;
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap(); // INVLPG [0x4000]
        assert_eq!(cpu.rip, (code + 5) as u64, "memory INVLPG advances IP");
        assert_eq!(cpu.gpr_u32(CpuState::RAX), 0x1122_3344, "GPRs unchanged");
        assert_eq!(cpu.gpr_u32(CpuState::RBX), 0x5566_7788, "GPRs unchanged");
        assert_eq!(cpu.cr0, cr0_before, "CR0 unchanged (no PE/PM side effects)");
        assert_eq!(bus.mem[0x4000], 0xA5, "operand memory not accessed");
        assert_eq!(bus.mem[0x4001], 0x5A, "operand memory not accessed");

        // Register form → #UD via IVT
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0B00);
        assert_eq!(cpu.cs.selector, 0);
        assert_eq!(cpu.cr0, cr0_before, "CR0 still unchanged after #UD path");
    }

    /// LGDT/SGDT m16&32 — opcode 0F 01 /2 and /0 (SDM Vol. 2 LGDT/SGDT; Vol. 3 §2.4.1).
    /// Real-mode opsize-16 uses 24-bit base; 0x66 uses full 32-bit base. mod=11 → #UD.
    #[test]
    fn lgdt_sgdt_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // IVT #UD vector 6 → handler at 0x0B00 (keep code out of IVT bytes 0..0x400)
        mem[6 * 4] = 0x00;
        mem[6 * 4 + 1] = 0x0B;
        mem[6 * 4 + 2] = 0x00;
        mem[6 * 4 + 3] = 0x00;
        let code = 0x1000usize;
        // +0: 0F 01 16 00 40    LGDT [0x4000]  (opsize 16 → 24-bit base)
        // +5: 0F 01 06 00 50    SGDT [0x5000]
        // +A: 66 0F 01 16 00 60 LGDT [0x6000] (opsize 32 → 32-bit base)
        // +10: 66 0F 01 06 00 70 SGDT [0x7000]
        // +16: 0F 01 C0         SGDT EAX (mod=11) → #UD
        // +19: F4               HLT
        mem[code] = 0x0F;
        mem[code + 1] = 0x01;
        mem[code + 2] = 0x16;
        mem[code + 3] = 0x00;
        mem[code + 4] = 0x40;
        mem[code + 5] = 0x0F;
        mem[code + 6] = 0x01;
        mem[code + 7] = 0x06;
        mem[code + 8] = 0x00;
        mem[code + 9] = 0x50;
        mem[code + 10] = 0x66;
        mem[code + 11] = 0x0F;
        mem[code + 12] = 0x01;
        mem[code + 13] = 0x16;
        mem[code + 14] = 0x00;
        mem[code + 15] = 0x60;
        mem[code + 16] = 0x66;
        mem[code + 17] = 0x0F;
        mem[code + 18] = 0x01;
        mem[code + 19] = 0x06;
        mem[code + 20] = 0x00;
        mem[code + 21] = 0x70;
        mem[code + 22] = 0x0F;
        mem[code + 23] = 0x01;
        mem[code + 24] = 0xC0; // mod=11, reg=0 (SGDT r/m) — #UD
        mem[code + 25] = 0xF4;

        // Pseudo-descriptor at 0x4000: limit=0x0027, base=0x12ABCDEF (high byte ignored)
        mem[0x4000] = 0x27;
        mem[0x4001] = 0x00;
        mem[0x4002] = 0xEF;
        mem[0x4003] = 0xCD;
        mem[0x4004] = 0xAB;
        mem[0x4005] = 0x12;

        // Pseudo-descriptor at 0x6000: limit=0xFFFF, base=0xDEADBEEF
        mem[0x6000] = 0xFF;
        mem[0x6001] = 0xFF;
        mem[0x6002] = 0xEF;
        mem[0x6003] = 0xBE;
        mem[0x6004] = 0xAD;
        mem[0x6005] = 0xDE;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gdtr.limit, 0x0027);
        assert_eq!(
            cpu.gdtr.base, 0x00AB_CDEF,
            "16-bit opsize truncates base to 24 bits"
        );

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x5000).unwrap(), 0x0027);
        assert_eq!(bus.read_u32(0x5002).unwrap(), 0x00AB_CDEF);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gdtr.limit, 0xFFFF);
        assert_eq!(cpu.gdtr.base, 0xDEAD_BEEF);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x7000).unwrap(), 0xFFFF);
        assert_eq!(bus.read_u32(0x7002).unwrap(), 0xDEAD_BEEF);

        // Register form → #UD via IVT
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0B00);
        assert_eq!(cpu.cs.selector, 0);
    }

    /// Operand-size override 0x66: MOV/PUSH/POP/ALU 32-bit in real mode.
    /// Spec: Intel SDM Vol. 2 Ch. 2 (66H); Vol. 1 §3.6; instruction pages MOV/PUSH/POP/ADD.
    /// Segment model remains real-mode (selector<<4); without 0x66 stays 16-bit.
    #[test]
    fn opsize32_mov_push_pop_alu_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 66 B8 78 56 34 12  = MOV EAX, 0x12345678
        mem[0] = 0x66;
        mem[1] = 0xB8;
        mem[2] = 0x78;
        mem[3] = 0x56;
        mem[4] = 0x34;
        mem[5] = 0x12;
        // 66 BB 01 00 00 00  = MOV EBX, 1
        mem[6] = 0x66;
        mem[7] = 0xBB;
        mem[8] = 0x01;
        mem[9] = 0x00;
        mem[10] = 0x00;
        mem[11] = 0x00;
        // 66 01 D8          = ADD EAX, EBX
        mem[12] = 0x66;
        mem[13] = 0x01;
        mem[14] = 0xD8;
        // 66 50             = PUSH EAX
        mem[15] = 0x66;
        mem[16] = 0x50;
        // 66 5A             = POP EDX
        mem[17] = 0x66;
        mem[18] = 0x5A;
        // 66 3D 79 56 34 12 = CMP EAX, 0x12345679
        mem[19] = 0x66;
        mem[20] = 0x3D;
        mem[21] = 0x79;
        mem[22] = 0x56;
        mem[23] = 0x34;
        mem[24] = 0x12;
        // B8 CD AB          = MOV AX, 0xABCD (no 0x66 → 16-bit)
        mem[25] = 0xB8;
        mem[26] = 0xCD;
        mem[27] = 0xAB;
        mem[28] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        // Prove real-mode segment base = selector<<4 (unchanged by opsize).
        assert_eq!(cpu.ds.base, 0);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap(); // MOV EAX
        assert_eq!(cpu.eax(), 0x1234_5678);
        step(&mut cpu, &mut bus).unwrap(); // MOV EBX
        assert_eq!(cpu.gpr_u32(CpuState::RBX), 1);
        step(&mut cpu, &mut bus).unwrap(); // ADD EAX, EBX
        assert_eq!(cpu.eax(), 0x1234_5679);
        assert_eq!(cpu.rflags & 1, 0); // CF clear
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF clear

        step(&mut cpu, &mut bus).unwrap(); // PUSH EAX
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFA);
        assert_eq!(bus.read_u32(0xFFFA).unwrap(), 0x1234_5679);

        step(&mut cpu, &mut bus).unwrap(); // POP EDX
        assert_eq!(cpu.gpr_u32(CpuState::RDX), 0x1234_5679);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE);

        step(&mut cpu, &mut bus).unwrap(); // CMP EAX, imm32
        assert_eq!(cpu.eax(), 0x1234_5679); // unchanged
        assert_ne!(cpu.rflags & (1 << 6), 0); // ZF

        step(&mut cpu, &mut bus).unwrap(); // MOV AX,imm16 without 0x66
        assert_eq!(cpu.ax(), 0xABCD);
        // set_gpr_u16 preserves bits 31:16 of EAX.
        assert_eq!(cpu.eax(), 0x1234_ABCD);
        assert_eq!(cpu.ds.base, 0); // still real-mode flat DS
    }

    /// 0x66 ALU memory form + near CALL/RET with opsize 32.
    /// Spec: Intel SDM Vol. 2 ADD/XOR; "CALL"/"RET" near; Ch. 2 (66H).
    #[test]
    fn opsize32_alu_mem_and_near_call_ret() {
        let mut mem = vec![0u8; 0x10000];
        mem[0x4000] = 0x10;
        mem[0x4001] = 0x00;
        mem[0x4002] = 0x00;
        mem[0x4003] = 0x00;

        // 66 81 06 00 40 EF BE AD DE = ADD dword [0x4000], 0xDEADBEEF
        mem[0] = 0x66;
        mem[1] = 0x81;
        mem[2] = 0x06;
        mem[3] = 0x00;
        mem[4] = 0x40;
        mem[5] = 0xEF;
        mem[6] = 0xBE;
        mem[7] = 0xAD;
        mem[8] = 0xDE;
        // 66 31 C0 = XOR EAX, EAX
        mem[9] = 0x66;
        mem[10] = 0x31;
        mem[11] = 0xC0;
        // 66 E8 08 00 00 00 = CALL rel32; next=18, target=26 (RET)
        mem[12] = 0x66;
        mem[13] = 0xE8;
        mem[14] = 0x08;
        mem[15] = 0x00;
        mem[16] = 0x00;
        mem[17] = 0x00;
        // return site: 66 05 01 00 00 00 = ADD EAX, 1
        mem[18] = 0x66;
        mem[19] = 0x05;
        mem[20] = 0x01;
        mem[21] = 0x00;
        mem[22] = 0x00;
        mem[23] = 0x00;
        mem[24] = 0xF4; // HLT
                        // subroutine: 66 C3 = RET
        mem[26] = 0x66;
        mem[27] = 0xC3;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap(); // ADD [mem], imm32
        assert_eq!(bus.read_u32(0x4000).unwrap(), 0xDEAD_BEFF);
        assert_eq!(cpu.rflags & 1, 0);

        step(&mut cpu, &mut bus).unwrap(); // XOR EAX,EAX
        assert_eq!(cpu.eax(), 0);
        assert_ne!(cpu.rflags & (1 << 6), 0);

        step(&mut cpu, &mut bus).unwrap(); // CALL → RET at 26
        assert_eq!(cpu.ip16(), 26);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFA);
        assert_eq!(bus.read_u32(0xFFFA).unwrap(), 18);

        step(&mut cpu, &mut bus).unwrap(); // RET → 18
        assert_eq!(cpu.ip16(), 18);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE);

        step(&mut cpu, &mut bus).unwrap(); // ADD EAX, 1
        assert_eq!(cpu.eax(), 1);
    }

    /// 0x66 tranche-3: INC/DEC r32, XCHG EAX,r32, CWDE/CDQ, TEST EAX,imm32.
    /// Spec: Intel SDM Vol. 2 INC/DEC/XCHG/CBW/CWDE/CWD/CDQ/TEST; Ch. 2 (66H).
    #[test]
    fn opsize32_inc_dec_xchg_cwde_cdq_test_eax() {
        let mut mem = vec![0u8; 0x10000];
        // 66 40 = INC EAX
        mem[0] = 0x66;
        mem[1] = 0x40;
        // 66 48 = DEC EAX
        mem[2] = 0x66;
        mem[3] = 0x48;
        // 66 FF C3 = INC EBX (Group5 /0 r32)
        mem[4] = 0x66;
        mem[5] = 0xFF;
        mem[6] = 0xC3;
        // 66 FF CB = DEC EBX (Group5 /1 r32)
        mem[7] = 0x66;
        mem[8] = 0xFF;
        mem[9] = 0xCB;
        // 66 93 = XCHG EAX, EBX
        mem[10] = 0x66;
        mem[11] = 0x93;
        // 66 98 = CWDE
        mem[12] = 0x66;
        mem[13] = 0x98;
        // 66 99 = CDQ
        mem[14] = 0x66;
        mem[15] = 0x99;
        // 66 A9 EF BE AD DE = TEST EAX, 0xDEADBEEF
        mem[16] = 0x66;
        mem[17] = 0xA9;
        mem[18] = 0xEF;
        mem[19] = 0xBE;
        mem[20] = 0xAD;
        mem[21] = 0xDE;
        mem[22] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_eax(0x0FFF_FFFF);
        cpu.set_gpr_u32(CpuState::RBX, 0x10);
        cpu.set_cf(true); // INC/DEC must preserve CF
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap(); // INC EAX
        assert_eq!(cpu.eax(), 0x1000_0000);
        assert!(cpu.rflags & 1 != 0); // CF preserved
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF clear
        assert_eq!(cpu.rflags & (1 << 7), 0); // SF clear

        step(&mut cpu, &mut bus).unwrap(); // DEC EAX
        assert_eq!(cpu.eax(), 0x0FFF_FFFF);
        assert!(cpu.rflags & 1 != 0);

        step(&mut cpu, &mut bus).unwrap(); // INC EBX
        assert_eq!(cpu.gpr_u32(CpuState::RBX), 0x11);
        step(&mut cpu, &mut bus).unwrap(); // DEC EBX
        assert_eq!(cpu.gpr_u32(CpuState::RBX), 0x10);

        step(&mut cpu, &mut bus).unwrap(); // XCHG EAX, EBX
        assert_eq!(cpu.eax(), 0x10);
        assert_eq!(cpu.gpr_u32(CpuState::RBX), 0x0FFF_FFFF);

        // AX = 0x8000 → CWDE → EAX = 0xFFFF_8000
        cpu.set_eax(0x0000_8000);
        step(&mut cpu, &mut bus).unwrap(); // CWDE
        assert_eq!(cpu.eax(), 0xFFFF_8000);

        step(&mut cpu, &mut bus).unwrap(); // CDQ
        assert_eq!(cpu.gpr_u32(CpuState::RDX), 0xFFFF_FFFF);
        assert_eq!(cpu.eax(), 0xFFFF_8000);

        // TEST EAX, 0xDEADBEEF → EAX & imm = 0xDEAD_8000; SF=1 ZF=0
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.eax(), 0xFFFF_8000); // unchanged
        assert_eq!(cpu.rflags & 1, 0); // CF cleared
        assert_eq!(cpu.rflags & (1 << 11), 0); // OF cleared
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF
    }

    /// 0x66 LES/LDS r32,m16:32 — Spec: Intel SDM Vol. 2 LES/LDS; Ch. 2 (66H).
    #[test]
    fn opsize32_les_lds_r32() {
        let mut mem = vec![0u8; 0x10000];
        // Far ptr32 at 0x2000: offset 0x12345678, selector 0x1000
        mem[0x2000] = 0x78;
        mem[0x2001] = 0x56;
        mem[0x2002] = 0x34;
        mem[0x2003] = 0x12;
        mem[0x2004] = 0x00;
        mem[0x2005] = 0x10;
        // Far ptr32 at 0x3000: offset 0xABCDEF01, selector 0xF000
        mem[0x3000] = 0x01;
        mem[0x3001] = 0xEF;
        mem[0x3002] = 0xCD;
        mem[0x3003] = 0xAB;
        mem[0x3004] = 0x00;
        mem[0x3005] = 0xF0;
        // 66 C4 06 00 20 = LES EAX, [0x2000]
        mem[0] = 0x66;
        mem[1] = 0xC4;
        mem[2] = 0x06;
        mem[3] = 0x00;
        mem[4] = 0x20;
        // 66 C5 1E 00 30 = LDS EBX, [0x3000]
        mem[5] = 0x66;
        mem[6] = 0xC5;
        mem[7] = 0x1E;
        mem[8] = 0x00;
        mem[9] = 0x30;
        mem[10] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0x9999);
        cpu.rip = 0;
        cpu.rflags = 0x246;
        let flags_before = cpu.rflags;
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.eax(), 0x1234_5678);
        assert_eq!(cpu.es.selector, 0x1000);
        assert_eq!(cpu.es.base, 0x1000u64 << 4);
        assert_eq!(cpu.rflags, flags_before);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RBX), 0xABCD_EF01);
        assert_eq!(cpu.ds.selector, 0xF000);
        assert_eq!(cpu.ds.base, 0xF000u64 << 4);
        assert_eq!(cpu.rflags, flags_before);
    }

    /// 0x66 BOUND r32,m32&32 — Spec: Intel SDM Vol. 2 BOUND; Vol. 3 §6.15 (#BR).
    #[test]
    fn opsize32_bound_r32() {
        let mut mem = vec![0u8; 0x10000];
        // Bounds at 0x2000: lower=0x10, upper=0x20
        mem[0x2000] = 0x10;
        mem[0x2001] = 0x00;
        mem[0x2002] = 0x00;
        mem[0x2003] = 0x00;
        mem[0x2004] = 0x20;
        mem[0x2005] = 0x00;
        mem[0x2006] = 0x00;
        mem[0x2007] = 0x00;
        // IVT[5] → 0000:0B00
        mem[20] = 0x00;
        mem[21] = 0x0B;
        mem[22] = 0x00;
        mem[23] = 0x00;
        // 66 62 06 00 20 = BOUND EAX, [0x2000]
        mem[0] = 0x66;
        mem[1] = 0x62;
        mem[2] = 0x06;
        mem[3] = 0x00;
        mem[4] = 0x20;
        mem[0xB00] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_eax(0x0000_000F); // below lower → #BR
        cpu.set_interrupt_flag(true);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0B00);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0); // fault IP

        // Inclusive endpoints succeed
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_eax(0x10);
        cpu.halted = false;
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 5);
        cpu.rip = 0;
        cpu.set_eax(0x20);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 5);
    }

    /// 0x66 far CALL/JMP/RETF ptr16:32 and Group5 m16:32.
    /// Spec: Intel SDM Vol. 2 CALL/JMP/RET; Ch. 2 (66H). Real-mode OsZ32 → 6-byte frame.
    #[test]
    fn opsize32_far_call_jmp_retf_ptr16_32() {
        let mut mem = vec![0u8; 0x20000];
        // Far pointer memory at DS:0x4000 → 0x1000:0x0200
        mem[0x4000] = 0x00;
        mem[0x4001] = 0x02;
        mem[0x4002] = 0x00;
        mem[0x4003] = 0x00;
        mem[0x4004] = 0x00;
        mem[0x4005] = 0x10;
        // Target at 0x1000:0x0200 = linear 0x10200: 66 CB RETF
        let target = (0x1000u32 << 4) + 0x0200;
        mem[target as usize] = 0x66;
        mem[target as usize + 1] = 0xCB;
        // Landing: HLT
        mem[0x20] = 0xF4;

        // 66 9A 00 02 00 00 00 10 = CALL FAR 1000:00000200
        mem[0] = 0x66;
        mem[1] = 0x9A;
        mem[2] = 0x00;
        mem[3] = 0x02;
        mem[4] = 0x00;
        mem[5] = 0x00;
        mem[6] = 0x00;
        mem[7] = 0x10;
        // After RETF lands here (IP=8): NOP pad then JMP FAR mem
        // 66 FF 2E 00 40 = JMP FAR dword [0x4000]
        mem[8] = 0x66;
        mem[9] = 0xFF;
        mem[10] = 0x2E;
        mem[11] = 0x00;
        mem[12] = 0x40;
        // After second RETF would be HLT at 0x20 — rewrite target after first return
        // Also exercise Group5 CALL FAR: place at 0x30
        // 66 FF 1E 00 40 = CALL FAR [0x4000]
        mem[0x30] = 0x66;
        mem[0x31] = 0xFF;
        mem[0x32] = 0x1E;
        mem[0x33] = 0x00;
        mem[0x34] = 0x40;
        mem[0x35] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap(); // CALL FAR ptr16:32
        assert_eq!(cpu.cs.selector, 0x1000);
        assert_eq!(cpu.ip16(), 0x0200);
        // 6-byte frame: EIP32 then CS16 above it on stack growth down
        // SP was FFFE; push CS (−2→FFFC), push EIP (−4→FFF8)
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF8);
        assert_eq!(bus.read_u32(0xFFF8).unwrap(), 8); // return EIP
        assert_eq!(bus.read_u16(0xFFFC).unwrap(), 0); // saved CS

        step(&mut cpu, &mut bus).unwrap(); // RETF opsize32
        assert_eq!(cpu.cs.selector, 0);
        assert_eq!(cpu.ip16(), 8);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE);

        // JMP FAR m16:32 to same target (no stack) — overwrite RETF with HLT for landing
        bus.mem[target as usize] = 0xF4;
        step(&mut cpu, &mut bus).unwrap(); // JMP FAR [0x4000]
        assert_eq!(cpu.cs.selector, 0x1000);
        assert_eq!(cpu.ip16(), 0x0200);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE); // unchanged

        // Group5 CALL FAR m16:32
        bus.mem[target as usize] = 0x66;
        bus.mem[target as usize + 1] = 0xCB;
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0x30;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.cs.selector, 0x1000);
        assert_eq!(cpu.ip16(), 0x0200);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF8);
        assert_eq!(bus.read_u32(0xFFF8).unwrap(), 0x35); // next after CALL
        step(&mut cpu, &mut bus).unwrap(); // RETF
        assert_eq!(cpu.cs.selector, 0);
        assert_eq!(cpu.ip16(), 0x35);
    }

    /// 0x66 tranche-4: MOV moffs EAX (A1/A3), POP r/m32 (8F), MOV r32←Sreg (8C).
    /// Spec: Intel SDM Vol. 2 MOV/POP; Ch. 2 (66H); Vol. 1 §3.6.
    #[test]
    fn opsize32_moffs_eax_pop_rm32_mov_sreg_r32() {
        let mut mem = vec![0u8; 0x10000];
        // moffs dword at DS:0x3000
        mem[0x3000] = 0x78;
        mem[0x3001] = 0x56;
        mem[0x3002] = 0x34;
        mem[0x3003] = 0x12;
        // 66 A1 00 30 = MOV EAX, moffs16 0x3000
        mem[0] = 0x66;
        mem[1] = 0xA1;
        mem[2] = 0x00;
        mem[3] = 0x30;
        // 66 A3 00 40 = MOV moffs16 0x4000, EAX
        mem[4] = 0x66;
        mem[5] = 0xA3;
        mem[6] = 0x00;
        mem[7] = 0x40;
        // 66 8C D8 = MOV EAX, DS (zero-extend selector)
        mem[8] = 0x66;
        mem[9] = 0x8C;
        mem[10] = 0xD8;
        // 66 8C 06 00 50 = MOV [0x5000], ES — memory dest still 16-bit store
        mem[11] = 0x66;
        mem[12] = 0x8C;
        mem[13] = 0x06;
        mem[14] = 0x00;
        mem[15] = 0x50;
        // 66 8F C3 = POP EBX
        mem[16] = 0x66;
        mem[17] = 0x8F;
        mem[18] = 0xC3;
        // 66 8F 06 00 60 = POP dword [0x6000]
        mem[19] = 0x66;
        mem[20] = 0x8F;
        mem[21] = 0x06;
        mem[22] = 0x00;
        mem[23] = 0x60;
        mem[24] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0xABCD);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFA);
        // Stack: dword 0x11111111 at SP=0xFFFA; dword 0x22222222 at SP=0xFFF6
        mem[0xFFFA] = 0x11;
        mem[0xFFFB] = 0x11;
        mem[0xFFFC] = 0x11;
        mem[0xFFFD] = 0x11;
        mem[0xFFF6] = 0x22;
        mem[0xFFF7] = 0x22;
        mem[0xFFF8] = 0x22;
        mem[0xFFF9] = 0x22;
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap(); // MOV EAX, moffs
        assert_eq!(cpu.eax(), 0x1234_5678);

        step(&mut cpu, &mut bus).unwrap(); // MOV moffs, EAX
        assert_eq!(bus.read_u32(0x4000).unwrap(), 0x1234_5678);

        cpu.set_eax(0xDEAD_BEEF);
        cpu.ds = x86_core::SegmentReg::real_mode(0x1234);
        step(&mut cpu, &mut bus).unwrap(); // MOV EAX, DS
        assert_eq!(cpu.eax(), 0x0000_1234);

        // Poison high word of memory so 16-bit store is observable.
        bus.mem[0x5000] = 0xFF;
        bus.mem[0x5001] = 0xFF;
        bus.mem[0x5002] = 0xEE;
        bus.mem[0x5003] = 0xEE;
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        step(&mut cpu, &mut bus).unwrap(); // MOV [0x5000], ES
        assert_eq!(bus.read_u16(0x5000).unwrap(), 0xABCD);
        assert_eq!(bus.mem[0x5002], 0xEE); // upper bytes untouched
        assert_eq!(bus.mem[0x5003], 0xEE);

        step(&mut cpu, &mut bus).unwrap(); // POP EBX
        assert_eq!(cpu.gpr_u32(CpuState::RBX), 0x1111_1111);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE);

        cpu.set_gpr_u16(CpuState::RSP, 0xFFF6);
        step(&mut cpu, &mut bus).unwrap(); // POP dword [0x6000]
        assert_eq!(bus.read_u32(0x6000).unwrap(), 0x2222_2222);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFA);
    }

    /// 0x66 Group 2 D1/C1 and Group 3 F7 dword forms.
    /// Spec: Intel SDM Vol. 2 ROL/ROR/RCL/RCR/SHL/SHR/SAR; TEST/NOT/NEG/MUL/IMUL/DIV/IDIV; Ch. 2.
    #[test]
    fn opsize32_grp2_d1_c1_and_grp3_f7() {
        let mut mem = vec![0u8; 0x10000];
        // 66 D1 E0       = SHL EAX, 1
        mem[0] = 0x66;
        mem[1] = 0xD1;
        mem[2] = 0xE0;
        // 66 C1 E8 04    = SHR EAX, 4
        mem[3] = 0x66;
        mem[4] = 0xC1;
        mem[5] = 0xE8;
        mem[6] = 0x04;
        // 66 D1 C0       = ROL EAX, 1
        mem[7] = 0x66;
        mem[8] = 0xD1;
        mem[9] = 0xC0;
        // 66 F7 D0       = NOT EAX
        mem[10] = 0x66;
        mem[11] = 0xF7;
        mem[12] = 0xD0;
        // 66 F7 D8       = NEG EAX
        mem[13] = 0x66;
        mem[14] = 0xF7;
        mem[15] = 0xD8;
        // 66 F7 C0 EF BE AD DE = TEST EAX, 0xDEADBEEF
        mem[16] = 0x66;
        mem[17] = 0xF7;
        mem[18] = 0xC0;
        mem[19] = 0xEF;
        mem[20] = 0xBE;
        mem[21] = 0xAD;
        mem[22] = 0xDE;
        // 66 F7 E3       = MUL EBX
        mem[23] = 0x66;
        mem[24] = 0xF7;
        mem[25] = 0xE3;
        // 66 F7 EB       = IMUL EBX
        mem[26] = 0x66;
        mem[27] = 0xF7;
        mem[28] = 0xEB;
        // 66 F7 F3       = DIV EBX
        mem[29] = 0x66;
        mem[30] = 0xF7;
        mem[31] = 0xF3;
        // 66 F7 FB       = IDIV EBX
        mem[32] = 0x66;
        mem[33] = 0xF7;
        mem[34] = 0xFB;
        // 66 F7 06 00 40 = NOT dword [0x4000]
        mem[35] = 0x66;
        mem[36] = 0xF7;
        mem[37] = 0x16;
        mem[38] = 0x00;
        mem[39] = 0x40; // /2 NOT mem — ModRM 0x16 = mod=00 reg=2 rm=6 → [disp16]
        mem[40] = 0xF4;
        mem[0x4000] = 0x0F;
        mem[0x4001] = 0x0F;
        mem[0x4002] = 0x0F;
        mem[0x4003] = 0x0F;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };

        cpu.set_eax(0x4000_0000);
        step(&mut cpu, &mut bus).unwrap(); // SHL EAX,1
        assert_eq!(cpu.eax(), 0x8000_0000);
        assert_eq!(cpu.rflags & 1, 0); // CF
        assert_ne!(cpu.rflags & (1 << 11), 0); // OF
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF

        step(&mut cpu, &mut bus).unwrap(); // SHR EAX,4
        assert_eq!(cpu.eax(), 0x0800_0000);

        cpu.set_eax(0x8000_0000);
        step(&mut cpu, &mut bus).unwrap(); // ROL EAX,1
        assert_eq!(cpu.eax(), 0x0000_0001);
        assert_ne!(cpu.rflags & 1, 0); // CF=1

        cpu.set_eax(0x0F0F_0F0F);
        let flags_before = cpu.rflags;
        step(&mut cpu, &mut bus).unwrap(); // NOT EAX
        assert_eq!(cpu.eax(), 0xF0F0_F0F0);
        assert_eq!(cpu.rflags, flags_before);

        cpu.set_eax(1);
        step(&mut cpu, &mut bus).unwrap(); // NEG EAX
        assert_eq!(cpu.eax(), 0xFFFF_FFFF);
        assert_ne!(cpu.rflags & 1, 0); // CF
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF

        // Imm high half must participate: 0x12345678 & 0xFFFF0000 = 0x12340000 (ZF clear).
        // A mistaken imm16 decode (0x0000) would yield ZF set — catch length too (IP += 7).
        cpu.set_eax(0x1234_5678);
        let ip_before_test = cpu.ip16();
        step(&mut cpu, &mut bus).unwrap(); // TEST EAX, 0xDEADBEEF
        assert_eq!(cpu.ip16(), ip_before_test + 7);
        assert_eq!(cpu.eax(), 0x1234_5678); // unchanged
        assert_eq!(cpu.rflags & 1, 0); // CF
        assert_eq!(cpu.rflags & (1 << 11), 0); // OF
                                               // Result 0x12341668: ZF clear, SF clear.
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF
        assert_eq!(cpu.rflags & (1 << 7), 0); // SF

        // MUL EBX: EAX=2, EBX=3 → EDX:EAX = 0:6; CF=OF=0
        cpu.set_eax(2);
        cpu.set_gpr_u32(CpuState::RBX, 3);
        cpu.set_gpr_u32(CpuState::RDX, 0xFFFF_FFFF);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.eax(), 6);
        assert_eq!(cpu.gpr_u32(CpuState::RDX), 0);
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 11), 0);

        // IMUL EBX: EAX=-2, EBX=-3 → 6; fits in i32 → CF=OF=0
        cpu.set_eax(0xFFFF_FFFE);
        cpu.set_gpr_u32(CpuState::RBX, 0xFFFF_FFFD);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.eax(), 6);
        assert_eq!(cpu.gpr_u32(CpuState::RDX), 0);
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 11), 0);

        // DIV EBX: EDX:EAX = 0:100 / 7 → quot=14 rem=2
        cpu.set_eax(100);
        cpu.set_gpr_u32(CpuState::RDX, 0);
        cpu.set_gpr_u32(CpuState::RBX, 7);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.eax(), 14);
        assert_eq!(cpu.gpr_u32(CpuState::RDX), 2);

        // IDIV EBX: EDX:EAX = -20 / 3 → quot=-6 rem=-2
        cpu.set_eax((-20i32) as u32);
        cpu.set_gpr_u32(CpuState::RDX, 0xFFFF_FFFF); // sign-extend
        cpu.set_gpr_u32(CpuState::RBX, 3);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.eax(), (-6i32) as u32);
        assert_eq!(cpu.gpr_u32(CpuState::RDX), (-2i32) as u32);

        step(&mut cpu, &mut bus).unwrap(); // NOT dword [0x4000]
        assert_eq!(bus.read_u32(0x4000).unwrap(), 0xF0F0_F0F0);
    }

    /// 0x66 Group 2 D3 r/m32,CL and IMUL 69/6B r32,r/m32,imm.
    /// Spec: Intel SDM Vol. 2 SHL/IMUL; Ch. 2 (66H).
    #[test]
    fn opsize32_grp2_d3_cl_and_imul_69_6b() {
        let mut mem = vec![0u8; 0x10000];
        // 66 D3 E0                   = SHL EAX, CL
        mem[0] = 0x66;
        mem[1] = 0xD3;
        mem[2] = 0xE0;
        // 66 69 D8 02 00 00 00       = IMUL EBX, EAX, 2
        mem[3] = 0x66;
        mem[4] = 0x69;
        mem[5] = 0xD8;
        mem[6] = 0x02;
        mem[7] = 0x00;
        mem[8] = 0x00;
        mem[9] = 0x00;
        // 66 69 D8 00 00 01 00       = IMUL EBX, EAX, 0x00010000
        mem[10] = 0x66;
        mem[11] = 0x69;
        mem[12] = 0xD8;
        mem[13] = 0x00;
        mem[14] = 0x00;
        mem[15] = 0x01;
        mem[16] = 0x00;
        // 66 6B D8 FD                = IMUL EBX, EAX, -3
        mem[17] = 0x66;
        mem[18] = 0x6B;
        mem[19] = 0xD8;
        mem[20] = 0xFD;
        // 66 69 1E 00 40 03 00 00 00 = IMUL EBX, [0x4000], 3
        mem[21] = 0x66;
        mem[22] = 0x69;
        mem[23] = 0x1E;
        mem[24] = 0x00;
        mem[25] = 0x40;
        mem[26] = 0x03;
        mem[27] = 0x00;
        mem[28] = 0x00;
        mem[29] = 0x00;
        mem[30] = 0xF4;
        mem[0x4000] = 0x05;
        mem[0x4001] = 0x00;
        mem[0x4002] = 0x00;
        mem[0x4003] = 0x00;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };

        // SHL EAX, CL: 0x4000_0000 << 1 = 0x8000_0000; CF=0, OF=1
        cpu.set_eax(0x4000_0000);
        cpu.set_gpr_u8_low(CpuState::RCX, 1);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.eax(), 0x8000_0000);
        assert_eq!(cpu.rflags & 1, 0);
        assert_ne!(cpu.rflags & (1 << 11), 0);

        // IMUL EBX, EAX, 2: 3*2=6 fits → CF=OF=0; EAX unchanged
        cpu.set_eax(3);
        cpu.set_gpr_u32(CpuState::RBX, 0xDEAD_BEEF);
        let ip_before = cpu.ip16();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), ip_before + 7); // 66 + 69 + modrm + imm32
        assert_eq!(cpu.gpr_u32(CpuState::RBX), 6);
        assert_eq!(cpu.eax(), 3);
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 11), 0);

        // IMUL EBX, EAX, 0x10000: 0x10000*0x10000 = 0x1_0000_0000 does not fit in i32
        cpu.set_eax(0x0001_0000);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RBX), 0);
        assert_ne!(cpu.rflags & 1, 0);
        assert_ne!(cpu.rflags & (1 << 11), 0);

        // IMUL EBX, EAX, -3: (-2)*(-3)=6 fits
        cpu.set_eax(0xFFFF_FFFE);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RBX), 6);
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 11), 0);

        // IMUL EBX, [0x4000], 3: 5*3=15; memory unchanged
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RBX), 15);
        assert_eq!(bus.read_u32(0x4000).unwrap(), 5);
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 11), 0);
    }

    /// Real-mode 0x67: 32-bit ModRM effective addresses (selector<<4 + EA32).
    /// Spec: Intel SDM Vol. 1 §3.6; Vol. 2 Chapter 2 (address-size attribute).
    #[test]
    fn asize32_modrm_ea_mov_and_lea() {
        let mut mem = vec![0u8; 0x20000];
        // 67 8B 03 = MOV AX, [EBX]
        mem[0] = 0x67;
        mem[1] = 0x8B;
        mem[2] = 0x03;
        // 67 8D 4B 10 = LEA CX, [EBX+0x10]
        mem[3] = 0x67;
        mem[4] = 0x8D;
        mem[5] = 0x4B;
        mem[6] = 0x10;
        // 67 8B 44 24 04 = MOV AX, [ESP+4]
        mem[7] = 0x67;
        mem[8] = 0x8B;
        mem[9] = 0x44;
        mem[10] = 0x24;
        mem[11] = 0x04;
        mem[12] = 0xF4;

        // DS:EBX → linear 0x1000; payload 0xBEEF
        mem[0x1000] = 0xEF;
        mem[0x1001] = 0xBE;
        // SS:ESP+4 → linear 0x3004; payload 0xCAFE
        mem[0x3004] = 0xFE;
        mem[0x3005] = 0xCA;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u32(CpuState::RBX, 0x1000);
        cpu.set_gpr_u32(CpuState::RSP, 0x3000);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RAX), 0xBEEF);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0x1010);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RAX), 0xCAFE);
    }

    /// Absolute disp32 under 0x67 uses DS:(disp32), not EBP.
    /// Spec: Intel SDM Vol. 2 Chapter 2 — mod=00 rm=101 → disp32.
    #[test]
    fn asize32_modrm_disp32_absolute() {
        let mut mem = vec![0u8; 0x20000];
        // 67 8A 05 00 40 00 00 = MOV AL, [0x4000]
        mem[0] = 0x67;
        mem[1] = 0x8A;
        mem[2] = 0x05;
        mem[3] = 0x00;
        mem[4] = 0x40;
        mem[5] = 0x00;
        mem[6] = 0x00;
        mem[7] = 0xF4;
        mem[0x4000] = 0x5A;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u32(CpuState::RBP, 0xFFFF_FFFF); // must not participate
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x5A);
    }

    /// String ops with 0x67 use ESI/EDI (and ECX for REP), not SI/DI/CX.
    /// Spec: Intel SDM Vol. 1 §3.6; Vol. 2 MOVS / REP (address-size attribute).
    #[test]
    fn asize32_movsb_uses_esi_edi_and_rep_ecx() {
        let mut mem = vec![0u8; 0x20000];
        // F3 67 A4 = REP MOVSB
        mem[0] = 0xF3;
        mem[1] = 0x67;
        mem[2] = 0xA4;
        mem[3] = 0xF4;
        mem[0x5000] = 0x11;
        mem[0x5001] = 0x22;
        mem[0x5002] = 0x33;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        // High halves must participate under asize32 (would be ignored with SI/DI/CX).
        cpu.set_gpr_u32(CpuState::RSI, 0x0000_5000);
        cpu.set_gpr_u32(CpuState::RDI, 0x0000_6000);
        cpu.set_gpr_u32(CpuState::RCX, 0x0000_0003);
        cpu.set_direction_flag(false);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x6000).unwrap(), 0x11);
        assert_eq!(bus.read_u8(0x6001).unwrap(), 0x22);
        assert_eq!(bus.read_u8(0x6002).unwrap(), 0x33);
        assert_eq!(cpu.gpr_u32(CpuState::RSI), 0x5003);
        assert_eq!(cpu.gpr_u32(CpuState::RDI), 0x6003);
        assert_eq!(cpu.gpr_u32(CpuState::RCX), 0);
    }

    /// Default real-mode DS limit 64KiB: accesses within 0..=FFFF succeed; 16-bit EA wrap.
    /// Spec: SDM Vol. 3 §3.4.2–§3.4.3, §5.3; docs/cpu-profile-core2.md.
    #[test]
    fn real_mode_default_segment_limit_unchanged() {
        let mut mem = vec![0u8; 0x10000];
        mem[0x1234] = 0xAB;
        mem[0x2000] = 0x5A;
        // A0 34 12 = MOV AL, [0x1234]
        mem[0] = 0xA0;
        mem[1] = 0x34;
        mem[2] = 0x12;
        // 8A 87 FE 1F = MOV AL, [BX+0x1FFE] with BX=2 → EA 0x2000 (16-bit wrap add)
        mem[3] = 0x8A;
        mem[4] = 0x87;
        mem[5] = 0xFE;
        mem[6] = 0x1F;
        mem[7] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        assert_eq!(cpu.ds.limit, 0xFFFF);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RBX, 2);
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xAB);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x5A);
        assert_eq!(cpu.ds.limit, 0xFFFF);
    }

    /// Expanded DS limit (unreal): moffs32 beyond 64KiB succeeds; beyond limit → #GP via IVT.
    /// Spec: SDM Vol. 3 §3.4.3 (cached limit), §5.3, §6.15 (#GP); Vol. 2 MOV moffs.
    #[test]
    fn unreal_expanded_ds_limit_moffs32_and_gp() {
        // --- success path: limit=4GiB-1, read [0x10000] ---
        {
            let mut mem = vec![0u8; 0x20000];
            mem[0x10000] = 0xC3;
            // 67 A0 00 00 01 00 = MOV AL, moffs32 0x10000
            mem[0] = 0x67;
            mem[1] = 0xA0;
            mem[2] = 0x00;
            mem[3] = 0x00;
            mem[4] = 0x01;
            mem[5] = 0x00;
            mem[6] = 0xF4;
            let mut cpu = CpuState::reset();
            cpu.cs = x86_core::SegmentReg::real_mode_code(0);
            cpu.ds = x86_core::SegmentReg::real_mode(0);
            cpu.ds.limit = 0xFFFF_FFFF;
            cpu.rip = 0;
            let mut bus = VecBus { mem, ports: vec![] };
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.al(), 0xC3);
            assert_eq!(cpu.ip16(), 6);
        }

        // --- #GP when offset past cached limit (still >64KiB) ---
        {
            let mut mem = vec![0u8; 0x20000];
            // IVT[13] → 0000:0D00
            mem[13 * 4] = 0x00;
            mem[13 * 4 + 1] = 0x0D;
            mem[13 * 4 + 2] = 0x00;
            mem[13 * 4 + 3] = 0x00;
            mem[0xD00] = 0xF4;
            // 67 A0 00 80 01 00 = MOV AL, [0x18000]
            mem[0] = 0x67;
            mem[1] = 0xA0;
            mem[2] = 0x00;
            mem[3] = 0x80;
            mem[4] = 0x01;
            mem[5] = 0x00;
            let mut cpu = CpuState::reset();
            cpu.cs = x86_core::SegmentReg::real_mode_code(0);
            cpu.ss = x86_core::SegmentReg::real_mode(0);
            cpu.ds = x86_core::SegmentReg::real_mode(0);
            cpu.ds.limit = 0x1_7FFF; // allows 0x10000, not 0x18000
            cpu.rip = 0;
            cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
            cpu.set_interrupt_flag(true);
            let mut bus = VecBus { mem, ports: vec![] };
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.ip16(), 0x0D00);
            assert!(!cpu.interrupt_flag());
            assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0); // faulting IP
        }
    }

    /// Real-mode MOV DS keeps expanded cached limit (sticky unreal descriptor cache).
    /// Spec: SDM Vol. 3 §3.4.2–§3.4.3.
    #[test]
    fn unreal_mov_ds_preserves_expanded_limit() {
        let mut mem = vec![0u8; 0x10000];
        // B8 34 12 = MOV AX, 0x1234; 8E D8 = MOV DS, AX
        mem[0] = 0xB8;
        mem[1] = 0x34;
        mem[2] = 0x12;
        mem[3] = 0x8E;
        mem[4] = 0xD8;
        mem[5] = 0xF4;
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ds.limit = 0xFFFF_FFFF;
        cpu.ds.flags = 0x0093;
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ds.selector, 0x1234);
        assert_eq!(cpu.ds.base, 0x1234u64 << 4);
        assert_eq!(cpu.ds.limit, 0xFFFF_FFFF);
        assert_eq!(cpu.ds.flags, 0x0093);
    }

    /// Reduced limit with 16-bit ModRM EA → #GP via IVT (no asize32 required).
    /// Spec: SDM Vol. 3 §5.3, §6.15 (#GP); Vol. 2 MOV.
    #[test]
    fn segment_limit_gp_modrm_via_ivt() {
        let mut mem = vec![0u8; 0x10000];
        mem[13 * 4] = 0x00;
        mem[13 * 4 + 1] = 0x0D;
        mem[13 * 4 + 2] = 0x00;
        mem[13 * 4 + 3] = 0x00;
        mem[0xD00] = 0xF4;
        // 8A 87 00 90 = MOV AL, [BX+0x9000] with BX=0
        mem[0] = 0x8A;
        mem[1] = 0x87;
        mem[2] = 0x00;
        mem[3] = 0x90;
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ds.limit = 0x7FFF;
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RBX, 0);
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0D00);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0);
    }

    /// Address-size 0x67: LOOP/LOOPcc use ECX; JECXZ tests ECX (SDM Vol. 2 LOOP / JCXZ).
    /// High half of ECX participates (asize16 would only see CX=0 when ECX=0x10000).
    #[test]
    fn asize32_loop_jecxz_uses_ecx() {
        let mut mem = vec![0u8; 0x10000];
        // 0: 67 E2 FD = LOOP $-3 (self; 3-byte insn with 0x67)
        // 3: 67 E3 02 = JECXZ +2
        // 6: F4 F4 F4
        mem[0] = 0x67;
        mem[1] = 0xE2;
        mem[2] = 0xFD;
        mem[3] = 0x67;
        mem[4] = 0xE3;
        mem[5] = 0x02;
        mem[6] = 0xF4;
        mem[7] = 0xF4;
        mem[8] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        // ECX=0x10000 → after dec 0xFFFF ≠ 0 → take; CX alone would already be 0.
        cpu.set_gpr_u32(CpuState::RCX, 0x1_0000);
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RCX), 0xFFFF);
        assert_eq!(cpu.ip16(), 0);

        // Fall-through when ECX becomes 0 (short path; high-half case covered above).
        cpu.set_gpr_u32(CpuState::RCX, 1);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RCX), 0);
        assert_eq!(cpu.ip16(), 3);

        // JECXZ: ECX=0 takes
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 8);
        assert_eq!(cpu.gpr_u32(CpuState::RCX), 0);

        // JECXZ: ECX=0x10000 (CX=0) must NOT take under asize32
        cpu.rip = 3;
        cpu.set_gpr_u32(CpuState::RCX, 0x1_0000);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 6);
        assert_eq!(cpu.gpr_u32(CpuState::RCX), 0x1_0000);
    }

    /// Address-size 0x67: XLAT uses EBX+AL (SDM Vol. 2 XLAT/XLATB; Vol. 1 §3.6).
    #[test]
    fn asize32_xlat_uses_ebx() {
        let mut mem = vec![0u8; 0x20000];
        mem[0x10005] = 0x5A;
        // 67 D7 = XLAT (asize32)
        mem[0] = 0x67;
        mem[1] = 0xD7;
        mem[2] = 0xF4;
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ds.limit = 0xFFFF_FFFF;
        cpu.rip = 0;
        // BX=0 would miss; EBX high half required.
        cpu.set_gpr_u32(CpuState::RBX, 0x1_0000);
        cpu.set_al(0x05);
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x5A);
        assert_eq!(cpu.ip16(), 2);
    }

    /// String ops enforce cached SegmentReg.limit before bus access (parity with ModRM).
    /// Spec: SDM Vol. 3 §5.3 / §6.15; Vol. 2 MOVS.
    #[test]
    fn string_op_segment_limit_gp_via_ivt() {
        let mut mem = vec![0u8; 0x10000];
        mem[13 * 4] = 0x00;
        mem[13 * 4 + 1] = 0x0D;
        mem[13 * 4 + 2] = 0x00;
        mem[13 * 4 + 3] = 0x00;
        mem[0xD00] = 0xF4;
        // A4 = MOVSB; SI=0x9000 past DS.limit=0x7FFF
        mem[0] = 0xA4;
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.ds.limit = 0x7FFF;
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSI, 0x9000);
        cpu.set_gpr_u16(CpuState::RDI, 0x1000);
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0D00);
        // Faulting IP; indices must not advance on limit fault.
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x9000);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x1000);
    }

    /// STOSB ES limit → #GP; SS override on LODSB source → #SS.
    /// Spec: SDM Vol. 3 §5.3, §6.15 (#GP/#SS); Vol. 2 STOS/LODS.
    #[test]
    fn string_op_es_limit_gp_and_ss_override_ss() {
        // STOSB past ES.limit → #GP
        {
            let mut mem = vec![0u8; 0x10000];
            mem[13 * 4] = 0x00;
            mem[13 * 4 + 1] = 0x0D;
            mem[13 * 4 + 2] = 0x00;
            mem[13 * 4 + 3] = 0x00;
            mem[0xD00] = 0xF4;
            mem[0] = 0xAA; // STOSB
            let mut cpu = CpuState::reset();
            cpu.cs = x86_core::SegmentReg::real_mode_code(0);
            cpu.ss = x86_core::SegmentReg::real_mode(0);
            cpu.es = x86_core::SegmentReg::real_mode(0);
            cpu.es.limit = 0x0FFF;
            cpu.rip = 0;
            cpu.set_gpr_u16(CpuState::RDI, 0x2000);
            cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
            let mut bus = VecBus { mem, ports: vec![] };
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.ip16(), 0x0D00);
            assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x2000);
        }
        // LODSB with SS override past SS.limit → #SS
        {
            let mut mem = vec![0u8; 0x10000];
            mem[12 * 4] = 0x00;
            mem[12 * 4 + 1] = 0x0C;
            mem[12 * 4 + 2] = 0x00;
            mem[12 * 4 + 3] = 0x00;
            mem[0xC00] = 0xF4;
            // 36 AC = LODSB SS:
            mem[0] = 0x36;
            mem[1] = 0xAC;
            let mut cpu = CpuState::reset();
            cpu.cs = x86_core::SegmentReg::real_mode_code(0);
            cpu.ss = x86_core::SegmentReg::real_mode(0);
            cpu.ss.limit = 0x0FFF;
            cpu.rip = 0;
            cpu.set_gpr_u16(CpuState::RSI, 0x2000);
            cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
            let mut bus = VecBus { mem, ports: vec![] };
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.ip16(), 0x0C00);
            assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x2000);
        }
    }

    /// CS instruction-fetch past cached limit → #GP via IVT.
    /// Spec: SDM Vol. 3 §5.3, §6.15 (#GP); Vol. 1 §3.3.4 (CS:IP fetch).
    #[test]
    fn cs_fetch_limit_gp_via_ivt() {
        let mut mem = vec![0u8; 0x10000];
        mem[13 * 4] = 0x00;
        mem[13 * 4 + 1] = 0x0D;
        mem[13 * 4 + 2] = 0x00;
        mem[13 * 4 + 3] = 0x00;
        mem[0xD00] = 0xF4;
        mem[0x2000] = 0xF4; // would be HLT if fetch succeeded
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.cs.limit = 0x1FFF;
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0x2000;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0D00);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0x2000);
    }

    /// Non-REP instruction: external IRQ when IF=1 is serviced before fetch/execute.
    /// Spec: Intel SDM Vol. 3 §6.8.1 — saved IP is the interrupted instruction.
    #[test]
    fn non_rep_external_irq_before_instruction() {
        let mut mem = vec![0u8; 0x10000];
        mem[0x20 * 4] = 0x00;
        mem[0x20 * 4 + 1] = 0x0E;
        mem[0x20 * 4 + 2] = 0x00;
        mem[0x20 * 4 + 3] = 0x00;
        mem[0] = 0x90; // NOP — must not execute
        mem[0xE00] = 0xF4;
        mem[0x1000] = 0x00; // sentinel; NOP must not touch memory

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        cpu.request_interrupt(0x20);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.ip16(), 0x0E00);
        assert!(!cpu.interrupt_flag());
        assert_eq!(cpu.pending_irq, None);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0); // saved IP = NOP
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF8);
    }

    /// IF=0: pending IRQ stays latched; non-REP instruction runs normally.
    #[test]
    fn non_rep_external_irq_ignored_when_if_clear() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x90; // NOP
        mem[1] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(false);
        cpu.request_interrupt(0x20);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.ip16(), 1);
        assert_eq!(cpu.pending_irq, Some(0x20));
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE);
    }

    /// Code-fetch bus MemoryFault → #GP via IVT (same classify as CS limit fault).
    /// Spec: Intel SDM Vol. 3 §6.15 (#GP); Vol. 1 §3.3.4 (instruction fetch).
    #[test]
    fn code_fetch_memory_fault_gp_via_ivt() {
        let mut mem = vec![0u8; 0x10000];
        mem[13 * 4] = 0x00;
        mem[13 * 4 + 1] = 0x0D;
        mem[13 * 4 + 2] = 0x00;
        mem[13 * 4 + 3] = 0x00;
        mem[0] = 0x90; // NOP at poisoned fetch address
        mem[0xD00] = 0xF4;
        let poison = 0u64; // CS.base=0, IP=0 → linear 0
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        let mut bus = PoisonBus {
            mem,
            poison,
            tripped: false,
        };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0D00);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0); // fault IP
        assert!(!cpu.interrupt_flag());
    }

    /// ENTER with address-size override (0x67) is Unsupported — needs ESP stack path.
    /// Spec: Intel SDM Vol. 2 "ENTER"; Vol. 1 §3.6 (stack address-size).
    #[test]
    fn enter_asize32_unsupported() {
        let mut mem = vec![0u8; 0x10000];
        // 67 C8 08 00 00 = ENTER 8, 0 with asize32
        mem[0] = 0x67;
        mem[1] = 0xC8;
        mem[2] = 0x08;
        mem[3] = 0x00;
        mem[4] = 0x00;
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };
        let err = step(&mut cpu, &mut bus).unwrap_err();
        assert_eq!(err, ExecError::Unsupported(0xC8));
        assert_eq!(cpu.ip16(), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE);
    }

    /// PUSHA with address-size override (0x67) is Unsupported — needs ESP stack path.
    /// Spec: Intel SDM Vol. 2 "PUSHA/PUSHAD"; Vol. 1 §3.6 (stack address-size).
    #[test]
    fn pusha_asize32_unsupported() {
        let mut mem = vec![0u8; 0x10000];
        // 67 60 = PUSHA with asize32
        mem[0] = 0x67;
        mem[1] = 0x60;
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };
        let err = step(&mut cpu, &mut bus).unwrap_err();
        assert_eq!(err, ExecError::Unsupported(0x60));
        assert_eq!(cpu.ip16(), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE);
    }
}
