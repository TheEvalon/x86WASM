//! The floppy, fixed-disk, and boot-option CMOS bytes BIOS POST reads.
//!
//! Spec: Ralf Brown's Interrupt List, CMOS memory map.
//!
//! - `10h` "IBM - FLOPPY DRIVE TYPE": bits 7-4 first floppy disk drive type,
//!   bits 3-0 second (Table C0007). Types (Table C0008): `00h` no drive, `01h`
//!   360 KB 5.25, `02h` 1.2 MB 5.25, `03h` 720 KB 3.5, `04h` 1.44 MB 3.5, `05h`
//!   2.88 MB 3.5, `06h`-`0Fh` unused. RBIL's own example: "With a single 1.44
//!   drive: 40h."
//! - `12h` "IBM - HARD DISK DATA" (Table C0014): bits 7-4 first hard disk
//!   drive, `00` no drive, `01-0Eh` type 1-14, `0Fh` "Hard Disk Type 16-255
//!   (actual Hard Drive Type is in CMOS RAM 19h)"; bits 3-0 second hard disk
//!   drive type, "same as first except extended type will be found in 1Ah".
//! - `19h` "IBM - FIRST EXTENDED HARD DISK DRIVE TYPE" (Table C0020): `00-0Fh`
//!   unused, `10h-FFh` extended type 16-255. "For most manufacturers the last
//!   drive type (typically either 47d or 49d) is 'user defined' and parameters
//!   are stored elsewhere in the CMOS." `1Ah` is the second drive's.
//! - `1Bh`-`23h` "AMI - First Hard Disk (type 47) user defined": cylinders LSB
//!   (`1Bh`), cylinders high byte (`1Ch`), number of heads (`1Dh`), WPC-low
//!   (`1Eh`), WPC-high (`1Fh`), control byte (`20h`, Table C0025), landing zone
//!   low (`21h`), landing zone high (`22h`), sectors per track (`23h`).
//! - `24h`-`2Ch` "AMI - Second Hard Disk user defined": the same nine fields.
//! - `2Dh` "AMI Hi-Flex BIOS - CONFIGURATION OPTIONS" (Table C0032): bit 7
//!   Weitek installed, bit 6 floppy drive seek, bit 5 boot order
//!   (0 = drive C: then A:, 1 = drive A: then C:), bit 4 boot speed, bit 3
//!   external cache, bit 2 internal cache, bit 1 fast gate A20, bit 0 turbo.
//! - `2Eh`/`2Fh`: the additive sum over `10h`-`2Dh`, which every byte above
//!   lands inside.

use devices::CmosRtc;

/// Spec: RBIL CMOS `10h` Table C0007/C0008 — the nibble pair, including RBIL's
/// worked example of `40h` for a single 1.44 MB drive.
#[test]
fn floppy_drive_type_nibbles_follow_table_c0008() {
    let mut cmos = CmosRtc::new();
    assert_eq!(cmos.read_reg(0x10), 0x00, "no drives at power-on");

    cmos.set_floppy_drive_types(CmosRtc::FLOPPY_TYPE_1440K, CmosRtc::FLOPPY_TYPE_NONE);
    assert_eq!(cmos.read_reg(0x10), 0x40);
    assert_eq!(cmos.floppy_drive_type(0), CmosRtc::FLOPPY_TYPE_1440K);
    assert_eq!(cmos.floppy_drive_type(1), CmosRtc::FLOPPY_TYPE_NONE);

    // RBIL: "a PC having a 5 1/4 1.2 Mb A: drive and a 1.44 Mb B: drive will
    // have a value of 24h in byte 10h."
    cmos.set_floppy_drive_types(CmosRtc::FLOPPY_TYPE_1200K, CmosRtc::FLOPPY_TYPE_1440K);
    assert_eq!(cmos.read_reg(0x10), 0x24);
    assert_eq!(cmos.floppy_drive_type(0), CmosRtc::FLOPPY_TYPE_1200K);
    assert_eq!(cmos.floppy_drive_type(1), CmosRtc::FLOPPY_TYPE_1440K);

    cmos.set_floppy_drive_types(CmosRtc::FLOPPY_TYPE_NONE, CmosRtc::FLOPPY_TYPE_NONE);
    assert_eq!(cmos.read_reg(0x10), 0x00);
}

