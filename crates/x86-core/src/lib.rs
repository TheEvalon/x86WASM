//! CPU architectural state (64-bit-capable from day one).
//!
//! Reset values follow Intel SDM Vol. 3 processor initialization (real-mode
//! bring-up defaults used by the Milestone 1 lab).

#![forbid(unsafe_code)]

/// Segment register selector + hidden descriptor cache.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SegmentReg {
    pub selector: u16,
    pub base: u64,
    pub limit: u32,
    /// Access rights / attributes (opaque to M1 beyond present defaults).
    pub flags: u16,
}

impl SegmentReg {
    pub const fn flat_real(selector: u16, base: u64) -> Self {
        Self {
            selector,
            base,
            limit: 0xFFFF,
            flags: 0x0093,
        }
    }

    /// Real-address mode segment: `base = selector << 4` (Intel SDM Vol. 3 §3.4.2).
    pub const fn real_mode(selector: u16) -> Self {
        Self::flat_real(selector, (selector as u64) << 4)
    }

    /// Real-address mode code segment (same base rule; code access rights).
    pub const fn real_mode_code(selector: u16) -> Self {
        Self {
            selector,
            base: (selector as u64) << 4,
            limit: 0xFFFF,
            flags: 0x009B,
        }
    }
}

/// GDTR / IDTR.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DescriptorTableReg {
    pub base: u64,
    pub limit: u16,
}

/// Full CPU state. GPRs are `u64` even while only real mode runs.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CpuState {
    /// RAX..R15
    pub gpr: [u64; 16],
    pub rip: u64,
    pub rflags: u64,
    pub es: SegmentReg,
    pub cs: SegmentReg,
    pub ss: SegmentReg,
    pub ds: SegmentReg,
    pub fs: SegmentReg,
    pub gs: SegmentReg,
    pub gdtr: DescriptorTableReg,
    pub idtr: DescriptorTableReg,
    pub ldtr: SegmentReg,
    pub tr: SegmentReg,
    pub cr0: u64,
    pub cr2: u64,
    pub cr3: u64,
    pub cr4: u64,
    pub cr8: u64,
    pub efer: u64,
    pub halted: bool,
}

impl Default for CpuState {
    fn default() -> Self {
        Self::reset()
    }
}

impl CpuState {
    pub const RAX: usize = 0;
    pub const RCX: usize = 1;
    pub const RDX: usize = 2;
    pub const RBX: usize = 3;
    pub const RSP: usize = 4;
    pub const RBP: usize = 5;
    pub const RSI: usize = 6;
    pub const RDI: usize = 7;

    /// Intel-style reset for the M1 lab (see `docs/cpu-profile-core2.md`).
    pub fn reset() -> Self {
        let null_seg = SegmentReg::flat_real(0, 0);
        Self {
            gpr: [0; 16],
            rip: 0x0000_FFF0,
            rflags: 0x0000_0002,
            es: null_seg.clone(),
            cs: SegmentReg {
                selector: 0xF000,
                base: 0xFFFF_0000,
                limit: 0xFFFF,
                flags: 0x009B,
            },
            ss: null_seg.clone(),
            ds: null_seg.clone(),
            fs: null_seg.clone(),
            gs: null_seg.clone(),
            gdtr: DescriptorTableReg::default(),
            idtr: DescriptorTableReg::default(),
            ldtr: null_seg.clone(),
            tr: null_seg,
            cr0: 0x6000_0010,
            cr2: 0,
            cr3: 0,
            cr4: 0,
            cr8: 0,
            efer: 0,
            halted: false,
        }
    }

    pub fn gpr_u16(&self, idx: usize) -> u16 {
        self.gpr[idx] as u16
    }

    pub fn set_gpr_u16(&mut self, idx: usize, val: u16) {
        let old = self.gpr[idx];
        self.gpr[idx] = (old & !0xFFFF) | u64::from(val);
    }

    pub fn gpr_u8_low(&self, idx: usize) -> u8 {
        self.gpr[idx] as u8
    }

    pub fn set_gpr_u8_low(&mut self, idx: usize, val: u8) {
        let old = self.gpr[idx];
        self.gpr[idx] = (old & !0xFF) | u64::from(val);
    }

    pub fn al(&self) -> u8 {
        self.gpr_u8_low(Self::RAX)
    }

