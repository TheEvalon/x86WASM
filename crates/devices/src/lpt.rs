//! Classic IBM PC parallel (LPT) port register-file stub.
//!
//! Spec:
//! - IBM PC Technical Reference / OSDev Wiki "Parallel Port" — three-byte
//!   register file at a classic base:
//!   - `base+0` Data (R/W)
//!   - `base+1` Status (R; bit7 = Busy, **active low** — 1 means not busy)
//!   - `base+2` Control (R/W)
//! - Classic bases: LPT1 `0x378`, LPT2 `0x278` (LPT3 `0x3BC` not claimed here).
//!
//! SeaBIOS POST probes these ports. With no printer attached, floating status
//! lines typically read high, so Busy# is inactive (bit7 = 1). This stub uses
//! that default so firmware concludes "no printer" without treating the ports
//! as open-bus (`0xFF`).
//!
//! Explicitly out of scope: IRQ7, ECP/EPP, DMA, actual printer handshake.

use crate::PortDevice;

/// Classic LPT1 base (PC/AT).
pub const LPT1_BASE: u16 = 0x378;

/// Classic LPT2 base.
pub const LPT2_BASE: u16 = 0x278;

/// Classic LPT3 base (MDA/printer adapter). Not claimed by this stub.
pub const LPT3_BASE: u16 = 0x3BC;

/// Data register offset from base.
pub const LPT_DATA: u16 = 0;

/// Status register offset from base.
pub const LPT_STATUS: u16 = 1;

/// Control register offset from base.
pub const LPT_CONTROL: u16 = 2;

/// Last owned offset (data/status/control only; ECP/EPP beyond `base+2` out).
pub const LPT_LAST_OFFSET: u16 = 2;

/// Status bit7 — Busy, active low (1 = not busy / inactive).
pub const LPT_STATUS_BUSY_N: u8 = 1 << 7;

/// Control bit0 — Strobe.
pub const LPT_CTRL_STROBE: u8 = 1 << 0;

/// Control bit1 — Auto Line Feed.
pub const LPT_CTRL_AUTOLF: u8 = 1 << 1;

/// Control bit2 — Initialize Printer, **active low** (1 = not asserting /INIT).
pub const LPT_CTRL_INIT_N: u8 = 1 << 2;

/// Control bit3 — Select Input (select printer).
pub const LPT_CTRL_SELECT: u8 = 1 << 3;

/// Control bit4 — IRQ enable (IRQ7). Not delivered in this stub.
pub const LPT_CTRL_IRQ_ENABLE: u8 = 1 << 4;

/// Reset / no-printer status: Busy# inactive (bit7 = 1) plus other floating-high
/// lines commonly observed on an empty port (`Ack#`, `Select`, `Error#`).
///
/// Model choice (not a printed datasheet constant): `0xDF` so bit7 is high and
/// the byte is **not** ISA open-bus `0xFF`. SeaBIOS presence probes that look
/// for a live register file still see a claimed device, while Busy inactive
/// reports no printer traffic.
pub const LPT_STATUS_NO_PRINTER: u8 = 0xDF;

/// Power-on / reset data default.
const LPT_DATA_DEFAULT: u8 = 0x00;

/// Power-on / reset control default: `/INIT` inactive + Select asserted.
///
/// Spec: IBM PC Technical Reference Parallel Printer Adapter control port —
/// bit2 `/INIT` is active-low; bit3 Select Input selects the printer. Classic
/// adapters leave `/INIT` deasserted and Select asserted after reset (`0x0C`).
/// R13 deepens the R6 stub (which left control `0x00`) so firmware RMW of the
/// control register starts from the documented idle state.
pub const LPT_CONTROL_DEFAULT: u8 = LPT_CTRL_INIT_N | LPT_CTRL_SELECT;

/// One classic parallel-port register file (3 I/O bytes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParallelPort {
    base: u16,
    data: u8,
    /// Fixed no-printer status (reads only in this stub).
    status: u8,
    control: u8,
}

impl ParallelPort {
    pub fn new(base: u16) -> Self {
        Self {
            base,
            data: LPT_DATA_DEFAULT,
            status: LPT_STATUS_NO_PRINTER,
            control: LPT_CONTROL_DEFAULT,
        }
    }

    pub fn lpt1() -> Self {
        Self::new(LPT1_BASE)
    }

    pub fn lpt2() -> Self {
        Self::new(LPT2_BASE)
    }

    pub fn reset(&mut self) {
        self.data = LPT_DATA_DEFAULT;
        self.status = LPT_STATUS_NO_PRINTER;
        self.control = LPT_CONTROL_DEFAULT;
    }

    pub fn base(&self) -> u16 {
        self.base
    }

    pub fn data(&self) -> u8 {
        self.data
    }

    pub fn status(&self) -> u8 {
        self.status
    }

    pub fn control(&self) -> u8 {
        self.control
    }

    /// Whether `port` falls in this device's three-byte window.
    pub fn owns_port(&self, port: u16) -> bool {
        (self.base..=self.base.saturating_add(LPT_LAST_OFFSET)).contains(&port)
    }

    /// Whether `port` is in either classic LPT1 or LPT2 window.
    pub fn owns_classic_lpt(port: u16) -> bool {
        (LPT1_BASE..=LPT1_BASE + LPT_LAST_OFFSET).contains(&port)
            || (LPT2_BASE..=LPT2_BASE + LPT_LAST_OFFSET).contains(&port)
    }

    /// Whether `port` is the classic LPT3 window (unclaimed by this model).
    pub fn is_lpt3_window(port: u16) -> bool {
        (LPT3_BASE..=LPT3_BASE + LPT_LAST_OFFSET).contains(&port)
    }
}

