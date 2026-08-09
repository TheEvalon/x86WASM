//! Address translation helpers.
//!
//! Segmentation: real-mode style `segment.base + offset` (CS.base is the Intel
//! reset base `0xFFFF_0000`), with limit checks against the cached effective
//! limit (SDM Vol. 3 §5.3).
//!
//! Paging: [`paging`] is a standalone 32-bit paging translation engine (SDM
//! Vol. 3 Chapter 4). **It is not wired to anything.** The interpreter's memory
//! path still treats a linear address as a physical address, so no guest can
//! reach the engine; see the module documentation and
//! `docs/mmu-r3-32bit-paging.md`.

#![forbid(unsafe_code)]

pub mod paging;

use x86_core::{CpuState, SegmentReg};

pub fn linear_addr(seg: &SegmentReg, offset: u64) -> u64 {
    seg.base.wrapping_add(offset)
}

/// Expand-up segment limit check: every byte of `[offset, offset+size)` must be
/// within `0..=seg.limit` (inclusive max offset).
///
/// Spec: Intel SDM Vol. 3 §5.3 (Limit Checking); §3.4.3 (cached limit).
/// Unsupported here: expand-down segments; long-mode canonical checks.
pub fn offset_within_limit(seg: &SegmentReg, offset: u64, size: u64) -> bool {
    if size == 0 {
        return true;
    }
    let last = match offset.checked_add(size - 1) {
        Some(v) => v,
        None => return false,
    };
    last <= u64::from(seg.limit)
}

/// Linear address after a successful limit check.
///
/// Spec: Intel SDM Vol. 3 §5.3; Vol. 2 MOV real-address `#GP`/`#SS` on limit.
pub fn checked_linear_addr(seg: &SegmentReg, offset: u64, size: u64) -> Result<u64, u64> {
    if offset_within_limit(seg, offset, size) {
        Ok(linear_addr(seg, offset))
    } else {
        Err(offset)
    }
}

pub fn cs_linear(cpu: &CpuState) -> u64 {
    // Instruction fetch still uses IP low 16 bits (real CS).
    // Limit enforcement lives in the interpreter fetch path (`seg_linear_checked`).
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

    /// Default real-mode 64KiB limit: offsets 0..=0xFFFF ok; beyond → fail.
    /// Spec: SDM Vol. 3 §5.3; docs/cpu-profile-core2.md reset limits.
    #[test]
    fn default_real_mode_limit_64k() {
        let seg = SegmentReg::real_mode(0);
        assert_eq!(seg.limit, 0xFFFF);
        assert!(offset_within_limit(&seg, 0, 1));
        assert!(offset_within_limit(&seg, 0xFFFF, 1));
        assert!(offset_within_limit(&seg, 0xFFFE, 2));
        assert!(!offset_within_limit(&seg, 0xFFFF, 2));
        assert!(!offset_within_limit(&seg, 0x1_0000, 1));
        assert!(checked_linear_addr(&seg, 0x100, 1).is_ok());
        assert!(checked_linear_addr(&seg, 0x1_0000, 1).is_err());
    }

    /// Expanded (unreal) limit permits offsets above 64KiB within the cached limit.
    /// Spec: SDM Vol. 3 §3.4.3 (cached limit survives real-address mode).
    #[test]
    fn unreal_expanded_limit_allows_high_offset() {
        let mut seg = SegmentReg::real_mode(0);
        seg.limit = 0xFFFF_FFFF;
        assert!(offset_within_limit(&seg, 0x1_0000, 1));
        assert_eq!(checked_linear_addr(&seg, 0x1_0000, 1).unwrap(), 0x1_0000);
        // Still fault past a smaller expanded limit.
        seg.limit = 0x1_7FFF;
        assert!(offset_within_limit(&seg, 0x1_0000, 1));
        assert!(!offset_within_limit(&seg, 0x1_8000, 1));
    }
}
