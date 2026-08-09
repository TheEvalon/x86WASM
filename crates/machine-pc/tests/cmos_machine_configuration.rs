//! The machine fills in the disk and floppy CMOS bytes from what it actually
//! has attached.
//!
//! `devices::CmosRtc` owns the RBIL register layout and knows nothing about the
//! machine; `machine_pc::Machine` knows what media is attached and nothing about
//! CMOS indices. `Machine::sync_firmware_configuration` is where they meet, and
//! this checks it describes the real configuration rather than a plausible one.
//!
//! Spec: Ralf Brown's Interrupt List CMOS map — `10h` floppy drive type
//! (Table C0007/C0008), `12h` hard disk data (Table C0014), `19h`/`1Ah`
//! extended type (Table C0020), `1Bh`-`23h` / `24h`-`2Ch` user-defined
//! parameter blocks, `2Dh` configuration options (Table C0032), `2Eh`/`2Fh`
//! standard checksum over `10h`-`2Dh`.

use devices::{CmosRtc, FDC_1440_IMAGE_SIZE};
use machine_pc::Machine;

const SECTOR: usize = 512;
/// 16 heads × 63 sectors is the geometry IDENTIFY already reports in its
/// obsolete CHS words, so the CMOS block has to agree with it.
const TRACK_SECTORS: usize = 16 * 63;

fn disk_image(cylinders: usize) -> Vec<u8> {
    vec![0u8; cylinders * TRACK_SECTORS * SECTOR]
}

/// A machine with nothing attached says so: both floppy nibbles and both hard
/// disk nibbles read "no drive", and the checksum still validates.
#[test]
fn a_bare_machine_reports_no_drives() {
    let m = Machine::new(4 * 1024 * 1024);

    assert_eq!(m.cmos.read_reg(0x10), 0x00, "10h floppy drive type");
    assert_eq!(m.cmos.read_reg(0x12), 0x00, "12h hard disk data");
    assert_eq!(m.cmos.read_reg(0x19), 0x00);
    assert_eq!(m.cmos.read_reg(0x1A), 0x00);
    assert_eq!(m.cmos.hard_disk_geometry(0), None);
    assert_eq!(m.cmos.hard_disk_geometry(1), None);
    assert!(m.cmos.standard_checksum_valid());
}

/// Spec: RBIL CMOS `10h` — "With a single 1.44 drive: 40h". The floppy nibble
/// and the equipment byte have to describe the same machine.
#[test]
fn attaching_a_floppy_sets_the_1440k_nibble_and_keeps_the_checksum_valid() {
    let mut m = Machine::new(4 * 1024 * 1024);
    m.attach_floppy_image(vec![0u8; FDC_1440_IMAGE_SIZE])
        .expect("exact 1.44MB image");

    assert_eq!(m.cmos.read_reg(0x10), 0x40);
    assert_eq!(m.cmos.floppy_drive_type(0), CmosRtc::FLOPPY_TYPE_1440K);
    assert_eq!(m.cmos.floppy_drive_type(1), CmosRtc::FLOPPY_TYPE_NONE);
    assert_eq!(m.cmos.equipment_byte() & 0x01, 0x01, "equipment 14h bit 0");
    assert!(m.cmos.standard_checksum_valid());
}

/// Spec: RBIL CMOS `12h` Table C0014 / `19h` Table C0020 / `1Bh`-`23h` — an
/// attached IDE image is described as the user-defined type with a geometry
/// derived from its size.
#[test]
fn attaching_an_ide_image_fills_the_user_defined_parameter_block() {
    let mut m = Machine::new(4 * 1024 * 1024);
    m.attach_ide_image(disk_image(200));

    assert_eq!(m.cmos.read_reg(0x12) >> 4, 0x0F, "12h escapes to 19h");
    assert_eq!(m.cmos.read_reg(0x12) & 0x0F, 0x00, "no second fixed disk");
    assert_eq!(
        m.cmos.read_reg(0x19),
        CmosRtc::HARD_DISK_TYPE_USER_DEFINED,
        "19h extended type"
    );
    assert_eq!(m.cmos.hard_disk_geometry(0), Some((200, 16, 63)));
    assert_eq!(m.cmos.read_reg(0x1B), 200, "1Bh cylinders LSB");
    assert_eq!(m.cmos.read_reg(0x1C), 0, "1Ch cylinders high byte");
    assert_eq!(m.cmos.read_reg(0x1D), 16, "1Dh heads");
    assert_eq!(m.cmos.read_reg(0x23), 63, "23h sectors per track");
    assert!(m.cmos.standard_checksum_valid());

    // The CMOS geometry and `Machine::ide_chs_geometry` agree by construction.
    assert_eq!(m.ide_chs_geometry(), Some((200, 16, 63)));
}

/// A disk smaller than one cylinder still has to be describable: rounding to
/// zero cylinders would encode a disk with no sectors at all.
#[test]
fn a_disk_smaller_than_one_cylinder_reports_one_cylinder() {
    let mut m = Machine::new(4 * 1024 * 1024);
    m.attach_ide_image(vec![0u8; 16 * SECTOR]);
    assert_eq!(m.cmos.hard_disk_geometry(0), Some((1, 16, 63)));
}

/// Spec: IBM PC/AT battery-backed CMOS — the configuration a host attached
/// survives a machine reset, checksum included.
#[test]
fn disk_configuration_survives_machine_reset() {
    let mut m = Machine::new(4 * 1024 * 1024);
    m.attach_floppy_image(vec![0u8; FDC_1440_IMAGE_SIZE])
        .expect("exact 1.44MB image");
    m.attach_ide_image(disk_image(64));

    m.reset();

    assert_eq!(m.cmos.read_reg(0x10), 0x40);
    assert_eq!(m.cmos.hard_disk_geometry(0), Some((64, 16, 63)));
    assert!(m.cmos.standard_checksum_valid());
}

/// Spec: RBIL CMOS `2Dh` Table C0032 bit 5 — boot order. The machine states the
/// order it can actually satisfy: a floppy-only machine boots A: first.
#[test]
fn boot_order_byte_follows_the_attached_media() {
    let mut m = Machine::new(4 * 1024 * 1024);
    assert!(!m.cmos.boot_floppy_first(), "no media: C: then A:");

    m.attach_floppy_image(vec![0u8; FDC_1440_IMAGE_SIZE])
        .expect("exact 1.44MB image");
    assert!(m.cmos.boot_floppy_first(), "floppy only: A: then C:");

    m.attach_ide_image(disk_image(16));
    assert!(
        !m.cmos.boot_floppy_first(),
        "fixed disk present: C: then A:"
    );
    assert!(m.cmos.standard_checksum_valid());
}
