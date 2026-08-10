//! Host-side El Torito detection via attached ATAPI CD-ROM image.
//!
//! # Spec refs
//!
//! - "El Torito" Bootable CD-ROM Format Specification Version 1.0 — Boot Record
//!   Volume Descriptor, Validation Entry key bytes `55h`/`AAh`, Initial/Default
//!   Entry boot indicator `88h`.
//! - Round-6 ATAPI medium model — image bytes exposed for host inspection only;
//!   no INT 13h CD emulation / boot-image handoff.

use firmware_interface::{
    ElToritoError, EL_TORITO_BOOTABLE, EL_TORITO_BOOT_SYSTEM_ID, EL_TORITO_KEY_55,
    EL_TORITO_KEY_AA, EL_TORITO_PLATFORM_X86, EL_TORITO_SECTOR_BYTES,
    EL_TORITO_VALIDATION_HEADER_ID, ISO9660_STANDARD_ID, ISO9660_VD_BOOT_RECORD,
    ISO9660_VD_TERMINATOR,
};
use machine_pc::Machine;

fn blank_iso(sectors: usize) -> Vec<u8> {
    vec![0u8; sectors * EL_TORITO_SECTOR_BYTES]
}

fn write_sector(img: &mut [u8], lba: u32, data: &[u8]) {
    let start = lba as usize * EL_TORITO_SECTOR_BYTES;
    img[start..start + data.len()].copy_from_slice(data);
}

fn make_bootable_iso() -> Vec<u8> {
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
    cat[38..40].copy_from_slice(&1u16.to_le_bytes());
    cat[40..44].copy_from_slice(&25u32.to_le_bytes());
    write_sector(&mut img, catalog_lba, &cat);
    img
}

#[test]
fn machine_reports_bootable_el_torito_from_atapi_image() {
    let mut m = Machine::new(16 * 1024 * 1024);
    m.attach_atapi_cdrom_image(make_bootable_iso());
    let info = m.inspect_atapi_el_torito().expect("El Torito present");
    assert_eq!(info.platform_id, EL_TORITO_PLATFORM_X86);
    assert!(info.bootable);
    assert_eq!(info.load_rba, 25);
}

#[test]
fn empty_atapi_tray_rejects_el_torito_inspect() {
    let mut m = Machine::new(16 * 1024 * 1024);
    m.ide.attach_atapi_cdrom();
    assert_eq!(m.inspect_atapi_el_torito(), Err(ElToritoError::Truncated));
}
