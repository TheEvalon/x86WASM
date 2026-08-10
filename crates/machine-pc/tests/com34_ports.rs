//! COM3/COM4 UART stubs claim historically unclaimed POST probe sites.
//!
//! Spec: NS16550A / classic PC — COM3 `0x3E8`–`0x3EF`, COM4 `0x2E8`–`0x2EF`.
//! R11 POST remeasure logged unclaimed `0x3E9`/`0x2E9` (IER). R13 claims the
//! full windows as debug-UART stubs without ISA IRQ routing.

use devices::{PortDevice, COM3_BASE, COM4_BASE};
use machine_pc::{Machine, PostStopReason};

#[test]
fn com3_com4_device_thr_and_reset() {
    let mut m = Machine::new(64 * 1024);
    assert_eq!(m.com3.port_read(COM3_BASE + 1, 1), 0); // IER
    assert_eq!(m.com4.port_read(COM4_BASE + 1, 1), 0);
    m.com3.port_write(COM3_BASE, 1, u32::from(b'C'));
    m.com4.port_write(COM4_BASE, 1, u32::from(b'D'));
    assert_eq!(m.com3_text(), "C");
    assert_eq!(m.com4_text(), "D");
    m.reset();
    assert_eq!(m.com3_text(), "");
    assert_eq!(m.com4_text(), "");
}

#[test]
fn probe_com3_com4_not_unclaimed() {
    let mut rom = vec![0xF4u8; 64 * 1024];
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBA, 0xE9, 0x03, // MOV DX,0x03E9  COM3 IER
        0xEC,             // IN AL,DX
        0xBA, 0xE9, 0x02, // MOV DX,0x02E9  COM4 IER
        0xEC,             // IN AL,DX
        0xF4,             // HLT
    ];
    rom[..code.len()].copy_from_slice(code);
    rom[0xFFF0..0xFFF5].copy_from_slice(&[0xEA, 0x00, 0x00, 0x00, 0xF0]);

    let mut m = Machine::with_bios_rom(1024 * 1024, &rom).expect("map");
    let report = m.probe_post(64);
    assert_eq!(report.stop, PostStopReason::Halted, "{report}");
    assert!(
        !report
            .unclaimed_ports
            .iter()
            .any(|a| a.port == 0x3E9 || a.port == 0x2E9),
        "COM3/COM4 IER must be claimed: {report}"
    );
}
