//! Option ROM mapping in the legacy `0xC0000`-`0xDFFFF` region.
//!
//! Spec: PCI Firmware Specification / BIOS Boot Specification, PC-compatible
//! expansion ROM header — `0x55 0xAA` signature, initialization size in
//! 512-byte blocks at offset 2, entry point from offset 3, byte-wise checksum
//! zero over the declared size; the BIOS scans the region on 2 KiB boundaries.
//! A real VGA BIOS image is not part of this tree, so the blobs here are
//! synthetic and nothing executes them.

use firmware_interface::{
    OptionRomError, OPTION_ROM_BLOCK_SIZE, OPTION_ROM_SCAN_STEP, VGA_OPTION_ROM_BASE,
};
use machine_pc::{Machine, PamRead, PamWrite, PAM_FIELD_RE, PAM_FIELD_WE};

/// Conventional memory ends at `0xA0000`, so nothing in the legacy window is
/// DRAM-backed and an unmapped scan slot reads open bus rather than zeroes.
const CONVENTIONAL_RAM: usize = 640 * 1024;

/// Valid expansion ROM of `blocks` 512-byte blocks, checksum corrected.
fn synthetic_option_rom(blocks: u8, marker: u8) -> Vec<u8> {
    let mut rom = vec![0u8; usize::from(blocks) * OPTION_ROM_BLOCK_SIZE];
    rom[0] = 0x55;
    rom[1] = 0xAA;
    rom[2] = blocks;
    rom[3] = 0xCB; // RETF entry-point stub
    rom[4] = marker;
    let sum = rom.iter().fold(0u8, |acc, b| acc.wrapping_add(*b));
    let last = rom.len() - 1;
    rom[last] = rom[last].wrapping_sub(sum);
    rom
}

/// 64 KiB BIOS image whose reset vector far-jumps to `F000:0000`.
fn bios_image_64k(code: &[u8]) -> Vec<u8> {
    let mut rom = vec![0xF4u8; 64 * 1024];
    rom[..code.len()].copy_from_slice(code);
    rom[0xFFF0..0xFFF5].copy_from_slice(&[0xEA, 0x00, 0x00, 0x00, 0xF0]);
    rom
}

/// The 2 KiB-granular signature scan a BIOS performs finds the ROM, and the
/// declared size and checksum are what the scan would read.
#[test]
fn option_rom_is_visible_to_a_signature_scan() {
    let rom = synthetic_option_rom(4, 0x5A);
    let mut m =
        Machine::with_bios_rom(CONVENTIONAL_RAM, &bios_image_64k(&[0xF4])).expect("map BIOS");
    m.map_vga_option_rom(&rom).expect("map option ROM");

    assert_eq!(m.mem.read_u8(VGA_OPTION_ROM_BASE), Ok(0x55));
    assert_eq!(m.mem.read_u8(VGA_OPTION_ROM_BASE + 1), Ok(0xAA));
    assert_eq!(m.mem.read_u8(VGA_OPTION_ROM_BASE + 2), Ok(4));
    assert_eq!(m.mem.read_u8(VGA_OPTION_ROM_BASE + 4), Ok(0x5A));

    // The mapped image checksums to zero, as the scan requires before entry.
    let len = 4 * OPTION_ROM_BLOCK_SIZE as u64;
    let sum = (0..len).fold(0u8, |acc, off| {
        acc.wrapping_add(m.mem.read_u8(VGA_OPTION_ROM_BASE + off).unwrap())
    });
    assert_eq!(sum, 0);

    // Nothing is mapped at the next scan slot.
    assert_eq!(
        m.mem
            .read_u8(VGA_OPTION_ROM_BASE + OPTION_ROM_SCAN_STEP * 2),
        Ok(0xFF)
    );
}

