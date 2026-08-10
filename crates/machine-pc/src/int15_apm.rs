//! Host-side APM BIOS 1.2 INT 15h AH=53h stub (installation / connect subset).
//!
//! FreeDOS and some firmware probe APM via `INT 15h, AH=53h` before using
//! power services. This host dispatcher answers the installation check and a
//! real-mode connect/disconnect subset so guests see a coherent "APM present"
//! reply. It is **not** real System Management Mode, does not enter SMRAM, and
//! does not change architectural power state.
//!
//! Spec: Advanced Power Management (APM) BIOS Interface Specification 1.2 —
//! §4 INT 15h AH=53h functions (Installation Check `AL=00h`, Connect Real-Mode
//! `AL=01h`, Disconnect `AL=04h`); Ralf Brown's Interrupt List INT 15h AH=53h.

use crate::{Machine, MachineError};
use x86_core::CpuState;

/// INT 15h vector.
pub const INT15_VECTOR: u8 = 0x15;

/// AH=53h — APM BIOS function group.
pub const INT15_AH_APM: u8 = 0x53;

/// AL=00h — APM installation check.
pub const APM_AL_INSTALLATION_CHECK: u8 = 0x00;
/// AL=01h — connect real-mode interface.
pub const APM_AL_CONNECT_REAL: u8 = 0x01;
/// AL=02h — connect 16-bit protected-mode interface (unsupported here).
pub const APM_AL_CONNECT_16: u8 = 0x02;
/// AL=03h — connect 32-bit protected-mode interface (unsupported here).
pub const APM_AL_CONNECT_32: u8 = 0x03;
/// AL=04h — disconnect interface.
pub const APM_AL_DISCONNECT: u8 = 0x04;

/// APM BIOS major version reported by the installation check (1.x).
pub const APM_VERSION_MAJOR: u8 = 1;
/// APM BIOS minor version reported by the installation check (.2 → 1.2).
pub const APM_VERSION_MINOR: u8 = 2;

/// APM error: interface connection already in use / engaged.
pub const APM_ERR_INTERFACE_CONNECTED: u8 = 0x02;
/// APM error: interface not connected.
pub const APM_ERR_INTERFACE_NOT_CONNECTED: u8 = 0x03;
/// APM error: unsupported function / interface type.
pub const APM_ERR_UNSUPPORTED: u8 = 0x86;

/// Legacy ModR/M byte index for BH (high byte of BX).
const GPR_BH: usize = 7;
/// Legacy ModR/M byte index for BL (low byte of BX).
const GPR_BL: usize = 3;

/// RFLAGS Carry Flag — set on APM error returns.
const RFLAGS_CF: u64 = 1 << 0;

impl Machine {
    /// Host-side INT 15h AH=53h APM subset using current CPU registers.
    ///
    /// Spec: APM BIOS 1.2 §4 —
    /// - `AX=5300h`, `BX=0000h`: installation check → `AH=00`, `BH/BL` = 1.2,
    ///   `CX` flags clear (PM interfaces unsupported), CF clear.
    /// - `AX=5301h`, `BX=0000h`: connect real-mode → success once; second call
    ///   returns `AH=02h` (already connected) with CF set.
    /// - `AX=5304h`: disconnect → success when connected; else `AH=03h`.
    /// - `AX=5302h` / `5303h`: protected-mode connect → `AH=86h` unsupported
    ///   (honest: no PM entry stubs / no SMM).
    ///
    /// Other AH values leave registers unchanged and set CF (not APM).
    ///
    /// Guest `INT 15h` still needs a real IVT handler (SeaBIOS) or an explicit
    /// call into this API. **No real SMM.**
    pub fn service_int15_apm(&mut self) {
        if self.cpu.ah() != INT15_AH_APM {
            self.apm_set_cf(true);
            return;
        }
        match self.cpu.al() {
            APM_AL_INSTALLATION_CHECK => self.apm_installation_check(),
            APM_AL_CONNECT_REAL => self.apm_connect_real(),
            APM_AL_CONNECT_16 | APM_AL_CONNECT_32 => self.apm_fail(APM_ERR_UNSUPPORTED),
            APM_AL_DISCONNECT => self.apm_disconnect(),
            _ => self.apm_fail(APM_ERR_UNSUPPORTED),
        }
    }

    /// Install a real-mode IVT entry for vector `0x15` that points at `handler`.
    ///
    /// Does **not** install an APM BIOS body — only the far pointer. Host
    /// harnesses that want APM services must call [`Self::service_int15_apm`].
    pub fn install_int15_ivt_pointer(
        &mut self,
        handler_seg: u16,
        handler_off: u16,
    ) -> Result<(), MachineError> {
        let base = u64::from(INT15_VECTOR) * 4;
        self.mem
            .write_u8(base, (handler_off & 0xFF) as u8)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        self.mem
            .write_u8(base + 1, (handler_off >> 8) as u8)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        self.mem
            .write_u8(base + 2, (handler_seg & 0xFF) as u8)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        self.mem
            .write_u8(base + 3, (handler_seg >> 8) as u8)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        Ok(())
    }

    /// Whether the host APM real-mode interface is connected.
    pub fn apm_bios_connected(&self) -> bool {
        self.apm_rm_connected
    }

