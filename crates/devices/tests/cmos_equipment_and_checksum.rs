//! Guest-visible behavior of the CMOS configuration bytes BIOS POST validates
//! before it trusts the rest of the register file: the diagnostic status byte,
//! the equipment byte, and the standard CMOS checksum.
//!
//! Spec:
//!
//! - RBIL CMOS `0Eh` — IBM PS/2 "DIAGNOSTIC STATUS BYTE", Table C0005: bit 7
//!   clock lost power, bit 6 incorrect checksum, bit 5 equipment configuration
//!   incorrect, bit 4 error in memory size, bit 3 controller or disk drive
//!   failed initialization, bit 2 time invalid, bit 1 installed adaptors do not
//!   match configuration, bit 0 time-out while reading adaptor ID.
//! - RBIL CMOS `14h` — IBM "EQUIPMENT BYTE", Table C0019: bits 7-6 number of
//!   floppy drives, bits 5-4 monitor type, bit 3 display enabled, bit 2
//!   keyboard enabled, bit 1 math coprocessor installed, bit 0 floppy drive
//!   installed.
//! - RBIL CMOS `2Eh`/`2Fh` — IBM "Standard CMOS Checksum, High/Low Byte": "as
//!   defined by the original IBM PC/AT specification and represent a byte-wise
//!   additive sum of the values in locations 10h-2Dh only, 00h-0Fh and 30h-33h
//!   are not included."
//!
//! Integration tests may only use the crate's re-exported surface, so the CMOS
//! indices and bit names are repeated here as local literals with their
//! citation until `devices/src/lib.rs` re-exports the `REG_DIAGNOSTIC` /
//! `REG_EQUIPMENT` / `REG_CHECKSUM_*` / `DIAG_*` / `EQUIP_*` items.

use devices::{CmosRtc, PortDevice, CMOS_DATA, CMOS_INDEX};

/// Spec: RBIL CMOS 0Eh — diagnostic status byte.
const REG_DIAGNOSTIC: u8 = 0x0E;
/// Spec: RBIL CMOS 14h — equipment byte.
const REG_EQUIPMENT: u8 = 0x14;
/// Spec: RBIL CMOS 2Eh/2Fh — standard checksum, high then low byte.
const REG_CHECKSUM_HIGH: u8 = 0x2E;
const REG_CHECKSUM_LOW: u8 = 0x2F;
/// Spec: RBIL CMOS 2Fh note — the summed range is 10h through 2Dh inclusive.
const CHECKSUM_FIRST: u8 = 0x10;
const CHECKSUM_LAST: u8 = 0x2D;

/// Spec: RBIL Table C0005 bit 6 — incorrect checksum.
const DIAG_BAD_CHECKSUM: u8 = 1 << 6;
/// Spec: RBIL Table C0019 — monitor type 00b, observed for EGA and VGA.
const EQUIP_DISPLAY_EGA_VGA: u8 = 0x00;
/// Spec: RBIL Table C0019 bit 3 / bit 2 / bit 0.
const EQUIP_DISPLAY_ENABLED: u8 = 1 << 3;
const EQUIP_KEYBOARD_ENABLED: u8 = 1 << 2;
const EQUIP_FLOPPY_INSTALLED: u8 = 1 << 0;

fn cmos_read(c: &mut CmosRtc, index: u8) -> u8 {
    c.port_write(CMOS_INDEX, 1, u32::from(index));
    c.port_read(CMOS_DATA, 1) as u8
}

fn cmos_write(c: &mut CmosRtc, index: u8, value: u8) {
    c.port_write(CMOS_INDEX, 1, u32::from(index));
    c.port_write(CMOS_DATA, 1, u32::from(value));
}

/// Nothing is configured until the host says so; the device does not invent a
/// machine description.
#[test]
fn configuration_bytes_start_clear() {
    let mut c = CmosRtc::new();
    assert_eq!(cmos_read(&mut c, REG_DIAGNOSTIC), 0);
    assert_eq!(cmos_read(&mut c, REG_EQUIPMENT), 0);
    assert_eq!(cmos_read(&mut c, REG_CHECKSUM_HIGH), 0);
    assert_eq!(cmos_read(&mut c, REG_CHECKSUM_LOW), 0);
}

/// Spec: RBIL CMOS 14h Table C0019 — the equipment byte is ordinary CMOS RAM
/// whose fields describe the installed hardware.
#[test]
fn equipment_byte_stores_and_reads_back() {
    let mut c = CmosRtc::new();
    // One floppy drive (bits 7-6 = 00b), VGA (bits 5-4 = 00b), display and
    // keyboard enabled, no coprocessor, floppy installed.
    let value = CmosRtc::equipment_floppy_field(1)
        | EQUIP_DISPLAY_EGA_VGA
        | EQUIP_DISPLAY_ENABLED
        | EQUIP_KEYBOARD_ENABLED;
    c.set_equipment_byte(value);

    assert_eq!(cmos_read(&mut c, REG_EQUIPMENT), value);
    assert_eq!(c.equipment_byte(), value);
    assert_eq!(value & EQUIP_FLOPPY_INSTALLED, EQUIP_FLOPPY_INSTALLED);
}

