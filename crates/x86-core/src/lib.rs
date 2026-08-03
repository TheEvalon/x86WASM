//! CPU architectural state (64-bit-capable from day one).
//!
//! Reset values follow Intel SDM Vol. 3 processor initialization (real-mode
//! bring-up defaults used by the Milestone 1 lab).

#![forbid(unsafe_code)]

/// Segment register selector + hidden descriptor cache.
///
/// Spec: Intel SDM Vol. 3 §3.4.2–§3.4.3 (visible selector; cached base/limit/AR).
/// `limit` is the effective inclusive max offset (G-bit already applied if set by a
/// prior protected-mode load). Unreal/"big real" keeps an expanded data-segment
/// limit after returning to real-address mode (SeaBIOS flat 4GiB DS/ES/…).
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SegmentReg {
    pub selector: u16,
    pub base: u64,
    pub limit: u32,
    /// Access rights / attributes (opaque beyond present defaults).
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

    /// Real-address mode data/stack segment load: update selector and base only.
    ///
    /// Cached `limit` and `flags` are retained so an expanded unreal-mode limit
    /// survives `MOV/POP/LDS/LES` of DS/ES/SS/FS/GS. Spec: SDM Vol. 3 §3.4.2
    /// (`base = selector << 4`); §3.4.3 (descriptor cache).
    pub fn load_real_mode_selector(&mut self, selector: u16) {
        self.selector = selector;
        self.base = (selector as u64) << 4;
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
    /// Latched maskable external IRQ vector (PIC stub for later).
    ///
    /// Not guest-architectural beyond interrupt delivery. Recognized at
    /// interpreter poll points when `IF=1` (currently: between REP string
    /// iterations). Use [`Self::request_interrupt`]. Full 8259 is out of scope.
    #[cfg_attr(feature = "serde", serde(default))]
    pub pending_irq: Option<u8>,
    /// Latched non-maskable interrupt request (platform `#NMI` pin stub).
    ///
    /// Delivered at interpreter poll points as IVT vector 2; **not** gated by
    /// `RFLAGS.IF`. Platform CMOS `0x70` bit7 masking is enforced by the machine
    /// before calling [`Self::request_nmi`]. No SMRAM/SMI nesting in this stub.
    #[cfg_attr(feature = "serde", serde(default))]
    pub pending_nmi: bool,
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
            pending_irq: None,
            pending_nmi: false,
        }
    }

    /// Queue a maskable external interrupt vector (test / future PIC hook).
    ///
    /// Delivered at architected poll points when `RFLAGS.IF=1`. Does not
    /// implement 8259 priority, IRR/ISR, or spurious IRQ semantics.
    pub fn request_interrupt(&mut self, vector: u8) {
        self.pending_irq = Some(vector);
    }

    /// Latch a platform `#NMI` request (IVT vector 2).
    ///
    /// Spec: Intel SDM Vol. 3 §6.3.3 / §6.7 — NMI is not maskable by `IF`.
    /// Callers that model IBM PC/AT CMOS index bit7 must drop the request
    /// before calling this when NMI is masked.
    pub fn request_nmi(&mut self) {
        self.pending_nmi = true;
    }

    pub fn gpr_u16(&self, idx: usize) -> u16 {
        self.gpr[idx] as u16
    }

    pub fn set_gpr_u16(&mut self, idx: usize, val: u16) {
        let old = self.gpr[idx];
        self.gpr[idx] = (old & !0xFFFF) | u64::from(val);
    }

    /// 32-bit GPR view (EAX…EDI). Spec: Intel SDM Vol. 1 §3.4.1 / §3.6
    /// (operand-size attribute selects 16 vs 32 in real-address mode).
    pub fn gpr_u32(&self, idx: usize) -> u32 {
        self.gpr[idx] as u32
    }

    /// Write 32-bit GPR; preserves bits 63:32 of the u64 storage
    /// (same pattern as `set_gpr_u16`; long-mode zero-extend is a later slice).
    pub fn set_gpr_u32(&mut self, idx: usize, val: u32) {
        let old = self.gpr[idx];
        self.gpr[idx] = (old & !0xFFFF_FFFF) | u64::from(val);
    }

    pub fn gpr_u8_low(&self, idx: usize) -> u8 {
        self.gpr[idx] as u8
    }

    pub fn set_gpr_u8_low(&mut self, idx: usize, val: u8) {
        let old = self.gpr[idx];
        self.gpr[idx] = (old & !0xFF) | u64::from(val);
    }

    /// Legacy 8-bit GPR view for ModR/M.reg / ModR/M.rm / opcodes B0–B7 (no REX).
    ///
    /// Indices 0–3 → AL/CL/DL/BL; 4–7 → AH/CH/DH/BH.
    /// Spec: Intel SDM Vol. 1 §3.4.1.1; Vol. 2 Appendix B (ModR/M byte).
    pub fn gpr_u8(&self, idx: usize) -> u8 {
        debug_assert!(idx < 8, "legacy byte GPR index must be 0..7");
        if idx < 4 {
            self.gpr_u8_low(idx)
        } else {
            (self.gpr[idx - 4] >> 8) as u8
        }
    }

    /// Write legacy 8-bit GPR (AL..BH). Preserves the sibling low/high byte and upper bits.
    /// Spec: Intel SDM Vol. 1 §3.4.1.1; Vol. 2 Appendix B (ModR/M byte).
    pub fn set_gpr_u8(&mut self, idx: usize, val: u8) {
        debug_assert!(idx < 8, "legacy byte GPR index must be 0..7");
        if idx < 4 {
            self.set_gpr_u8_low(idx, val);
        } else {
            let g = idx - 4;
            let old = self.gpr[g];
            self.gpr[g] = (old & !0xFF00) | (u64::from(val) << 8);
        }
    }

    pub fn al(&self) -> u8 {
        self.gpr_u8_low(Self::RAX)
    }

    pub fn set_al(&mut self, v: u8) {
        self.set_gpr_u8_low(Self::RAX, v);
    }

    pub fn ah(&self) -> u8 {
        self.gpr_u8(4)
    }

    pub fn set_ah(&mut self, v: u8) {
        self.set_gpr_u8(4, v);
    }

    pub fn ax(&self) -> u16 {
        self.gpr_u16(Self::RAX)
    }

    pub fn set_ax(&mut self, v: u16) {
        self.set_gpr_u16(Self::RAX, v);
    }

    pub fn eax(&self) -> u32 {
        self.gpr_u32(Self::RAX)
    }

    pub fn set_eax(&mut self, v: u32) {
        self.set_gpr_u32(Self::RAX, v);
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

    /// Direction flag (RFLAGS.DF, bit 10) — SDM Vol. 1 §3.4.3.
    pub fn direction_flag(&self) -> bool {
        self.rflags & (1 << 10) != 0
    }

    pub fn set_direction_flag(&mut self, on: bool) {
        if on {
            self.rflags |= 1 << 10;
        } else {
            self.rflags &= !(1 << 10);
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
        if self.pending_irq != other.pending_irq {
            out.push("pending_irq");
        }
        if self.pending_nmi != other.pending_nmi {
            out.push("pending_nmi");
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

    /// 32-bit GPR helpers (opsize attribute / 0x66). Spec: SDM Vol. 1 §3.4.1, §3.6.
    #[test]
    fn gpr_u32_helpers_preserve_upper_dword() {
        let mut cpu = CpuState::reset();
        cpu.gpr[CpuState::RAX] = 0x1111_2222_3333_4444;
        assert_eq!(cpu.gpr_u32(CpuState::RAX), 0x3333_4444);
        cpu.set_gpr_u32(CpuState::RAX, 0xABCD_EF01);
        assert_eq!(cpu.gpr[CpuState::RAX], 0x1111_2222_ABCD_EF01);
        cpu.set_eax(0x1234_5678);
        assert_eq!(cpu.eax(), 0x1234_5678);
        assert_eq!(cpu.ax(), 0x5678);
        assert_eq!(cpu.gpr[CpuState::RAX], 0x1111_2222_1234_5678);
    }

    /// Legacy ModR/M byte regs 4–7 are AH/CH/DH/BH (SDM Vol. 1 §3.4.1.1).
    #[test]
    fn gpr_u8_legacy_high_bytes() {
        let mut cpu = CpuState::reset();
        cpu.gpr[CpuState::RAX] = 0x1111_2222_3333_4455;
        cpu.gpr[CpuState::RCX] = 0xAAAA_BBBB_CCCC_DDEE;
        cpu.gpr[CpuState::RDX] = 0x0000_0000_0000_1122;
        cpu.gpr[CpuState::RBX] = 0x0000_0000_0000_3344;

        assert_eq!(cpu.gpr_u8(0), 0x55); // AL
        assert_eq!(cpu.gpr_u8(4), 0x44); // AH
        assert_eq!(cpu.gpr_u8(1), 0xEE); // CL
        assert_eq!(cpu.gpr_u8(5), 0xDD); // CH
        assert_eq!(cpu.gpr_u8(2), 0x22); // DL
        assert_eq!(cpu.gpr_u8(6), 0x11); // DH
        assert_eq!(cpu.gpr_u8(3), 0x44); // BL
        assert_eq!(cpu.gpr_u8(7), 0x33); // BH

        cpu.set_gpr_u8(4, 0xAB); // AH
        assert_eq!(cpu.gpr[CpuState::RAX], 0x1111_2222_3333_AB55);
        cpu.set_gpr_u8(5, 0x10); // CH
        assert_eq!(cpu.gpr[CpuState::RCX], 0xAAAA_BBBB_CCCC_10EE);
        cpu.set_ah(0x99);
        assert_eq!(cpu.ah(), 0x99);
        assert_eq!(cpu.al(), 0x55);
    }

    #[test]
    fn diff_detects_rip() {
        let a = CpuState::reset();
        let mut b = a.clone();
        b.rip = 0x1234;
        assert_eq!(a.diff(&b), vec!["rip"]);
    }
}
