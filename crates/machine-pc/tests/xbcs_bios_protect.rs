//! PIIX XBCS (`4Eh`) controls BIOSCS# write-protect in the memory model.
//!
//! Spec: Intel 82371AB §4.1.9 — default `03h`; bit2 enables BIOSCS# on writes
//! when set. ROM image bytes are never mutated either way.

use machine_pc::{
    Machine, PostStopReason, WriteDisposition, XBCS_BIOS_WRITE_PROTECT_ENABLE, XBCS_DEFAULT,
};

#[test]
fn xbcs_defaults_to_piix_03h_with_write_protect() {
    let m = Machine::new(64 * 1024);
    assert_eq!(m.xbcs.value(), XBCS_DEFAULT);
    assert!(m.mem.bios_write_protect());
}

#[test]
fn guest_config_write_lifts_bios_write_protect() {
    // Real-mode program: write XBCS via Mechanism #1, then HLT.
    // CF8 ← enable|bus0|dev1|fn0|reg 0x4C; CFE ← (DEFAULT|bit2) for byte 4Eh.
    let mut rom = vec![0xF4u8; 64 * 1024];
    let xbcs = XBCS_DEFAULT | XBCS_BIOS_WRITE_PROTECT_ENABLE;
    #[rustfmt::skip]
    let entry: &[u8] = &[
        // MOV EAX, 0x8000084C — Type-1 address for 00:01.0 dword 0x4C
        0x66, 0xB8, 0x4C, 0x08, 0x00, 0x80,
        0xBA, 0xF8, 0x0C,             // MOV DX, 0xCF8
        0x66, 0xEF,                   // OUT DX, EAX
        0xB0, xbcs,                   // MOV AL, xbcs
        0xBA, 0xFE, 0x0C,             // MOV DX, 0xCFE
        0xEE,                         // OUT DX, AL
        0xEC,                         // IN AL, DX
        0xF4,                         // HLT
    ];
    rom[..entry.len()].copy_from_slice(entry);
    rom[0xFFF0..0xFFF5].copy_from_slice(&[0xEA, 0x00, 0x00, 0x00, 0xF0]);

    let mut m = Machine::with_bios_rom(1024 * 1024, &rom).expect("map");
    let report = m.probe_post(64);
    assert_eq!(report.stop, PostStopReason::Halted, "{report}");
    assert_eq!(m.cpu.al(), xbcs, "readback through XBCS overlay: {report}");
    assert_eq!(m.xbcs.value(), xbcs, "{report}");
    assert!(!m.mem.bios_write_protect(), "{report}");
}

#[test]
fn rom_writes_still_drop_when_write_protect_lifted() {
    let mut m = Machine::new(1024 * 1024);
    m.xbcs.write(XBCS_DEFAULT | XBCS_BIOS_WRITE_PROTECT_ENABLE);
    m.mem
        .set_bios_write_protect(m.xbcs.bios_write_protect_enabled());
    assert!(!m.mem.bios_write_protect());
    m.mem.add_rom(0xFFFF_0000, vec![0xAA; 64]);
    assert_eq!(
        m.mem.write_u8_classified(0xFFFF_0000, 0x55),
        WriteDisposition::DroppedRom
    );
    assert_eq!(m.mem.read_u8(0xFFFF_0000).unwrap(), 0xAA);
}

#[test]
fn reset_restores_xbcs_default() {
    let mut m = Machine::new(64 * 1024);
    m.xbcs.write(XBCS_DEFAULT | XBCS_BIOS_WRITE_PROTECT_ENABLE);
    m.mem.set_bios_write_protect(false);
    m.reset();
    assert_eq!(m.xbcs.value(), XBCS_DEFAULT);
    assert!(m.mem.bios_write_protect());
}