impl PortDevice for ParallelPort {
    fn port_read(&mut self, port: u16, _size: u8) -> u32 {
        if !self.owns_port(port) {
            return 0xFFFF_FFFF;
        }
        let off = port - self.base;
        u32::from(match off {
            LPT_DATA => self.data,
            LPT_STATUS => self.status,
            LPT_CONTROL => self.control,
            _ => 0xFF,
        })
    }

    fn port_write(&mut self, port: u16, _size: u8, value: u32) {
        if !self.owns_port(port) {
            return;
        }
        let byte = value as u8;
        match port - self.base {
            LPT_DATA => self.data = byte,
            // Spec: status is an input register from the printer; writes have no
            // modeled side effect in this stub (IRQ7 clear / ECP deferred).
            LPT_STATUS => {}
            LPT_CONTROL => self.control = byte,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spec: OSDev Parallel Port — reset leaves data clear; control idle
    /// (`/INIT` inactive + Select); status shows Busy# inactive (bit7 = 1).
    #[test]
    fn reset_defaults_no_printer_status_busy_inactive() {
        let p = ParallelPort::lpt1();
        assert_eq!(p.base(), LPT1_BASE);
        assert_eq!(p.data(), 0);
        assert_eq!(p.control(), LPT_CONTROL_DEFAULT);
        assert_eq!(p.control(), 0x0C);
        assert_eq!(p.control() & LPT_CTRL_INIT_N, LPT_CTRL_INIT_N);
        assert_eq!(p.control() & LPT_CTRL_SELECT, LPT_CTRL_SELECT);
        assert_eq!(p.status(), LPT_STATUS_NO_PRINTER);
        assert_ne!(p.status(), 0xFF, "must not look like open bus");
        assert_eq!(p.status() & LPT_STATUS_BUSY_N, LPT_STATUS_BUSY_N);
    }

    /// Spec: OSDev Parallel Port — data and control are store/readback.
    #[test]
    fn data_and_control_store_readback() {
        let mut p = ParallelPort::lpt1();
        p.port_write(LPT1_BASE, 1, 0xA5);
        assert_eq!(p.port_read(LPT1_BASE, 1) as u8, 0xA5);
        p.port_write(LPT1_BASE + LPT_CONTROL, 1, 0x0C);
        assert_eq!(p.port_read(LPT1_BASE + LPT_CONTROL, 1) as u8, 0x0C);
        // Status stays the no-printer default.
        assert_eq!(
            p.port_read(LPT1_BASE + LPT_STATUS, 1) as u8,
            LPT_STATUS_NO_PRINTER
        );
    }

    /// Spec: status writes are ignored in this stub (input register).
    #[test]
    fn status_write_ignored() {
        let mut p = ParallelPort::lpt2();
        p.port_write(LPT2_BASE + LPT_STATUS, 1, 0x00);
        assert_eq!(
            p.port_read(LPT2_BASE + LPT_STATUS, 1) as u8,
            LPT_STATUS_NO_PRINTER
        );
    }

    #[test]
    fn owns_only_three_bytes_per_base() {
        let p1 = ParallelPort::lpt1();
        let p2 = ParallelPort::lpt2();
        assert!(p1.owns_port(0x378));
        assert!(p1.owns_port(0x379));
        assert!(p1.owns_port(0x37A));
        assert!(!p1.owns_port(0x37B));
        assert!(!p1.owns_port(0x377));
        assert!(p2.owns_port(0x278));
        assert!(p2.owns_port(0x27A));
        assert!(!p2.owns_port(0x27B));
        assert!(ParallelPort::owns_classic_lpt(0x378));
        assert!(ParallelPort::owns_classic_lpt(0x27A));
        // COM3/COM4 IER sites are not LPT — owned by UART stubs on MachineBus.
        assert!(!ParallelPort::owns_classic_lpt(0x3E9));
        assert!(!ParallelPort::owns_classic_lpt(0x2E9));
        // LPT3 window is documented but unclaimed.
        assert!(ParallelPort::is_lpt3_window(LPT3_BASE));
        assert!(ParallelPort::is_lpt3_window(LPT3_BASE + 2));
        assert!(!ParallelPort::owns_classic_lpt(LPT3_BASE));
    }

    /// Spec: LPT1 and LPT2 are independent register files.
    #[test]
    fn lpt1_and_lpt2_are_independent() {
        let mut p1 = ParallelPort::lpt1();
        let mut p2 = ParallelPort::lpt2();
        p1.port_write(LPT1_BASE, 1, 0x11);
        p1.port_write(LPT1_BASE + LPT_CONTROL, 1, 0x01);
        p2.port_write(LPT2_BASE, 1, 0x22);
        p2.port_write(LPT2_BASE + LPT_CONTROL, 1, 0x02);
        assert_eq!(p1.data(), 0x11);
        assert_eq!(p1.control(), 0x01);
        assert_eq!(p2.data(), 0x22);
        assert_eq!(p2.control(), 0x02);
        assert_eq!(p1.status(), LPT_STATUS_NO_PRINTER);
        assert_eq!(p2.status(), LPT_STATUS_NO_PRINTER);
    }

    #[test]
    fn reset_clears_programmed_state() {
        let mut p = ParallelPort::lpt1();
        p.port_write(LPT1_BASE, 1, 0x55);
        p.port_write(LPT1_BASE + LPT_CONTROL, 1, 0x0F);
        p.reset();
        assert_eq!(p, ParallelPort::lpt1());
    }
}
