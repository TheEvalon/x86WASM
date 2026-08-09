//! BIOS shadowing end to end: the sequence SeaBIOS performs around
//! `make_bios_writable()`.
//!
//! Spec: Intel 440FX PMC datasheet, Programmable Attribute Map (config
//! `0x59`-`0x5F`). A region is first set to read-from-ROM / write-to-DRAM, the
//! firmware copies the region onto itself, and the region is then set to
//! read-from-DRAM / write-disabled so the shadow copy is what executes and
//! stray writes are dropped. Placement of the ROM itself comes from
//! `firmware_interface::prepare_bios_rom` (top of 4 GiB plus the below-1 MiB
//! alias), which must keep working through all of it.
//!
//! Model choices are recorded in `docs/machine-r2-pam-memory.md`.

use machine_pc::{
    Machine, PamRead, PamWrite, PAM_BIOS_REGION, PAM_FIELD_RE, PAM_FIELD_WE, PAM_REGIONS,
};

/// PAM0 — the configuration register whose high nibble owns `0xF0000`.
const PAM0: u8 = 0x59;

/// Marker the ROM carries at image offset `SENTINEL_OFF`.
const ROM_SENTINEL: u8 = 0xAA;
const SENTINEL_OFF: usize = 0x0100;

/// Guest shadowing routine, entered at `F000:0000` through the reset vector.
///
/// `REP MOVSB` twice over 32 KiB copies the whole 64 KiB region onto itself:
/// `DS:SI` reads resolve to the ROM (RE clear) and `ES:DI` writes land in
/// shadow DRAM (WE set). The `HLT` in the middle is where the host flips the
/// attributes, exactly like firmware writing PAM0 a second time.
#[rustfmt::skip]
const SHADOW_ROUTINE: &[u8] = &[
    0xFC,                   // CLD
    0xB8, 0x00, 0xF0,       // MOV AX, 0xF000
    0x8E, 0xD8,             // MOV DS, AX
    0x8E, 0xC0,             // MOV ES, AX
    0x31, 0xF6,             // XOR SI, SI
    0x31, 0xFF,             // XOR DI, DI
    0xB9, 0x00, 0x80,       // MOV CX, 0x8000
    0xF3, 0xA4,             // REP MOVSB
    0xB9, 0x00, 0x80,       // MOV CX, 0x8000
    0xF3, 0xA4,             // REP MOVSB
    0xF4,                   // HLT  (host flips PAM to read-DRAM / write-off)
    0xB0, b'R',             // MOV AL, 'R'   (shadow copy is patched to 'S')
    0xBA, 0x02, 0x04,       // MOV DX, 0x402
    0xEE,                   // OUT DX, AL
    0xF4,                   // HLT
];

/// Offset of the `MOV AL, imm8` immediate in [`SHADOW_ROUTINE`].
const ROUTINE_TAG_OFF: usize = 0x0018;

/// 64 KiB BIOS image whose reset vector far-jumps to `F000:0000`.
///
/// Spec: Intel SDM Vol. 3 §9.1.4 — the first fetch is `0xFFFFFFF0` with
/// `CS.base = 0xFFFF0000`; a far `JMP ptr16:16` to `F000:0000` moves execution
/// to the below-1 MiB alias, which is the window PAM attributes.
fn bios_image_64k(code: &[u8]) -> Vec<u8> {
    let mut rom = vec![0xF4u8; 64 * 1024];
    rom[..code.len()].copy_from_slice(code);
    rom[SENTINEL_OFF] = ROM_SENTINEL;
    rom[0xFFF0..0xFFF5].copy_from_slice(&[0xEA, 0x00, 0x00, 0x00, 0xF0]);
    rom
}