/// The guest sees the signature at `C000:0000`, which is where SeaBIOS's scan
/// starts.
#[test]
fn guest_reads_the_option_rom_signature() {
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x00, 0xC0,       // MOV AX, 0xC000
        0x8E, 0xD8,             // MOV DS, AX
        0xA1, 0x00, 0x00,       // MOV AX, [0x0000]
        0x3D, 0x55, 0xAA,       // CMP AX, 0xAA55
        0x75, 0x04,             // JNE +4
        0xB0, b'V',             // MOV AL, 'V'
        0xEB, 0x02,             // JMP +2
        0xB0, b'?',             // MOV AL, '?'
        0xBA, 0x02, 0x04,       // MOV DX, 0x402
        0xEE,                   // OUT DX, AL
        0xF4,                   // HLT
    ];
    let mut m = Machine::with_bios_rom(CONVENTIONAL_RAM, &bios_image_64k(code)).expect("map BIOS");
    m.map_vga_option_rom(&synthetic_option_rom(2, 0x11))
        .expect("map option ROM");
    m.reset();

    m.run(64).expect("scan runs");

    assert!(m.cpu.halted);
    assert_eq!(m.debug_text(), "V");
}

/// Option ROMs live under PAM regions 0-7, so the shadowing sequence works
/// there too. Spec: Intel 440FX PMC PAM1 (`0x5A`) low nibble = `0xC0000`.
#[test]
fn option_rom_region_can_be_shadowed_through_pam() {
    let mut m =
        Machine::with_bios_rom(CONVENTIONAL_RAM, &bios_image_64k(&[0xF4])).expect("map BIOS");
    m.map_vga_option_rom(&synthetic_option_rom(4, 0x5A))
        .expect("map option ROM");

    // Read ROM / write DRAM, copy the region onto itself, then lock.
    assert!(m.apply_pam_register(0x5A, PAM_FIELD_WE));
    for off in 0..4 * OPTION_ROM_BLOCK_SIZE as u64 {
        let b = m.mem.read_u8(VGA_OPTION_ROM_BASE + off).unwrap();
        m.mem.write_u8(VGA_OPTION_ROM_BASE + off, b).unwrap();
    }
    m.mem.write_u8(VGA_OPTION_ROM_BASE + 4, 0x99).unwrap();
    assert_eq!(m.mem.read_u8(VGA_OPTION_ROM_BASE + 4), Ok(0x5A));

    assert!(m.apply_pam_register(0x5A, PAM_FIELD_RE));
    assert_eq!(
        m.pam_attributes(0).map(|a| (a.read, a.write)),
        Some((PamRead::ShadowRam, PamWrite::Ignored))
    );
    assert_eq!(m.mem.read_u8(VGA_OPTION_ROM_BASE), Ok(0x55));
    assert_eq!(m.mem.read_u8(VGA_OPTION_ROM_BASE + 4), Ok(0x99));
    assert_eq!(m.mem.write_u8(VGA_OPTION_ROM_BASE + 4, 0x00), Ok(()));
    assert_eq!(m.mem.read_u8(VGA_OPTION_ROM_BASE + 4), Ok(0x99));
}

/// A malformed image is rejected rather than mapped, so a scan never sees a
/// half-valid ROM.
#[test]
fn malformed_option_rom_is_rejected() {
    let mut m =
        Machine::with_bios_rom(CONVENTIONAL_RAM, &bios_image_64k(&[0xF4])).expect("map BIOS");
    let mut bad = synthetic_option_rom(2, 0x00);
    bad[8] = bad[8].wrapping_add(1); // break the checksum

    let err = m.map_vga_option_rom(&bad).expect_err("checksum rejected");
    assert!(matches!(
        err,
        machine_pc::MachineError::OptionRom(OptionRomError::BadChecksum)
    ));
    assert_eq!(m.mem.read_u8(VGA_OPTION_ROM_BASE), Ok(0xFF));
}

/// The BIOS windows and the option-ROM window coexist; reloading the BIOS
/// clears every window, which is the documented ordering requirement.
#[test]
fn option_rom_coexists_with_the_bios_windows() {
    let bios = bios_image_64k(&[0xF4]);
    let mut m = Machine::with_bios_rom(CONVENTIONAL_RAM, &bios).expect("map BIOS");
    m.map_vga_option_rom(&synthetic_option_rom(2, 0x22))
        .expect("map option ROM");

    assert_eq!(m.mem.read_u8(0xFFFF_FFF0), Ok(0xEA));
    assert_eq!(m.mem.read_u8(0x000F_FFF0), Ok(0xEA));
    assert_eq!(m.mem.read_u8(VGA_OPTION_ROM_BASE), Ok(0x55));

    m.load_bios_rom(&bios).expect("reload BIOS");
    assert_eq!(
        m.mem.read_u8(VGA_OPTION_ROM_BASE),
        Ok(0xFF),
        "reloading the BIOS clears every ROM window"
    );
}