    fn apm_installation_check(&mut self) {
        // Spec: APM 1.2 — BX must be 0000h (system BIOS device ID).
        if self.cpu.gpr_u16(CpuState::RBX) != 0 {
            self.apm_fail(APM_ERR_UNSUPPORTED);
            return;
        }
        self.cpu.set_ah(0x00);
        // BH = major, BL = minor (APM 1.2).
        self.cpu.set_gpr_u8(GPR_BH, APM_VERSION_MAJOR);
        self.cpu.set_gpr_u8(GPR_BL, APM_VERSION_MINOR);
        // CX bit0 = 16-bit protected mode interface supported — leave clear;
        // only real-mode connect is implemented in this stub.
        self.cpu.set_gpr_u16(CpuState::RCX, 0x0000);
        self.apm_set_cf(false);
    }

    fn apm_connect_real(&mut self) {
        if self.cpu.gpr_u16(CpuState::RBX) != 0 {
            self.apm_fail(APM_ERR_UNSUPPORTED);
            return;
        }
        if self.apm_rm_connected {
            self.apm_fail(APM_ERR_INTERFACE_CONNECTED);
            return;
        }
        self.apm_rm_connected = true;
        self.cpu.set_ah(0x00);
        self.apm_set_cf(false);
    }

    fn apm_disconnect(&mut self) {
        if !self.apm_rm_connected {
            self.apm_fail(APM_ERR_INTERFACE_NOT_CONNECTED);
            return;
        }
        self.apm_rm_connected = false;
        self.cpu.set_ah(0x00);
        self.apm_set_cf(false);
    }

    fn apm_fail(&mut self, code: u8) {
        self.cpu.set_ah(code);
        self.apm_set_cf(true);
    }

    fn apm_set_cf(&mut self, set: bool) {
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

    fn cf(cpu: &CpuState) -> bool {
        cpu.rflags & RFLAGS_CF != 0
    }

    /// Spec: APM BIOS 1.2 §4.1 — Installation Check AX=5300h, BX=0000h.
    #[test]
    fn apm_installation_check_reports_1_2() {
        let mut m = Machine::new(64 * 1024);
        m.cpu.set_ax(0x5300);
        m.cpu.set_gpr_u16(CpuState::RBX, 0);
        m.service_int15_apm();
        assert!(!cf(&m.cpu));
        assert_eq!(m.cpu.ah(), 0x00);
        assert_eq!(m.cpu.gpr_u8(GPR_BH), APM_VERSION_MAJOR);
        assert_eq!(m.cpu.gpr_u8(GPR_BL), APM_VERSION_MINOR);
        assert_eq!(m.cpu.gpr_u16(CpuState::RCX), 0);
    }

    /// Spec: APM BIOS 1.2 — Connect Real-Mode AX=5301h succeeds once.
    #[test]
    fn apm_connect_real_then_already_connected() {
        let mut m = Machine::new(64 * 1024);
        m.cpu.set_ax(0x5301);
        m.cpu.set_gpr_u16(CpuState::RBX, 0);
        m.service_int15_apm();
        assert!(!cf(&m.cpu));
        assert!(m.apm_bios_connected());

        m.cpu.set_ax(0x5301);
        m.cpu.set_gpr_u16(CpuState::RBX, 0);
        m.service_int15_apm();
        assert!(cf(&m.cpu));
        assert_eq!(m.cpu.ah(), APM_ERR_INTERFACE_CONNECTED);
    }

    /// Spec: APM BIOS 1.2 — Disconnect AX=5304h; disconnect when idle → error.
    #[test]
    fn apm_disconnect_round_trip() {
        let mut m = Machine::new(64 * 1024);
        m.cpu.set_ax(0x5304);
        m.service_int15_apm();
        assert!(cf(&m.cpu));
        assert_eq!(m.cpu.ah(), APM_ERR_INTERFACE_NOT_CONNECTED);

        m.cpu.set_ax(0x5301);
        m.cpu.set_gpr_u16(CpuState::RBX, 0);
        m.service_int15_apm();
        m.cpu.set_ax(0x5304);
        m.service_int15_apm();
        assert!(!cf(&m.cpu));
        assert!(!m.apm_bios_connected());
    }

    /// Spec honesty: protected-mode connect is unsupported (no SMM / PM entry).
    #[test]
    fn apm_pm_connect_unsupported() {
        let mut m = Machine::new(64 * 1024);
        m.cpu.set_ax(0x5303);
        m.cpu.set_gpr_u16(CpuState::RBX, 0);
        m.service_int15_apm();
        assert!(cf(&m.cpu));
        assert_eq!(m.cpu.ah(), APM_ERR_UNSUPPORTED);
    }

    #[test]
    fn apm_ivt_pointer_install_and_reset_clears_connect() {
        let mut m = Machine::new(64 * 1024);
        m.install_int15_ivt_pointer(0xF000, 0xE000).unwrap();
        assert_eq!(m.mem.read_u8(0x15 * 4).unwrap(), 0x00);
        assert_eq!(m.mem.read_u8(0x15 * 4 + 1).unwrap(), 0xE0);
        assert_eq!(m.mem.read_u8(0x15 * 4 + 2).unwrap(), 0x00);
        assert_eq!(m.mem.read_u8(0x15 * 4 + 3).unwrap(), 0xF0);

        m.cpu.set_ax(0x5301);
        m.cpu.set_gpr_u16(CpuState::RBX, 0);
        m.service_int15_apm();
        assert!(m.apm_bios_connected());
        m.reset();
        assert!(!m.apm_bios_connected());
    }
}
