//! Host-side INT 15h memory-size stubs (AH=88h / AX=E801h).
//!
//! FreeDOS and early Linux probes often ask the BIOS for extended memory via
//! classic INT 15h before trusting e820. This host dispatcher answers those
//! two calls from the same CMOS memory-size bytes
//! [`Machine::sync_firmware_configuration`] already publishes — honest caps,
//! no invented >4 GiB path through these services.
//!
//! Spec: Ralf Brown's Interrupt List —
//! - INT 15h AH=88h "GET EXTENDED MEMORY SIZE"
//! - INT 15h AX=E801h "GET MEMORY SIZE FOR >64M CONFIGURATIONS"
//!
//! Guest `INT 15h` still needs a real IVT handler (SeaBIOS) or an explicit
//! call into this API. **Not** a full BIOS memory map; **not** e820.

use crate::Machine;
use devices::{CMOS_EXT_MEMORY_MAX_KB, CMOS_MEM_ABOVE_16M_MAX_BLOCKS};
use x86_core::CpuState;

/// AH=88h — get extended memory size (contiguous KB above 1 MB).
pub const INT15_AH_EXT_MEM: u8 = 0x88;

/// AX=E801h — get memory size for >64 MB configurations.
pub const INT15_AX_E801: u16 = 0xE801;

/// RFLAGS Carry Flag — set on unsupported / error returns.
const RFLAGS_CF: u64 = 1 << 0;

impl Machine {
    /// Host-side INT 15h AH=88h / AX=E801h memory-size subset.
    ///
    /// Spec: RBIL INT 15h —
    /// - `AH=88h`: `AX` = contiguous KB starting at 1 MB from CMOS `17h`/`18h`
    ///   (same clamp as [`devices::CmosRtc::set_memory_size`], max `3C00h` =
    ///   15 MB). CF clear.
    /// - `AX=E801h`: `AX`/`CX` = 1 MB–16 MB window in KB (CMOS `17h`/`18h`);
    ///   `BX`/`DX` = above 16 MB in 64 KB blocks (CMOS `34h`/`35h`). CF clear.
    /// - Any other AH/AX leaves registers unchanged and sets CF.
    pub fn service_int15_memory_size(&mut self) {
        let ax = self.cpu.ax();
        if ax == INT15_AX_E801 {
            self.int15_e801();
            return;
        }
        if self.cpu.ah() == INT15_AH_EXT_MEM {
            self.int15_ah88();
            return;
        }
        self.int15_mem_set_cf(true);
    }

    /// Contiguous extended memory in KB as AH=88h / CMOS `17h`/`18h` report it.
    pub fn int15_extended_memory_kb(&self) -> u16 {
        self.cmos.extended_memory_kb().min(CMOS_EXT_MEMORY_MAX_KB)
    }

    /// Above-16 MB memory in 64 KB blocks as E801h BX/DX / CMOS `34h`/`35h`.
    pub fn int15_memory_above_16m_blocks(&self) -> u16 {
        self.cmos
            .memory_above_16m_blocks()
            .min(CMOS_MEM_ABOVE_16M_MAX_BLOCKS)
    }

    fn int15_ah88(&mut self) {
        let kb = self.int15_extended_memory_kb();
        self.cpu.set_ax(kb);
        self.int15_mem_set_cf(false);
    }

    fn int15_e801(&mut self) {
        let between = self.int15_extended_memory_kb();
        let above = self.int15_memory_above_16m_blocks();
        self.cpu.set_ax(between);
        self.cpu.set_gpr_u16(CpuState::RBX, above);
        self.cpu.set_gpr_u16(CpuState::RCX, between);
        self.cpu.set_gpr_u16(CpuState::RDX, above);
        self.int15_mem_set_cf(false);
    }

    fn int15_mem_set_cf(&mut self, set: bool) {
        if set {
            self.cpu.rflags |= RFLAGS_CF;
        } else {
            self.cpu.rflags &= !RFLAGS_CF;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use devices::CMOS_EXT_MEMORY_MAX_KB;

    fn cf(cpu: &x86_core::CpuState) -> bool {
        cpu.rflags & RFLAGS_CF != 0
    }

    #[test]
    fn ah88_reports_cmos_extended_kb() {
        let mut m = Machine::new(8 * 1024 * 1024);
        assert_eq!(m.int15_extended_memory_kb(), 7 * 1024);
        m.cpu.set_ah(INT15_AH_EXT_MEM);
        m.cpu.rflags |= RFLAGS_CF;
        m.service_int15_memory_size();
        assert!(!cf(&m.cpu));
        assert_eq!(m.cpu.ax(), 7 * 1024);
    }

    #[test]
    fn ah88_caps_at_fifteen_mb() {
        let mut m = Machine::new(64 * 1024 * 1024);
        assert_eq!(m.int15_extended_memory_kb(), CMOS_EXT_MEMORY_MAX_KB);
        m.cpu.set_ah(INT15_AH_EXT_MEM);
        m.service_int15_memory_size();
        assert!(!cf(&m.cpu));
        assert_eq!(m.cpu.ax(), CMOS_EXT_MEMORY_MAX_KB);
    }

    #[test]
    fn e801_reports_cmos_split() {
        let mut m = Machine::new(32 * 1024 * 1024);
        assert_eq!(m.int15_extended_memory_kb(), CMOS_EXT_MEMORY_MAX_KB);
        assert_eq!(m.int15_memory_above_16m_blocks(), 256);
        m.cpu.set_ax(INT15_AX_E801);
        m.cpu.rflags |= RFLAGS_CF;
        m.service_int15_memory_size();
        assert!(!cf(&m.cpu));
        assert_eq!(m.cpu.ax(), CMOS_EXT_MEMORY_MAX_KB);
        assert_eq!(m.cpu.gpr_u16(CpuState::RCX), CMOS_EXT_MEMORY_MAX_KB);
        assert_eq!(m.cpu.gpr_u16(CpuState::RBX), 256);
        assert_eq!(m.cpu.gpr_u16(CpuState::RDX), 256);
    }

    #[test]
    fn ah88_zero_when_ram_under_1m() {
        let mut m = Machine::new(512 * 1024);
        m.cpu.set_ah(INT15_AH_EXT_MEM);
        m.service_int15_memory_size();
        assert!(!cf(&m.cpu));
        assert_eq!(m.cpu.ax(), 0);
    }

    #[test]
    fn unsupported_ah_sets_cf() {
        let mut m = Machine::new(4 * 1024 * 1024);
        m.cpu.set_ax(0x1234);
        m.cpu.rflags &= !RFLAGS_CF;
        m.service_int15_memory_size();
        assert!(cf(&m.cpu));
        assert_eq!(m.cpu.ax(), 0x1234);
    }

    #[test]
    fn stubs_read_live_cmos_not_private_ram_copy() {
        let mut m = Machine::new(32 * 1024 * 1024);
        m.cmos.set_memory_size(4 * 1024 * 1024);
        m.cpu.set_ah(INT15_AH_EXT_MEM);
        m.service_int15_memory_size();
        assert_eq!(m.cpu.ax(), 3 * 1024);
        m.cpu.set_ax(INT15_AX_E801);
        m.service_int15_memory_size();
        assert_eq!(m.cpu.ax(), 3 * 1024);
        assert_eq!(m.cpu.gpr_u16(CpuState::RBX), 0);
    }
}
