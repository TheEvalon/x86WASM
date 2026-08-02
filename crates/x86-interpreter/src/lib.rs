//! Reference interpreter for the Milestone 1 opcode subset.
//!
//! Semantics follow Intel SDM Vol. 2 for the implemented forms only.

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

/// Execute a single instruction at CS:IP.
pub fn step(cpu: &mut CpuState, bus: &mut dyn Bus) -> Result<(), ExecError> {
    if cpu.halted {
        return Ok(());
    }
    let insn = fetch_decode(cpu, bus)?;
    let next_ip = cpu.ip16().wrapping_add(insn.length as u16);
    let op = insn.opcode;

    match op {
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
        0xF5 => {
            let cf = cpu.rflags & 1 != 0;
            cpu.set_cf(!cf);
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
        0xE8 => {
            push16(cpu, bus, next_ip)?;
            let target = next_ip.wrapping_add(insn.immediate as i16 as u16);
            cpu.set_ip16(target);
        }
        0xC3 => {
            let ip = pop16(cpu, bus)?;
            cpu.set_ip16(ip);
        }
        0x74 => {
            // JZ
            if cpu.rflags & (1 << 6) != 0 {
                cpu.set_ip16(next_ip.wrapping_add(insn.immediate as i16 as u16));
            } else {
                cpu.set_ip16(next_ip);
            }
        }
        0x75 => {
            if cpu.rflags & (1 << 6) == 0 {
                cpu.set_ip16(next_ip.wrapping_add(insn.immediate as i16 as u16));
            } else {
                cpu.set_ip16(next_ip);
            }
        }
        0x72 => {
            if cpu.rflags & 1 != 0 {
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
}