/// Spec: RBIL CMOS `12h` Table C0014 and `19h`/`1Ah` Table C0020 — a drive
/// described by the user-defined type escapes through nibble `0Fh` into the
/// extension byte.
#[test]
fn hard_disk_type_uses_the_extension_escape_for_the_user_defined_type() {
    let mut cmos = CmosRtc::new();
    assert_eq!(cmos.read_reg(0x12), 0x00);
    assert_eq!(cmos.hard_disk_type(0), 0);
    assert_eq!(cmos.hard_disk_type(1), 0);

    cmos.set_hard_disk_user_geometry(0, 1024, 16, 63);
    assert_eq!(cmos.read_reg(0x12) >> 4, 0x0F, "nibble escapes to 19h");
    assert_eq!(
        cmos.read_reg(0x12) & 0x0F,
        0x00,
        "second drive still absent"
    );
    assert_eq!(cmos.read_reg(0x19), CmosRtc::HARD_DISK_TYPE_USER_DEFINED);
    assert_eq!(cmos.read_reg(0x1A), 0x00);
    assert_eq!(cmos.hard_disk_type(0), CmosRtc::HARD_DISK_TYPE_USER_DEFINED);

    cmos.set_hard_disk_user_geometry(1, 200, 4, 17);
    assert_eq!(cmos.read_reg(0x12), 0xFF);
    assert_eq!(cmos.read_reg(0x1A), CmosRtc::HARD_DISK_TYPE_USER_DEFINED);
    assert_eq!(cmos.hard_disk_type(1), CmosRtc::HARD_DISK_TYPE_USER_DEFINED);

    // An absent drive is encoded absent, extension byte and parameters cleared.
    cmos.set_hard_disk_absent(0);
    assert_eq!(cmos.read_reg(0x12), 0x0F);
    assert_eq!(cmos.read_reg(0x19), 0x00);
    assert_eq!(cmos.hard_disk_type(0), 0);
    assert_eq!(cmos.hard_disk_geometry(0), None);
    for index in 0x1Bu8..=0x23 {
        assert_eq!(cmos.read_reg(index), 0x00, "parameter {index:#04X} cleared");
    }
    // The second drive's block is untouched by the first drive's removal.
    assert_eq!(cmos.hard_disk_geometry(1), Some((200, 4, 17)));
}

/// Spec: RBIL CMOS `1Bh`-`23h` (first drive) and `24h`-`2Ch` (second drive) —
/// each field at its documented index, little-endian for the word pairs.
#[test]
fn user_defined_parameter_blocks_land_at_the_documented_indices() {
    let mut cmos = CmosRtc::new();
    cmos.set_hard_disk_user_geometry(0, 0x0410, 16, 63);
    cmos.set_hard_disk_user_geometry(1, 0x0083, 4, 17);

    // First drive: 1Bh/1Ch cylinders, 1Dh heads, 1Eh/1Fh WPC, 20h control,
    // 21h/22h landing zone, 23h sectors per track.
    assert_eq!(cmos.read_reg(0x1B), 0x10);
    assert_eq!(cmos.read_reg(0x1C), 0x04);
    assert_eq!(cmos.read_reg(0x1D), 16);
    assert_eq!(cmos.read_reg(0x1E), 0x00);
    assert_eq!(cmos.read_reg(0x1F), 0x00);
    assert_eq!(
        cmos.read_reg(0x20),
        0x08,
        "Table C0025 bit 3: more than 8 heads"
    );
    assert_eq!(cmos.read_reg(0x21), 0x10);
    assert_eq!(cmos.read_reg(0x22), 0x04);
    assert_eq!(cmos.read_reg(0x23), 63);
    assert_eq!(cmos.hard_disk_geometry(0), Some((0x0410, 16, 63)));

    // Second drive: the same nine fields starting at 24h.
    assert_eq!(cmos.read_reg(0x24), 0x83);
    assert_eq!(cmos.read_reg(0x25), 0x00);
    assert_eq!(cmos.read_reg(0x26), 4);
    assert_eq!(cmos.read_reg(0x27), 0x00);
    assert_eq!(cmos.read_reg(0x28), 0x00);
    assert_eq!(
        cmos.read_reg(0x29),
        0x00,
        "four heads is not more than eight"
    );
    assert_eq!(cmos.read_reg(0x2A), 0x83);
    assert_eq!(cmos.read_reg(0x2B), 0x00);
    assert_eq!(cmos.read_reg(0x2C), 17);
    assert_eq!(cmos.hard_disk_geometry(1), Some((0x0083, 4, 17)));

    // Every parameter byte is inside the checksum range, and none of them
    // reached the checksum bytes themselves.
    assert_eq!(cmos.read_reg(0x2E), 0x00);
    assert_eq!(cmos.read_reg(0x2F), 0x00);
    assert!(!cmos.standard_checksum_valid());
}

