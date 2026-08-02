//! Address translation helpers.
//!
//! Milestone 1: real-mode style `segment.base + offset` (CS.base is the
//! Intel reset base `0xFFFF_0000`). Paging is not enabled (CR0.PG = 0).

#![forbid(unsafe_code)]

use x86_core::{CpuState, SegmentReg};

pub fn linear_addr(seg: &SegmentReg, offset: u64) -> u64 {
    seg.base.wrapping_add(offset)
}

pub fn cs_linear(cpu: &CpuState) -> u64 {
    linear_addr(&cpu.cs, cpu.rip & 0xFFFF)
}

pub fn ss_linear(cpu: &CpuState, offset: u64) -> u64 {
    linear_addr(&cpu.ss, offset & 0xFFFF)
}

pub fn ds_linear(cpu: &CpuState, offset: u64) -> u64 {
    linear_addr(&cpu.ds, offset & 0xFFFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_fetch_address() {
        let cpu = CpuState::reset();
        assert_eq!(cs_linear(&cpu), 0xFFFF_FFF0);
    }
}
