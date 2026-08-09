//! CMOS `5Bh`–`5Dh`: memory above 4 GB, in 64 KiB units.
//!
//! Authority: `docs/adr/0006-cmos-above-4gb-memory.md`. These three indices are
//! a **de-facto standard, not silicon-documented behavior**. The MC146818
//! register file ends at `0Dh`; everything above it is general CMOS RAM whose
//! meaning is assigned by whoever wrote the BIOS. Bochs introduced this
//! encoding, QEMU follows it, and SeaBIOS reads exactly these indices — but no
//! chipset or RTC datasheet defines them, and none ever will.
//!
//! The ADR adopts the encoding and requires that its status be stated at the
//! point of use rather than dressed up as a specification. That is why this
//! file says so twice.
//!
//! The encoding: `5Bh` bits 7:0, `5Ch` bits 15:8, `5Dh` bits 23:16 of a count
//! of 64 KiB units of memory above 4 GB.
//!
//! Also asserted here: the split this introduces in the pre-existing registers.
//! `34h`/`35h` (RBIL "EXTENDED MEMORY >16M", 64 KiB blocks) now covers 16 MB to
//! 4 GB and stops there, because everything above 4 GB is reported in
//! `5Bh`–`5Dh` instead. Before this slice it saturated at `FFFFh`, which
//! double-counted the first 64 KiB above 4 GB and then lied about everything
//! past it.

use devices::{
    CmosRtc, PortDevice, CMOS_DATA, CMOS_INDEX, REG_MEM_ABOVE_16M_HIGH, REG_MEM_ABOVE_16M_LOW,
};

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

/// De-facto standard (ADR-0006), not a datasheet: memory above 4 GB in 64 KiB
/// units at `5Bh` (low), `5Ch` (middle), `5Dh` (high).
const REG_ABOVE_4G_LOW: u8 = 0x5B;
const REG_ABOVE_4G_MID: u8 = 0x5C;
const REG_ABOVE_4G_HIGH: u8 = 0x5D;

/// Largest value `34h`/`35h` can now hold: (4 GiB − 16 MiB) / 64 KiB.
const ABOVE_16M_MAX_BLOCKS: u16 = 0xFF00;

fn read_reg(c: &mut CmosRtc, index: u8) -> u8 {
    c.port_write(CMOS_INDEX, 1, u32::from(index));
    c.port_read(CMOS_DATA, 1) as u8
}

fn above_4g_bytes(c: &mut CmosRtc) -> [u8; 3] {
    [
        read_reg(c, REG_ABOVE_4G_LOW),
        read_reg(c, REG_ABOVE_4G_MID),
        read_reg(c, REG_ABOVE_4G_HIGH),
    ]
}

/// A machine at or below 4 GB has nothing to report above it, and the three
/// bytes stay zero — the same answer they gave before this encoding existed.
#[test]
fn machines_at_or_below_four_gigabytes_report_zero_above_four_gigabytes() {
    for ram in [16 * MIB, 512 * MIB, 4 * GIB] {
        let mut c = CmosRtc::new();
        c.set_memory_size(ram);
        assert_eq!(
            above_4g_bytes(&mut c),
            [0, 0, 0],
            "{ram} bytes has nothing above 4 GB"
        );
        assert_eq!(c.memory_above_4g_blocks(), 0);
    }
}

/// De-facto standard (ADR-0006): `5Bh` low, `5Ch` middle, `5Dh` high, counting
/// 64 KiB units above 4 GB.
#[test]
fn above_four_gigabytes_is_a_little_endian_24_bit_count_of_64_kib_units() {
    let mut c = CmosRtc::new();
    // 4 GiB + 256 MiB → 4096 units of 64 KiB.
    c.set_memory_size(4 * GIB + 256 * MIB);
    assert_eq!(c.memory_above_4g_blocks(), 4096);
    assert_eq!(above_4g_bytes(&mut c), [0x00, 0x10, 0x00]);

    // 4 GiB + 64 KiB → exactly one unit, which must land in the low byte.
    c.set_memory_size(4 * GIB + 64 * 1024);
    assert_eq!(c.memory_above_4g_blocks(), 1);
    assert_eq!(above_4g_bytes(&mut c), [0x01, 0x00, 0x00]);

    // A size needing the high byte: 4 GiB + 64 GiB → 0x100000 units.
    c.set_memory_size(4 * GIB + 64 * GIB);
    assert_eq!(c.memory_above_4g_blocks(), 0x0010_0000);
    assert_eq!(above_4g_bytes(&mut c), [0x00, 0x00, 0x10]);
}

