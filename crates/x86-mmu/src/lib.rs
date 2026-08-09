//! Address translation helpers.
//!
//! Segmentation: real-mode style `segment.base + offset` (CS.base is the Intel
//! reset base `0xFFFF_0000`), with limit checks against the cached effective
//! limit (SDM Vol. 3 §5.3).
//!
//! Paging: [`paging`] is a 32-bit paging translation engine (SDM Vol. 3
//! Chapter 4). Round 4 wired it to the interpreter, so a guest that sets
//! `CR0.PG` runs under it; engine behavior is in `docs/mmu-r3-32bit-paging.md`
//! and the integration in `docs/cpu-r4-paging-integration.md`.

#![forbid(unsafe_code)]

pub mod paging;

use x86_core::{CpuState, SegmentReg};

/// Width of the linear address space outside 64-bit mode.
///
/// Spec: Intel SDM Vol. 3 §3.3.1 — real-address mode and protected mode address
/// a 4-GiB linear address space, so `base + offset` is formed modulo 2^32.
const LINEAR_ADDRESS_MASK_32: u64 = 0xFFFF_FFFF;

/// Linear address of a segmented reference outside 64-bit mode.
///
/// The sum wraps at 4 GiB rather than carrying into bit 32: with
/// `CS.base = 0xFFFF0000`, offset `0x1_00FF000` addresses `0x000FF000` — the top
/// of the `0xF0000`–`0xFFFFF` BIOS segment — not an address above 4 GiB.
///
/// Spec: Intel SDM Vol. 3 §3.3.1 (logical → linear, 4-GiB linear address space);
/// §3.4.3 (cached base).
pub fn linear_addr(seg: &SegmentReg, offset: u64) -> u64 {
    seg.base.wrapping_add(offset) & LINEAR_ADDRESS_MASK_32
}

/// Linear address of a segmented reference in 64-bit mode, where the linear
/// address space is the full 64 bits and the sum is not truncated.
///
/// Nothing reaches this yet — no mode in this build is 64-bit. It exists so the
/// 32-bit wrap in [`linear_addr`] is not silently inherited when long mode
/// lands.
///
/// Spec: Intel SDM Vol. 3 §3.3.1. Unsupported here: the canonical-address check.
pub fn linear_addr64(seg: &SegmentReg, offset: u64) -> u64 {
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

    /// Outside 64-bit mode the linear address space is 4 GiB, so a segment base
    /// near the top plus a large offset wraps into low memory instead of
    /// carrying into bit 32.
    ///
    /// Spec: Intel SDM Vol. 3 §3.3.1 — "in protected mode ... the processor
    /// maps ... into a linear address space" of 4 GiB; the sum is taken modulo
    /// 2^32. This is the case SeaBIOS hits with `CS.base = 0xFFFF0000`: an
    /// f-segment reference must land in the `0xF0000`–`0xFFFFF` BIOS segment,
    /// not above 4 GiB.
    #[test]
    fn linear_address_wraps_at_4gib_outside_64_bit_mode() {
        let mut seg = SegmentReg::real_mode(0);
        seg.base = 0xFFFF_0000;
        seg.limit = 0xFFFF_FFFF;

        // Exactly at the top: the last byte of the space.
        assert_eq!(linear_addr(&seg, 0xFFFF), 0xFFFF_FFFF);
        // One past the top wraps to zero.
        assert_eq!(linear_addr(&seg, 0x1_0000), 0x0000_0000);
        // The two page touches the POST probe reported above 4 GiB. Untruncated
        // the sums are 0x1_000D_5000 and 0x1_000F_F000; modulo 2^32 they are
        // ordinary low memory, the latter the top of the BIOS segment.
        assert_eq!(linear_addr(&seg, 0x000E_5000), 0x000D_5000);
        assert_eq!(linear_addr(&seg, 0x0010_F000), 0x000F_F000);
        // A base at the very top wraps for any non-zero offset.
        seg.base = 0xFFFF_FFFF;
        assert_eq!(linear_addr(&seg, 1), 0x0000_0000);
        assert_eq!(linear_addr(&seg, 2), 0x0000_0001);
    }

    /// The checked form returns the same wrapped address; the limit check is on
    /// the *offset*, which §5.3 defines independently of the base.
    #[test]
    fn checked_linear_address_wraps_the_same_way() {
        let mut seg = SegmentReg::real_mode(0);
        seg.base = 0xFFFF_0000;
        seg.limit = 0xFFFF_FFFF;

        assert_eq!(checked_linear_addr(&seg, 0x1_0000, 1), Ok(0x0000_0000));
        assert_eq!(checked_linear_addr(&seg, 0x0010_F000, 1), Ok(0x000F_F000));
    }

    /// The segment helpers that mask the offset to 16 bits wrap too, so a
    /// real-mode `SS`/`DS` reference near the top of the space stays in the
    /// 4-GiB space.
    #[test]
    fn segment_helpers_wrap_at_4gib() {
        let mut cpu = CpuState::reset();
        cpu.ss.base = 0xFFFF_FFF0;
        cpu.ds.base = 0xFFFF_FFF0;

        assert_eq!(ss_linear(&cpu, 0x20), 0x0000_0010);
        assert_eq!(ds_linear(&cpu, 0x20), 0x0000_0010);

        // Reset CS still fetches from the top of the space, unchanged.
        let reset = CpuState::reset();
        assert_eq!(cs_linear(&reset), 0xFFFF_FFF0);
    }

    /// 64-bit mode uses the full 64-bit linear address space, so its helper
    /// must not truncate. Nothing reaches this yet; it exists so the 32-bit
    /// wrap is not silently inherited when long mode lands.
    ///
    /// Spec: Intel SDM Vol. 3 §3.3.1 — 64-bit mode linear addresses are 64 bits
    /// wide (subject to a separate canonical-form check, not modelled here).
    #[test]
    fn long_mode_linear_address_is_not_truncated() {
        let mut seg = SegmentReg::real_mode(0);
        seg.base = 0xFFFF_0000;

        assert_eq!(linear_addr64(&seg, 0x1_0000), 0x1_0000_0000);
        assert_eq!(linear_addr64(&seg, 0x0010_F000), 0x1_000F_F000);
        // Below 4 GiB the two agree.
        assert_eq!(linear_addr64(&seg, 0xFFFF), linear_addr(&seg, 0xFFFF));
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
