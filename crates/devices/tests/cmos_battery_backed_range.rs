//! Durability of the CMOS bytes the standard checksum covers.
//!
//! Spec: IBM PC/AT Technical Reference — CMOS RAM is powered by the system
//! battery, so the configuration POST writes survives a CPU or device reset.
//! Ralf Brown's Interrupt List, CMOS `2Eh`/`2Fh`: "2Eh and 2Fh are as defined
//! by the original IBM PC/AT specification and represent a byte-wise additive
//! sum of the values in locations 10h-2Dh only, 00h-0Fh and 30h-33h are not
//! included."
//!
//! Those two sentences have to hold together. A checksum stored over
//! `10h`-`2Dh` is only meaningful if every byte it covers is as durable as the
//! checksum bytes themselves; if a reset clears part of the range and keeps
//! `2Eh`/`2Fh`, the stored sum silently stops describing the file and POST
//! reads a configuration this machine never had.

use devices::{CmosRtc, CMOS_CHECKSUM_FIRST, CMOS_CHECKSUM_LAST, REG_CHECKSUM_HIGH};

/// A distinct, non-zero value per index so a cleared byte cannot pass by
/// coincidence.
fn marker(index: u8) -> u8 {
    index ^ 0xA5
}

/// Spec: IBM PC/AT battery-backed CMOS + RBIL `2Eh`/`2Fh` — every byte inside
/// the checksum range survives [`CmosRtc::reset`], one index at a time, so a
/// failure names the byte that leaked.
#[test]
fn every_byte_in_the_checksum_range_survives_reset() {
    for index in CMOS_CHECKSUM_FIRST..=CMOS_CHECKSUM_LAST {
        let mut cmos = CmosRtc::new();
        cmos.write_reg(index, marker(index));
        cmos.store_standard_checksum();

        cmos.reset();

        assert_eq!(
            cmos.read_reg(index),
            marker(index),
            "index {index:#04X} is inside the 10h-2Dh checksum range"
        );
        assert!(
            cmos.standard_checksum_valid(),
            "index {index:#04X} left the stored checksum stale"
        );
    }
}

/// The whole range at once, which is what a host that programs floppy, disk,
/// memory, and equipment bytes actually leaves behind.
#[test]
fn a_fully_programmed_checksum_range_round_trips_through_reset() {
    let mut cmos = CmosRtc::new();
    for index in CMOS_CHECKSUM_FIRST..=CMOS_CHECKSUM_LAST {
        cmos.write_reg(index, marker(index));
    }
    cmos.store_standard_checksum();
    let sum = cmos.standard_checksum();
    let stored_high = cmos.read_reg(REG_CHECKSUM_HIGH);

    cmos.reset();
    cmos.reset();

    for index in CMOS_CHECKSUM_FIRST..=CMOS_CHECKSUM_LAST {
        assert_eq!(cmos.read_reg(index), marker(index), "index {index:#04X}");
    }
    assert_eq!(cmos.standard_checksum(), sum);
    assert_eq!(cmos.read_reg(REG_CHECKSUM_HIGH), stored_high);
    assert!(cmos.standard_checksum_valid());
}

/// Staleness stays *detectable*: battery backing removes reset as a cause of a
/// stale checksum, it does not stop a guest from creating one. POST owns the
/// decision, so the device must report the mismatch and nothing else.
#[test]
fn a_guest_write_still_invalidates_the_stored_checksum() {
    let mut cmos = CmosRtc::new();
    cmos.store_standard_checksum();
    assert!(cmos.standard_checksum_valid());

    cmos.write_reg(CMOS_CHECKSUM_LAST, 0x5A);
    assert!(!cmos.standard_checksum_valid());

    // A reset preserves the mismatch rather than hiding or repairing it.
    cmos.reset();
    assert_eq!(cmos.read_reg(CMOS_CHECKSUM_LAST), 0x5A);
    assert!(!cmos.standard_checksum_valid());

    cmos.store_standard_checksum();
    assert!(cmos.standard_checksum_valid());
}

/// Spec: RBIL `2Eh`/`2Fh` — "00h-0Fh and 30h-33h are not included" in the sum.
/// The clock and status registers below `0Eh` are model state that reset
/// returns to its documented default, so they must *not* be battery backed;
/// asserting that keeps the preserved set from quietly growing into the RTC.
#[test]
fn clock_and_status_registers_are_not_battery_backed() {
    let mut cmos = CmosRtc::new();
    for index in 0x00u8..=0x0D {
        cmos.write_reg(index, marker(index));
    }

    cmos.reset();

    for index in 0x00u8..=0x09 {
        assert_eq!(
            cmos.read_reg(index),
            0x00,
            "time/calendar index {index:#04X} must return to reset state"
        );
    }
    // Status A-D come back as their documented power-on defaults.
    assert_ne!(cmos.read_reg(0x0A), marker(0x0A));
    assert_ne!(cmos.read_reg(0x0B), marker(0x0B));
}

/// The two indices POST reads before it trusts anything else — the diagnostic
/// status byte `0Eh` and the shutdown code `0Fh` — were already durable and
/// stay that way.
///
/// Spec: RBIL CMOS `0Eh` (Table C0005 diagnostic status byte) and `0Fh`
/// (shutdown status / reset code).
#[test]
fn diagnostic_and_shutdown_bytes_stay_durable() {
    let mut cmos = CmosRtc::new();
    cmos.write_reg(0x0E, 0x40);
    cmos.write_reg(0x0F, 0x0A);

    cmos.reset();

    assert_eq!(cmos.read_reg(0x0E), 0x40);
    assert_eq!(cmos.read_reg(0x0F), 0x0A);
    assert_eq!(cmos.shutdown_status(), 0x0A);
}