/// Spec: RBIL CMOS `2Dh` Table C0032 — bit 5 is the boot order, 0 = C: then A:.
#[test]
fn boot_options_byte_carries_the_boot_order_bit() {
    let mut cmos = CmosRtc::new();
    assert_eq!(cmos.read_reg(0x2D), 0x00);
    assert!(!cmos.boot_floppy_first());

    cmos.set_boot_options(CmosRtc::BOOT_OPTION_FLOPPY_FIRST);
    assert_eq!(cmos.read_reg(0x2D), 0x20);
    assert!(cmos.boot_floppy_first());

    cmos.set_boot_options(0x00);
    assert_eq!(cmos.read_reg(0x2D), 0x00);
    assert!(!cmos.boot_floppy_first());
}

/// The whole point of slice 2: these bytes are inside `10h`-`2Dh`, so once the
/// host recomputes the checksum they stay valid across a reset instead of
/// silently going stale.
#[test]
fn programmed_disk_configuration_survives_reset_with_a_valid_checksum() {
    let mut cmos = CmosRtc::new();
    cmos.set_floppy_drive_types(CmosRtc::FLOPPY_TYPE_1440K, CmosRtc::FLOPPY_TYPE_NONE);
    cmos.set_hard_disk_user_geometry(0, 1024, 16, 63);
    cmos.set_boot_options(CmosRtc::BOOT_OPTION_FLOPPY_FIRST);
    cmos.store_standard_checksum();
    assert!(cmos.standard_checksum_valid());

    cmos.reset();

    assert_eq!(cmos.read_reg(0x10), 0x40);
    assert_eq!(cmos.read_reg(0x12) >> 4, 0x0F);
    assert_eq!(cmos.read_reg(0x19), CmosRtc::HARD_DISK_TYPE_USER_DEFINED);
    assert_eq!(cmos.hard_disk_geometry(0), Some((1024, 16, 63)));
    assert_eq!(cmos.read_reg(0x2D), 0x20);
    assert!(cmos.standard_checksum_valid());
}

/// The CMOS word fields are eight bits wide per byte; a geometry that cannot be
/// described saturates rather than wrapping into a smaller, wrong disk.
#[test]
fn oversized_geometry_saturates_instead_of_wrapping() {
    let mut cmos = CmosRtc::new();
    cmos.set_hard_disk_user_geometry(0, 0xFFFF, 255, 255);
    assert_eq!(cmos.hard_disk_geometry(0), Some((0xFFFF, 255, 255)));
    assert_eq!(cmos.read_reg(0x1B), 0xFF);
    assert_eq!(cmos.read_reg(0x1C), 0xFF);

    // A drive number this byte cannot describe is refused, not aliased.
    cmos.set_hard_disk_user_geometry(2, 100, 2, 17);
    assert_eq!(cmos.hard_disk_geometry(2), None);
    assert_eq!(cmos.read_reg(0x12) & 0x0F, 0x00);
    assert_eq!(cmos.floppy_drive_type(2), CmosRtc::FLOPPY_TYPE_NONE);
}