/// A partial unit is not reported: the count is whole 64 KiB units, so memory
/// the firmware could not address in a whole unit is dropped rather than
/// rounded up into memory that does not exist.
#[test]
fn a_partial_unit_above_four_gigabytes_is_not_rounded_up() {
    let mut c = CmosRtc::new();
    c.set_memory_size(4 * GIB + 64 * 1024 - 1);
    assert_eq!(c.memory_above_4g_blocks(), 0);

    c.set_memory_size(4 * GIB + 96 * 1024);
    assert_eq!(c.memory_above_4g_blocks(), 1);
}

/// The field is 24 bits wide, so it saturates rather than wrapping into a small
/// value — a wrap would under-report by exactly the amount that matters most.
#[test]
fn above_four_gigabytes_saturates_at_the_width_of_the_field() {
    let mut c = CmosRtc::new();
    c.set_memory_size(u64::MAX);
    assert_eq!(c.memory_above_4g_blocks(), 0x00FF_FFFF);
    assert_eq!(above_4g_bytes(&mut c), [0xFF, 0xFF, 0xFF]);
}

/// Spec: RBIL CMOS `34h`/`35h` "EXTENDED MEMORY >16M". With `5Bh`–`5Dh` in
/// place, this pair covers 16 MB to 4 GB and stops: the ranges must partition
/// the address space rather than overlap, or firmware double-counts.
#[test]
fn the_above_16m_pair_now_stops_at_four_gigabytes() {
    let mut c = CmosRtc::new();

    c.set_memory_size(4 * GIB);
    assert_eq!(c.memory_above_16m_blocks(), ABOVE_16M_MAX_BLOCKS);
    assert_eq!(c.memory_above_4g_blocks(), 0);

    // Above 4 GB the pair holds still and the new bytes take over.
    c.set_memory_size(8 * GIB);
    assert_eq!(c.memory_above_16m_blocks(), ABOVE_16M_MAX_BLOCKS);
    assert_eq!(c.memory_above_4g_blocks(), (4 * GIB / (64 * 1024)) as u32);

    // The two ranges reconstruct the machine's size exactly.
    let below_4g = 16 * MIB + u64::from(c.memory_above_16m_blocks()) * 64 * 1024;
    let above_4g = u64::from(c.memory_above_4g_blocks()) * 64 * 1024;
    assert_eq!(below_4g + above_4g, 8 * GIB);
}

/// Spec: IBM PC/AT — CMOS RAM is battery backed, and a reset must not erase the
/// machine's memory map. The new bytes join the memory-size registers that
/// already survive, for the same reason they do.
#[test]
fn the_above_4gb_bytes_are_battery_backed_like_the_other_memory_size_bytes() {
    let mut c = CmosRtc::new();
    c.set_memory_size(8 * GIB);
    let before = above_4g_bytes(&mut c);

    c.reset();

    assert_eq!(above_4g_bytes(&mut c), before);
    assert_eq!(c.memory_above_4g_blocks(), (4 * GIB / (64 * 1024)) as u32);
    assert_eq!(c.memory_above_16m_blocks(), ABOVE_16M_MAX_BLOCKS);
    for index in [REG_ABOVE_4G_LOW, REG_ABOVE_4G_MID, REG_ABOVE_4G_HIGH] {
        assert!(CmosRtc::is_battery_backed(index), "{index:#04x}");
    }
}

/// They stay ordinary read/write CMOS RAM afterwards: a guest can overwrite
/// them, exactly like `15h`–`35h`.
#[test]
fn the_above_4gb_bytes_stay_ordinary_read_write_cmos_ram() {
    let mut c = CmosRtc::new();
    c.set_memory_size(8 * GIB);

    for (index, value) in [
        (REG_ABOVE_4G_LOW, 0x12u8),
        (REG_ABOVE_4G_MID, 0x34),
        (REG_ABOVE_4G_HIGH, 0x56),
    ] {
        c.port_write(CMOS_INDEX, 1, u32::from(index));
        c.port_write(CMOS_DATA, 1, u32::from(value));
        assert_eq!(read_reg(&mut c, index), value);
    }
    assert_eq!(c.memory_above_4g_blocks(), 0x0056_3412);
}

/// Reconfiguring replaces rather than accumulating, including down to a machine
/// with nothing above 4 GB.
#[test]
fn set_memory_size_replaces_the_above_4gb_bytes() {
    let mut c = CmosRtc::new();
    c.set_memory_size(16 * GIB);
    assert_ne!(c.memory_above_4g_blocks(), 0);

    c.set_memory_size(64 * MIB);
    assert_eq!(c.memory_above_4g_blocks(), 0);
    assert_eq!(above_4g_bytes(&mut c), [0, 0, 0]);
    assert_eq!(
        read_reg(&mut c, REG_MEM_ABOVE_16M_LOW),
        0x00,
        "48 MB above 16 MB = 768 blocks"
    );
    assert_eq!(read_reg(&mut c, REG_MEM_ABOVE_16M_HIGH), 0x03);
}
