//! CMOS shutdown status `0Fh` survives Machine pulse-reset.
//!
//! Spec: IBM PC/AT CMOS map + RBIL CMOS 0Fh — battery-backed shutdown / reset
//! code. SeaBIOS soft-reset writes `0Ah` before CF9/`qemu_reboot`. Machine
//! exposes get/set helpers and does **not** dispatch on the code.

use devices::{
    PortDevice, CMOS_DATA, CMOS_INDEX, REG_SHUTDOWN, SHUTDOWN_JMP, SHUTDOWN_SOFT_OR_UNEXPECTED,
};
use machine_pc::Machine;

#[test]
fn machine_shutdown_status_helpers_and_port_path() {
    let mut m = Machine::new(64 * 1024);
    assert_eq!(m.shutdown_status(), SHUTDOWN_SOFT_OR_UNEXPECTED);
    m.set_shutdown_status(SHUTDOWN_JMP);
    assert_eq!(m.shutdown_status(), SHUTDOWN_JMP);
    m.cmos.port_write(CMOS_INDEX, 1, u32::from(REG_SHUTDOWN));
    assert_eq!(m.cmos.port_read(CMOS_DATA, 1) as u8, SHUTDOWN_JMP);
}

#[test]
fn machine_reset_preserves_shutdown_status() {
    let mut m = Machine::new(64 * 1024);
    m.set_shutdown_status(SHUTDOWN_JMP);
    m.reset();
    assert_eq!(m.shutdown_status(), SHUTDOWN_JMP);
}
