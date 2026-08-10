//! LPT1/LPT2 parallel-port stubs claim classic bases on MachineBus.
//!
//! Spec: IBM PC / OSDev Parallel Port — data/status/control at `0x378`–`0x37A`
//! and `0x278`–`0x27A`. Status bit7 Busy is inactive (high) with no printer.
//! R13: control idle default `/INIT` inactive + Select (`0x0C`); LPT3 `0x3BC`
//! remains open-bus; COM3/COM4 IER sites are UART-owned.

use devices::{
    ParallelPort, PortDevice, LPT1_BASE, LPT2_BASE, LPT3_BASE, LPT_CONTROL, LPT_CONTROL_DEFAULT,
    LPT_STATUS_BUSY_N, LPT_STATUS_NO_PRINTER,
};
use machine_pc::{Machine, PostStopReason};
use x86_core::CpuState;

#[test]
fn device_lpt_wired_on_machine_reset() {
    let mut m = Machine::new(64 * 1024);
    assert_eq!(m.lpt1.control(), LPT_CONTROL_DEFAULT);
    assert_eq!(m.lpt2.control(), LPT_CONTROL_DEFAULT);
    m.lpt1.port_write(LPT1_BASE, 1, 0x11);
    m.lpt2.port_write(LPT2_BASE + LPT_CONTROL, 1, 0x0C);
    m.reset();
    assert_eq!(m.lpt1, ParallelPort::lpt1());
    assert_eq!(m.lpt2, ParallelPort::lpt2());
}

#[test]
fn lpt1_lpt2_independent_register_files() {
    let mut m = Machine::new(64 * 1024);
    m.lpt1.port_write(LPT1_BASE, 1, 0x11);
    m.lpt2.port_write(LPT2_BASE, 1, 0x22);
    m.lpt1.port_write(LPT1_BASE + LPT_CONTROL, 1, 0x01);
    m.lpt2.port_write(LPT2_BASE + LPT_CONTROL, 1, 0x02);
    assert_eq!(m.lpt1.data(), 0x11);
    assert_eq!(m.lpt2.data(), 0x22);
    assert_eq!(m.lpt1.control(), 0x01);
    assert_eq!(m.lpt2.control(), 0x02);
    // LPT3 honesty: classic MDA printer base is not owned by either stub.
    assert!(!m.lpt1.owns_port(LPT3_BASE));
    assert!(!m.lpt2.owns_port(LPT3_BASE));
    assert!(ParallelPort::is_lpt3_window(LPT3_BASE));
}

#[test]
fn probe_lpt_ports_claimed_no_gp() {
    // Guest: OUT data to LPT1/LPT2, IN status (expect Busy# inactive), HLT.
    // OUT imm8 only reaches ports 0x00–0xFF; use DX forms for 0x378/0x278.
    let mut rom = vec![0xF4u8; 64 * 1024];
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBA, 0x78, 0x03, // MOV DX,0x0378
        0xB0, 0x55,       // MOV AL,0x55
        0xEE,             // OUT DX,AL
        0x42,             // INC DX -> 0x379 status
        0xEC,             // IN AL,DX
        0xBA, 0x78, 0x02, // MOV DX,0x0278
        0xB0, 0xAA,       // MOV AL,0xAA
        0xEE,             // OUT DX,AL
        0x42,             // INC DX -> 0x279
        0xEC,             // IN AL,DX
        0xF4,             // HLT
    ];
    rom[..code.len()].copy_from_slice(code);
    rom[0xFFF0..0xFFF5].copy_from_slice(&[0xEA, 0x00, 0x00, 0x00, 0xF0]);

    let mut m = Machine::with_bios_rom(1024 * 1024, &rom).expect("map");
    let report = m.probe_post(64);

    assert_eq!(report.stop, PostStopReason::Halted, "{report}");
    assert!(
        !report.unclaimed_ports.iter().any(|a| {
            (LPT1_BASE..=LPT1_BASE + 2).contains(&a.port)
                || (LPT2_BASE..=LPT2_BASE + 2).contains(&a.port)
        }),
        "LPT1/LPT2 must be claimed: {report}"
    );
    assert_eq!(m.lpt1.data(), 0x55);
    assert_eq!(m.lpt2.data(), 0xAA);
    assert_eq!(m.lpt1.status() & LPT_STATUS_BUSY_N, LPT_STATUS_BUSY_N);
    assert_eq!(m.lpt1.status(), LPT_STATUS_NO_PRINTER);
}

#[test]
fn bus_direct_lpt_status_not_open_bus() {
    let mut rom = vec![0xF4u8; 64 * 1024];
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBA, 0x79, 0x03, // MOV DX,0x0379
        0xEC,             // IN AL,DX
        0xF4,
    ];
    rom[..code.len()].copy_from_slice(code);
    rom[0xFFF0..0xFFF5].copy_from_slice(&[0xEA, 0x00, 0x00, 0x00, 0xF0]);
    let mut m = Machine::with_bios_rom(1024 * 1024, &rom).expect("map");
    let _ = m.run(16).expect("run");
    assert_eq!(m.cpu.gpr[CpuState::RAX] as u8, LPT_STATUS_NO_PRINTER);
}