/// Spec: RBIL Table C0019 — bits 7-6 encode "00b 1 Drive", "01b 2 Drives",
/// and so on, alongside bit 0 "floppy drive installed".
#[test]
fn equipment_floppy_field_encodes_drive_count() {
    assert_eq!(CmosRtc::equipment_floppy_field(0), 0x00);
    assert_eq!(CmosRtc::equipment_floppy_field(1), 0x01);
    assert_eq!(CmosRtc::equipment_floppy_field(2), 0x41);
    assert_eq!(CmosRtc::equipment_floppy_field(3), 0x81);
    assert_eq!(CmosRtc::equipment_floppy_field(4), 0xC1);
    // More drives than the two-bit field can describe saturates at four.
    assert_eq!(CmosRtc::equipment_floppy_field(9), 0xC1);
}

/// Spec: RBIL CMOS 2Fh — "a byte-wise additive sum of the values in locations
/// 10h-2Dh only", stored high byte at `2Eh` and low byte at `2Fh`.
#[test]
fn standard_checksum_sums_10h_to_2dh_and_stores_big_endian() {
    let mut c = CmosRtc::new();
    for index in CHECKSUM_FIRST..=CHECKSUM_LAST {
        cmos_write(&mut c, index, 0xFF);
    }
    // 0x2D - 0x10 + 1 = 30 bytes of 0xFF.
    let expected = 30u16 * 0xFF;
    assert_eq!(c.standard_checksum(), expected);

    c.store_standard_checksum();
    assert_eq!(cmos_read(&mut c, REG_CHECKSUM_HIGH), (expected >> 8) as u8);
    assert_eq!(cmos_read(&mut c, REG_CHECKSUM_LOW), expected as u8);
    assert!(c.standard_checksum_valid());
}

/// The summed range excludes `00h`-`0Fh` and everything from `2Eh` up, so the
/// clock, the shutdown byte, and the checksum bytes themselves do not feed it.
#[test]
fn standard_checksum_excludes_the_documented_ranges() {
    let mut c = CmosRtc::new();
    c.store_standard_checksum();
    let baseline = c.standard_checksum();

    for outside in [0x00u8, 0x0E, 0x0F, 0x2E, 0x2F, 0x30, 0x33, 0x35] {
        cmos_write(&mut c, outside, 0x5A);
        assert_eq!(
            c.standard_checksum(),
            baseline,
            "index {outside:#04x} must not contribute"
        );
    }

    cmos_write(&mut c, CHECKSUM_FIRST, 0x01);
    assert_eq!(c.standard_checksum(), baseline + 1);
    cmos_write(&mut c, CHECKSUM_LAST, 0x02);
    assert_eq!(c.standard_checksum(), baseline + 3);
}

/// A guest that rewrites a covered byte without recomputing the checksum makes
/// it stale — which is exactly the condition POST detects.
#[test]
fn checksum_goes_stale_when_a_covered_byte_changes() {
    let mut c = CmosRtc::new();
    c.set_equipment_byte(0x21);
    c.store_standard_checksum();
    assert!(c.standard_checksum_valid());

    cmos_write(&mut c, REG_EQUIPMENT, 0x61);
    assert!(!c.standard_checksum_valid());

    c.store_standard_checksum();
    assert!(c.standard_checksum_valid());
}

/// Spec: RBIL CMOS 0Eh Table C0005 — the diagnostic byte is ordinary CMOS RAM
/// that POST reads and writes; the device does not evaluate it.
#[test]
fn diagnostic_status_is_plain_storage() {
    let mut c = CmosRtc::new();
    c.set_diagnostic_status(DIAG_BAD_CHECKSUM);
    assert_eq!(cmos_read(&mut c, REG_DIAGNOSTIC), DIAG_BAD_CHECKSUM);
    assert_eq!(c.diagnostic_status(), DIAG_BAD_CHECKSUM);

    // Storing a good checksum does not clear the byte: acting on it is POST's
    // job, not the RTC's.
    c.store_standard_checksum();
    assert!(c.standard_checksum_valid());
    assert_eq!(c.diagnostic_status(), DIAG_BAD_CHECKSUM);

    cmos_write(&mut c, REG_DIAGNOSTIC, 0x00);
    assert_eq!(c.diagnostic_status(), 0);
}

/// Spec: IBM PC/AT — these are battery-backed configuration bytes, so a reset
/// must not invalidate the checksum it just preserved.
#[test]
fn configuration_bytes_and_checksum_survive_reset() {
    let mut c = CmosRtc::new();
    c.set_memory_size(16 * 1024 * 1024);
    c.set_equipment_byte(
        CmosRtc::equipment_floppy_field(1) | EQUIP_DISPLAY_ENABLED | EQUIP_KEYBOARD_ENABLED,
    );
    c.set_diagnostic_status(0);
    c.store_standard_checksum();
    let equipment = c.equipment_byte();
    let checksum = c.standard_checksum();

    c.reset();

    assert_eq!(c.equipment_byte(), equipment);
    assert_eq!(c.diagnostic_status(), 0);
    assert_eq!(c.standard_checksum(), checksum);
    assert!(
        c.standard_checksum_valid(),
        "a preserved checksum must still validate after reset"
    );
}
