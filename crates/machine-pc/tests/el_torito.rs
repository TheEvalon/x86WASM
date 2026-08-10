//! Host-side El Torito detection and no-emulation boot handoff.
//!
//! # Spec refs
//!
//! - "El Torito" Bootable CD-ROM Format Specification Version 1.0 — Boot Record
//!   Volume Descriptor, Validation Entry key bytes `55h`/`AAh`, Initial/Default
//!   Entry boot indicator `88h`, no-emulation media type `00h`, load segment
//!   default `07C0h`.
//! - Round-8 host handoff restores `Machine::load_eltorito_to_7c00`; INT 13h CD
//!   emulation remains out of scope.

use firmware_interface::{
    ElToritoError, EL_TORITO_BOOTABLE, EL_TORITO_BOOT_SYSTEM_ID, EL_TORITO_DEFAULT_LOAD_PHYS,
    EL_TORITO_DEFAULT_LOAD_SEGMENT, EL_TORITO_KEY_55, EL_TORITO_KEY_AA, EL_TORITO_MEDIA_NO_EMUL,
    EL_TORITO_PLATFORM_X86, EL_TORITO_SECTOR_BYTES, EL_TORITO_VALIDATION_HEADER_ID,
    ISO9660_STANDARD_ID, ISO9660_VD_BOOT_RECORD, ISO9660_VD_TERMINATOR,
};
use machine_pc::{Machine, MachineError};

fn blank_iso(sectors: usize) -> Vec<u8> {
    vec![0u8; sectors * EL_TORITO_SECTOR_BYTES]
}

fn write_sector(img: &mut [u8], lba: u32, data: &[u8]) {
    let start = lba as usize * EL_TORITO_SECTOR_BYTES;
    img[start..start + data.len()].copy_from_slice(data);
}

fn make_bootable_iso(boot_fill: u8) -> Vec<u8> {
    let mut img = blank_iso(32);
    let mut pvd = vec![0u8; EL_TORITO_SECTOR_BYTES];
    pvd[0] = 1;
    pvd[1..6].copy_from_slice(ISO9660_STANDARD_ID);
    pvd[6] = 1;
    write_sector(&mut img, 16, &pvd);

    let mut br = vec![0u8; EL_TORITO_SECTOR_BYTES];
    br[0] = ISO9660_VD_BOOT_RECORD;
    br[1..6].copy_from_slice(ISO9660_STANDARD_ID);
    br[6] = 1;
    br[7..7 + EL_TORITO_BOOT_SYSTEM_ID.len()].copy_from_slice(EL_TORITO_BOOT_SYSTEM_ID);
    let catalog_lba = 20u32;
    br[0x47..0x4B].copy_from_slice(&catalog_lba.to_le_bytes());
    write_sector(&mut img, 17, &br);

    let mut term = vec![0u8; EL_TORITO_SECTOR_BYTES];
    term[0] = ISO9660_VD_TERMINATOR;
    term[1..6].copy_from_slice(ISO9660_STANDARD_ID);
    term[6] = 1;
    write_sector(&mut img, 18, &term);

    let mut cat = vec![0u8; EL_TORITO_SECTOR_BYTES];
    let mut validation = [0u8; 32];
    validation[0] = EL_TORITO_VALIDATION_HEADER_ID;
    validation[1] = EL_TORITO_PLATFORM_X86;
    validation[30] = EL_TORITO_KEY_55;
    validation[31] = EL_TORITO_KEY_AA;
    let mut sum = 0u16;
    for i in (0..32).step_by(2) {
        if i == 28 {
            continue;
        }
        sum = sum.wrapping_add(u16::from_le_bytes([validation[i], validation[i + 1]]));
    }
    let checksum = 0u16.wrapping_sub(sum);
    validation[28..30].copy_from_slice(&checksum.to_le_bytes());
    cat[0..32].copy_from_slice(&validation);
    cat[32] = EL_TORITO_BOOTABLE;
    cat[33] = EL_TORITO_MEDIA_NO_EMUL;
    cat[38..40].copy_from_slice(&4u16.to_le_bytes());
    let boot_lba = 24u32;
    cat[40..44].copy_from_slice(&boot_lba.to_le_bytes());
    write_sector(&mut img, catalog_lba, &cat);

    let mut boot = vec![boot_fill; EL_TORITO_SECTOR_BYTES];
    boot[0] = 0xF4; // HLT
    write_sector(&mut img, boot_lba, &boot);
    img
}

#[test]
fn machine_reports_bootable_el_torito_from_atapi_image() {
    let mut m = Machine::new(16 * 1024 * 1024);
    m.attach_atapi_cdrom_image(make_bootable_iso(0x90));
    let info = m.inspect_atapi_el_torito().expect("El Torito present");
    assert_eq!(info.platform_id, EL_TORITO_PLATFORM_X86);
    assert!(info.bootable);
    assert_eq!(info.load_rba, 24);
    assert_eq!(info.media_type, EL_TORITO_MEDIA_NO_EMUL);
}

#[test]
fn empty_atapi_tray_rejects_el_torito_inspect() {
    let mut m = Machine::new(16 * 1024 * 1024);
    m.ide.attach_atapi_cdrom();
    assert_eq!(m.inspect_atapi_el_torito(), Err(ElToritoError::Truncated));
}

/// Spec: El Torito no-emul — load to phys `0x7C00`, `CS:IP = 07C0:0000`.
#[test]
fn load_eltorito_to_7c00_sets_cs_ip_and_memory() {
    let mut m = Machine::new(64 * 1024);
    m.attach_atapi_cdrom_image(make_bootable_iso(0x90));
    m.load_eltorito_to_7c00().expect("handoff");
    assert_eq!(m.cpu.cs.selector, EL_TORITO_DEFAULT_LOAD_SEGMENT);
    assert_eq!(m.cpu.ip16(), 0);
    assert_eq!(m.mem.read_u8(EL_TORITO_DEFAULT_LOAD_PHYS).unwrap(), 0xF4);
    assert_eq!(
        m.mem.read_u8(EL_TORITO_DEFAULT_LOAD_PHYS + 1).unwrap(),
        0x90
    );
}

/// Spec: after handoff, guest fetch at `07C0:0000` runs the boot image.
#[test]
fn load_eltorito_handoff_executes_hlt() {
    let mut m = Machine::new(64 * 1024);
    m.attach_atapi_cdrom_image(make_bootable_iso(0x90));
    m.load_eltorito_to_7c00().unwrap();
    assert!(!m.cpu.halted);
    m.step().expect("HLT");
    assert!(m.cpu.halted);
}

#[test]
fn load_eltorito_rejects_floppy_emulation() {
    let mut iso = make_bootable_iso(0x00);
    iso[20 * EL_TORITO_SECTOR_BYTES + 33] = 0x02;
    let mut m = Machine::new(64 * 1024);
    m.attach_atapi_cdrom_image(iso);
    assert!(matches!(
        m.load_eltorito_to_7c00(),
        Err(MachineError::ElToritoUnsupportedMedia)
    ));
}

#[test]
fn load_eltorito_rejects_missing_boot_image() {
    let mut iso = make_bootable_iso(0x00);
    iso[20 * EL_TORITO_SECTOR_BYTES + 40..20 * EL_TORITO_SECTOR_BYTES + 44]
        .copy_from_slice(&100u32.to_le_bytes());
    let mut m = Machine::new(64 * 1024);
    m.attach_atapi_cdrom_image(iso);
    assert!(matches!(
        m.load_eltorito_to_7c00(),
        Err(MachineError::ElToritoBootImageOob)
    ));
}