/// The full firmware sequence, driven by guest instructions.
#[test]
fn guest_shadows_bios_region_then_locks_it() {
    let rom = bios_image_64k(SHADOW_ROUTINE);
    let mut m = Machine::with_bios_rom(1024 * 1024, &rom).expect("map BIOS image");
    m.reset();

    // Reset attributes: the alias reads ROM and a write is dropped, not a
    // fault. This is what lets POST survive a stray write before PAM is set.
    assert_eq!(
        m.mem.read_u8(0x000F_0000 + SENTINEL_OFF as u64),
        Ok(ROM_SENTINEL)
    );
    assert_eq!(
        m.mem.write_u8(0x000F_0000 + SENTINEL_OFF as u64, 0x00),
        Ok(())
    );
    assert_eq!(
        m.mem.read_u8(0x000F_0000 + SENTINEL_OFF as u64),
        Ok(ROM_SENTINEL)
    );

    // make_bios_writable(): PAM0 high nibble WE — read ROM, write DRAM.
    assert!(m.apply_pam_register(PAM0, PAM_FIELD_WE << 4));
    let steps = m.run(200_000).expect("shadow copy runs");
    assert!(m.cpu.halted, "guest halts after the copy ({steps} steps)");

    // The copy landed in shadow DRAM but reads still come from ROM.
    assert_eq!(
        m.mem.read_u8(0x000F_0000 + SENTINEL_OFF as u64),
        Ok(ROM_SENTINEL)
    );

    // Patch the shadow copy while writes still reach DRAM, so the next fetch
    // can only produce 'S' if it came from shadow rather than from ROM.
    m.mem
        .write_u8(0x000F_0000 + ROUTINE_TAG_OFF as u64, b'S')
        .expect("write reaches shadow DRAM while WE is set");

    // Lock: PAM0 high nibble RE — read DRAM, writes dropped.
    assert!(m.apply_pam_register(PAM0, PAM_FIELD_RE << 4));
    assert_eq!(
        m.pam_attributes(PAM_BIOS_REGION).map(|a| (a.read, a.write)),
        Some((PamRead::ShadowRam, PamWrite::Ignored))
    );

    // Reads now come from the shadow copy, including the patched byte.
    assert_eq!(
        m.mem.read_u8(0x000F_0000 + SENTINEL_OFF as u64),
        Ok(ROM_SENTINEL)
    );
    assert_eq!(
        m.mem.read_u8(0x000F_0000 + ROUTINE_TAG_OFF as u64),
        Ok(b'S')
    );

    // Writes are dropped without faulting.
    assert_eq!(
        m.mem.write_u8(0x000F_0000 + ROUTINE_TAG_OFF as u64, 0x99),
        Ok(())
    );
    assert_eq!(
        m.mem.read_u8(0x000F_0000 + ROUTINE_TAG_OFF as u64),
        Ok(b'S')
    );

    // The top-of-4 GiB window is outside PAM and still returns the ROM image.
    assert_eq!(
        m.mem.read_u8(0xFFFF_0000 + ROUTINE_TAG_OFF as u64),
        Ok(b'R')
    );
    assert_eq!(
        m.mem.read_u8(0xFFFF_0000 + SENTINEL_OFF as u64),
        Ok(ROM_SENTINEL)
    );

    // Resume: the instruction stream after the HLT is fetched from shadow.
    m.cpu.halted = false;
    m.run(64).expect("resume from shadow");
    assert!(m.cpu.halted);
    assert_eq!(m.debug_text(), "S", "fetch came from the shadow copy");
}

/// A guest write to the BIOS area with PAM at its reset value must not fault.
///
/// Before this slice `PhysMem::write_u8` returned `RomWrite`, which the bus
/// turns into a memory fault and which would stop POST at the first attempt to
/// write the BIOS area.
#[test]
fn guest_write_to_locked_bios_area_is_dropped_not_faulted() {
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x00, 0xF0,       // MOV AX, 0xF000
        0x8E, 0xC0,             // MOV ES, AX
        0xBF, 0x00, 0x01,       // MOV DI, 0x0100
        0xB0, 0x11,             // MOV AL, 0x11
        0x26, 0x88, 0x05,       // MOV ES:[DI], AL
        0xB0, b'K',             // MOV AL, 'K'
        0xBA, 0x02, 0x04,       // MOV DX, 0x402
        0xEE,                   // OUT DX, AL
        0xF4,                   // HLT
    ];
    let rom = bios_image_64k(code);
    let mut m = Machine::with_bios_rom(1024 * 1024, &rom).expect("map BIOS image");
    m.reset();

    m.run(64).expect("write is dropped, not faulted");

    assert!(m.cpu.halted);
    assert_eq!(m.debug_text(), "K", "execution continued past the write");
    assert_eq!(
        m.mem.read_u8(0x000F_0000 + SENTINEL_OFF as u64),
        Ok(ROM_SENTINEL)
    );
}

/// A 256 KiB image aliases its last 128 KiB at `0xE0000`, which spans PAM
/// regions 8-11 as well as the BIOS area. Shadowing one region must not move
/// the others, and the high map must keep the untouched image.
#[test]
fn shadowing_the_e0000_alias_leaves_the_high_map_and_neighbours_alone() {
    let mut img = vec![0xF4u8; 256 * 1024];
    img[128 * 1024] = 0x22; // first byte of the low alias -> 0xE0000
    img[128 * 1024 + 16 * 1024] = 0x33; // 0xE4000, the next PAM region
    let mut m = Machine::with_bios_rom(1024 * 1024, &img).expect("map 256 KiB BIOS");

    assert_eq!(m.mem.read_u8(0x000E_0000), Ok(0x22));
    assert_eq!(m.mem.read_u8(0x000E_4000), Ok(0x33));

    // Region 8 is 0xE0000-0xE3FFF (PAM5 low nibble, config 0x5E).
    assert_eq!(PAM_REGIONS[8].0, 0x000E_0000);
    assert!(m.apply_pam_register(0x5E, PAM_FIELD_WE));
    m.mem.write_u8(0x000E_0000, 0x99).expect("write to shadow");
    // Neighbouring region 9 is untouched: its write is still dropped.
    assert_eq!(m.mem.write_u8(0x000E_4000, 0x99), Ok(()));

    assert!(m.apply_pam_register(0x5E, PAM_FIELD_RE));
    assert_eq!(m.mem.read_u8(0x000E_0000), Ok(0x99));
    assert_eq!(m.mem.read_u8(0x000E_4000), Ok(0x33));

    // prepare_bios_rom's high placement is unaffected by any of this.
    assert_eq!(m.mem.read_u8(0xFFFE_0000), Ok(0x22));
    assert_eq!(m.mem.read_u8(0xFFFE_4000), Ok(0x33));
}
