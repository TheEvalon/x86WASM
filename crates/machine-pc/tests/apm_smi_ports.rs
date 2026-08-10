//! APM/SMI ports `0xB2`/`0xB3` complete SeaBIOS's SMM bring-up poll and
//! resume a post-command `HLT` without entering SMM.
//!
//! Spec: Intel APM_CNT/APM_STS fixed I/O; SeaBIOS `smm_relocate_and_restore`
//! writes status `0x01`, raises SMI via command `0x00`, then polls status
//! until the handler clears it. Optional `call32_smm` does `OUT 0xB2` then
//! `HLT`. This machine stub-completes on the command write and may clear
//! halt; it still does not enter SMM. See `docs/apm-r9-smi-handshake.md`.

use devices::{ApmSmi, PortDevice, APM_CNT_PORT, APM_STS_PORT};
use machine_pc::{Machine, PostStopReason};

#[test]
fn device_apm_is_wired_on_machine_reset() {
    let mut m = Machine::new(64 * 1024);
    m.apm.port_write(APM_STS_PORT, 1, 0x01);
    m.apm.port_write(APM_CNT_PORT, 1, 0x00);
    assert_eq!(m.apm.status(), 0);
    assert_eq!(m.apm.stub_completions(), 1);
    m.reset();
    assert_eq!(m.apm, ApmSmi::new());
}

#[test]
fn probe_seabios_style_apm_poll_halts_without_unclaimed_ports() {
    let mut rom = vec![0xF4u8; 64 * 1024];
    #[rustfmt::skip]
    let entry: &[u8] = &[
        0xB0, 0x01, 0xE6, 0xB3, // MOV AL,1 / OUT 0xB3,AL
        0xB0, 0x00, 0xE6, 0xB2, // MOV AL,0 / OUT 0xB2,AL
        0xE4, 0xB3,             // IN AL,0xB3
        0x84, 0xC0,             // TEST AL,AL
        0x75, 0xFA,             // JNZ back to IN (must not spin)
        0xF4,                   // HLT
    ];
    rom[..entry.len()].copy_from_slice(entry);
    rom[0xFFF0..0xFFF5].copy_from_slice(&[0xEA, 0x00, 0x00, 0x00, 0xF0]);

    let mut m = Machine::with_bios_rom(1024 * 1024, &rom).expect("map");
    let report = m.probe_post(64);

    assert_eq!(report.stop, PostStopReason::Halted, "{report}");
    assert!(
        !report
            .unclaimed_ports
            .iter()
            .any(|a| a.port == 0xB2 || a.port == 0xB3),
        "APM ports must be claimed: {report}"
    );
    assert_eq!(m.apm.stub_completions(), 1, "{report}");
    assert_eq!(m.apm.status(), 0, "{report}");
    assert_eq!(
        m.apm.smi_wake_stubs(),
        0,
        "status-0 poll must consume pending before HLT: {report}"
    );
}

/// Spec: SDM `HLT` + APM_CNT — OUT then HLT must not permanently wedge when the
/// stub latches a pending SMI (still no real SMM / EIP rewrite).
#[test]
fn out_apm_cnt_then_hlt_resumes_without_smm() {
    let mut rom = vec![0xF4u8; 64 * 1024];
    #[rustfmt::skip]
    let entry: &[u8] = &[
        0xB0, 0x01, 0xE6, 0xB2, // MOV AL,1 / OUT 0xB2,AL  (call32-style)
        0xF4,                   // HLT — stub must wake
        0xF4,                   // second HLT — permanent (pending already consumed)
    ];
    rom[..entry.len()].copy_from_slice(entry);
    rom[0xFFF0..0xFFF5].copy_from_slice(&[0xEA, 0x00, 0x00, 0x00, 0xF0]);

    let mut m = Machine::with_bios_rom(1024 * 1024, &rom).expect("map");
    let report = m.probe_post(64);

    assert_eq!(report.stop, PostStopReason::Halted, "{report}");
    assert_eq!(m.apm.stub_completions(), 1, "{report}");
    assert_eq!(m.apm.smi_wake_stubs(), 1, "{report}");
    assert!(!m.apm.smi_pending(), "{report}");
}