    pub fn set_al(&mut self, v: u8) {
        self.set_gpr_u8_low(Self::RAX, v);
    }

    pub fn ax(&self) -> u16 {
        self.gpr_u16(Self::RAX)
    }

    pub fn set_ax(&mut self, v: u16) {
        self.set_gpr_u16(Self::RAX, v);
    }

    pub fn ip16(&self) -> u16 {
        self.rip as u16
    }

    pub fn set_ip16(&mut self, ip: u16) {
        self.rip = (self.rip & !0xFFFF) | u64::from(ip);
    }

    pub fn interrupt_flag(&self) -> bool {
        self.rflags & (1 << 9) != 0
    }

    pub fn set_interrupt_flag(&mut self, on: bool) {
        if on {
            self.rflags |= 1 << 9;
        } else {
            self.rflags &= !(1 << 9);
        }
    }

    pub fn set_zf(&mut self, on: bool) {
        if on {
            self.rflags |= 1 << 6;
        } else {
            self.rflags &= !(1 << 6);
        }
    }

    pub fn set_sf(&mut self, on: bool) {
        if on {
            self.rflags |= 1 << 7;
        } else {
            self.rflags &= !(1 << 7);
        }
    }

    pub fn set_cf(&mut self, on: bool) {
        if on {
            self.rflags |= 1;
        } else {
            self.rflags &= !1;
        }
    }

    pub fn set_of(&mut self, on: bool) {
        if on {
            self.rflags |= 1 << 11;
        } else {
            self.rflags &= !(1 << 11);
        }
    }

    pub fn set_pf(&mut self, on: bool) {
        if on {
            self.rflags |= 1 << 2;
        } else {
            self.rflags &= !(1 << 2);
        }
    }

    pub fn set_af(&mut self, on: bool) {
        if on {
            self.rflags |= 1 << 4;
        } else {
            self.rflags &= !(1 << 4);
        }
    }

    /// Compare architectural state for tests / lockstep (M1: full eq).
    pub fn diff(&self, other: &Self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.gpr != other.gpr {
            out.push("gpr");
        }
        if self.rip != other.rip {
            out.push("rip");
        }
        if self.rflags != other.rflags {
            out.push("rflags");
        }
        if self.cs != other.cs {
            out.push("cs");
        }
        if self.ds != other.ds {
            out.push("ds");
        }
        if self.ss != other.ss {
            out.push("ss");
        }
        if self.es != other.es {
            out.push("es");
        }
        if self.fs != other.fs {
            out.push("fs");
        }
        if self.gs != other.gs {
            out.push("gs");
        }
        if self.cr0 != other.cr0 {
            out.push("cr0");
        }
        if self.halted != other.halted {
            out.push("halted");
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_mode_segment_base_is_selector_shifted() {
        let s = SegmentReg::real_mode(0x1234);
        assert_eq!(s.base, 0x1234u64 << 4);
        let c = SegmentReg::real_mode_code(0xF000);
        assert_eq!(c.base, 0xF000u64 << 4);
        assert_eq!(c.flags, 0x009B);
    }

    #[test]
    fn reset_vector_matches_docs() {
        let cpu = CpuState::reset();
        assert_eq!(cpu.rip, 0xFFF0);
        assert_eq!(cpu.cs.selector, 0xF000);
        assert_eq!(cpu.cs.base, 0xFFFF_0000);
        assert_eq!(cpu.rflags, 0x2);
        assert_eq!(cpu.cr0, 0x6000_0010);
        assert!(!cpu.halted);
    }

    #[test]
    fn gpr_width_helpers_preserve_high_bits() {
        let mut cpu = CpuState::reset();
        cpu.gpr[CpuState::RAX] = 0x1111_2222_3333_4444;
        cpu.set_gpr_u16(CpuState::RAX, 0xABCD);
        assert_eq!(cpu.gpr[CpuState::RAX], 0x1111_2222_3333_ABCD);
        cpu.set_al(0x55);
        assert_eq!(cpu.gpr[CpuState::RAX], 0x1111_2222_3333_AB55);
    }

    #[test]
    fn diff_detects_rip() {
        let a = CpuState::reset();
        let mut b = a.clone();
        b.rip = 0x1234;
        assert_eq!(a.diff(&b), vec!["rip"]);
    }
}
