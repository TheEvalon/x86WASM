//! Guest-visible behavior of the CMOS memory-size registers that BIOS POST
//! reads to build its memory map.
//!
//! Spec:
//!
//! - Ralf Brown's Interrupt List, CMOS memory map — `15h`/`16h` IBM "BASE
//!   MEMORY IN KB (low/high byte)", `17h`/`18h` IBM "EXTENDED MEMORY IN KB",
//!   `30h`/`31h` IBM "EXTENDED MEMORY IN KB" (SeeAlso CMOS `17h`), `34h`/`35h`
//!   "EXTENDED MEMORY >16M" whose two bytes "contain the total extended memory
//!   in 64K blocks".
//! - RBIL INT 15h AX=E801h "GET MEMORY SIZE FOR >64M CONFIGURATIONS" — the
//!   split this model reports: "AX = extended memory between 1M and 16M, in K
//!   (max 3C00h = 15MB)" and "BX = extended memory above 16M, in 64K blocks".
//! - IBM PC/AT CMOS map — these are ordinary battery-backed CMOS RAM bytes.
//!
//! Integration tests may only use the crate's re-exported surface, so the CMOS
//! indices are repeated here as local literals with their citation until
//! `devices/src/lib.rs` re-exports the `REG_*_MEM*` constants.

use devices::{CmosRtc, PortDevice, CMOS_DATA, CMOS_INDEX};

/// Spec: RBIL CMOS 15h/16h — base memory in KB, low then high byte.
const REG_BASE_MEM_LOW: u8 = 0x15;
const REG_BASE_MEM_HIGH: u8 = 0x16;
/// Spec: RBIL CMOS 17h/18h — extended memory in KB, low then high byte.
const REG_EXT_MEM_LOW: u8 = 0x17;
const REG_EXT_MEM_HIGH: u8 = 0x18;
/// Spec: RBIL CMOS 30h/31h — extended memory in KB, low then high byte.
const REG_EXT_MEM2_LOW: u8 = 0x30;
const REG_EXT_MEM2_HIGH: u8 = 0x31;
/// Spec: RBIL CMOS 34h/35h — memory above 16 MB in 64 KB blocks.
const REG_MEM_ABOVE_16M_LOW: u8 = 0x34;
const REG_MEM_ABOVE_16M_HIGH: u8 = 0x35;

const KIB: u64 = 1024;
const MIB: u64 = 1024 * 1024;

fn cmos_read(c: &mut CmosRtc, index: u8) -> u8 {
    c.port_write(CMOS_INDEX, 1, u32::from(index));
    c.port_read(CMOS_DATA, 1) as u8
}

fn cmos_read_u16(c: &mut CmosRtc, low: u8, high: u8) -> u16 {
    u16::from_le_bytes([cmos_read(c, low), cmos_read(c, high)])
}

fn cmos_write(c: &mut CmosRtc, index: u8, value: u8) {
    c.port_write(CMOS_INDEX, 1, u32::from(index));
    c.port_write(CMOS_DATA, 1, u32::from(value));
}

/// An unconfigured device reports no memory at all. Reporting a fabricated
/// default would be worse than reporting nothing, because a guest cannot tell
/// the difference between an invented size and a measured one.
#[test]
fn memory_size_registers_are_zero_until_the_host_configures_them() {
    let mut c = CmosRtc::new();
    for index in [
        REG_BASE_MEM_LOW,
        REG_BASE_MEM_HIGH,
        REG_EXT_MEM_LOW,
        REG_EXT_MEM_HIGH,
        REG_EXT_MEM2_LOW,
        REG_EXT_MEM2_HIGH,
        REG_MEM_ABOVE_16M_LOW,
        REG_MEM_ABOVE_16M_HIGH,
    ] {
        assert_eq!(cmos_read(&mut c, index), 0, "index {index:#04x}");
    }
}

/// Spec: RBIL CMOS 15h/16h. Base memory is the 640 KB DOS area, so any machine
/// with at least that much RAM reports exactly 640 KB.
#[test]
fn base_memory_reports_640k_and_clamps() {
    let mut c = CmosRtc::new();

    c.set_memory_size(16 * MIB);
    assert_eq!(
        cmos_read_u16(&mut c, REG_BASE_MEM_LOW, REG_BASE_MEM_HIGH),
        640
    );

    // A machine smaller than the DOS area reports what it actually has.
    c.set_memory_size(512 * KIB);
    assert_eq!(
        cmos_read_u16(&mut c, REG_BASE_MEM_LOW, REG_BASE_MEM_HIGH),
        512
    );
}

/// Spec: RBIL INT 15h AX=E801h — extended memory between 1M and 16M in KB.
/// Both the `17h`/`18h` and `30h`/`31h` pairs carry the same value, as RBIL's
/// SeeAlso between CMOS `30h` and CMOS `17h` implies.
#[test]
fn extended_memory_kb_pairs_agree_and_exclude_the_first_megabyte() {
    let mut c = CmosRtc::new();
    c.set_memory_size(8 * MIB);

    let expected = (8 * 1024) - 1024;
    assert_eq!(
        cmos_read_u16(&mut c, REG_EXT_MEM_LOW, REG_EXT_MEM_HIGH),
        expected
    );
    assert_eq!(
        cmos_read_u16(&mut c, REG_EXT_MEM2_LOW, REG_EXT_MEM2_HIGH),
        expected
    );
}

/// Spec: RBIL INT 15h AX=E801h — "max 3C00h = 15MB" for the KB pair, with
/// everything beyond 16 MB reported through the 64 KB-block pair instead.
#[test]
fn extended_memory_kb_clamps_at_15mb_and_overflow_moves_to_the_64k_pair() {
    let mut c = CmosRtc::new();
    c.set_memory_size(64 * MIB);

    assert_eq!(
        cmos_read_u16(&mut c, REG_EXT_MEM_LOW, REG_EXT_MEM_HIGH),
        0x3C00
    );
    assert_eq!(
        cmos_read_u16(&mut c, REG_EXT_MEM2_LOW, REG_EXT_MEM2_HIGH),
        0x3C00
    );
    // (64 - 16) MB / 64 KB = 768 blocks.
    assert_eq!(
        cmos_read_u16(&mut c, REG_MEM_ABOVE_16M_LOW, REG_MEM_ABOVE_16M_HIGH),
        768
    );
}

/// Below 16 MB there is nothing to report above 16 MB.
#[test]
fn memory_above_16m_is_zero_for_small_machines() {
    let mut c = CmosRtc::new();
    c.set_memory_size(4 * MIB);
    assert_eq!(
        cmos_read_u16(&mut c, REG_MEM_ABOVE_16M_LOW, REG_MEM_ABOVE_16M_HIGH),
        0
    );
    assert_eq!(
        cmos_read_u16(&mut c, REG_EXT_MEM_LOW, REG_EXT_MEM_HIGH),
        3 * 1024
    );
}

/// A machine below 1 MB has no extended memory of any kind.
#[test]
fn sub_megabyte_machine_reports_no_extended_memory() {
    let mut c = CmosRtc::new();
    c.set_memory_size(640 * KIB);
    assert_eq!(
        cmos_read_u16(&mut c, REG_BASE_MEM_LOW, REG_BASE_MEM_HIGH),
        640
    );
    assert_eq!(cmos_read_u16(&mut c, REG_EXT_MEM_LOW, REG_EXT_MEM_HIGH), 0);
    assert_eq!(
        cmos_read_u16(&mut c, REG_MEM_ABOVE_16M_LOW, REG_MEM_ABOVE_16M_HIGH),
        0
    );
}

/// Spec: IBM PC/AT CMOS map — these are battery-backed CMOS RAM bytes, so they
/// survive a device reset exactly like the shutdown-status byte at `0Fh`.
/// Without that, POST would re-read a zeroed memory map after a soft reset.
#[test]
fn memory_size_registers_survive_reset() {
    let mut c = CmosRtc::new();
    c.set_memory_size(32 * MIB);
    let before = [
        cmos_read_u16(&mut c, REG_BASE_MEM_LOW, REG_BASE_MEM_HIGH),
        cmos_read_u16(&mut c, REG_EXT_MEM_LOW, REG_EXT_MEM_HIGH),
        cmos_read_u16(&mut c, REG_EXT_MEM2_LOW, REG_EXT_MEM2_HIGH),
        cmos_read_u16(&mut c, REG_MEM_ABOVE_16M_LOW, REG_MEM_ABOVE_16M_HIGH),
    ];

    c.reset();

    let after = [
        cmos_read_u16(&mut c, REG_BASE_MEM_LOW, REG_BASE_MEM_HIGH),
        cmos_read_u16(&mut c, REG_EXT_MEM_LOW, REG_EXT_MEM_HIGH),
        cmos_read_u16(&mut c, REG_EXT_MEM2_LOW, REG_EXT_MEM2_HIGH),
        cmos_read_u16(&mut c, REG_MEM_ABOVE_16M_LOW, REG_MEM_ABOVE_16M_HIGH),
    ];
    assert_eq!(before, after);
    assert_eq!(before[0], 640);
    assert_eq!(before[3], 256);
}

/// They remain ordinary read/write CMOS RAM: a setup utility overwriting them
/// is not second-guessed by the device, and the write also survives reset.
#[test]
fn memory_size_registers_stay_guest_writable() {
    let mut c = CmosRtc::new();
    c.set_memory_size(16 * MIB);

    cmos_write(&mut c, REG_EXT_MEM_LOW, 0x12);
    cmos_write(&mut c, REG_EXT_MEM_HIGH, 0x34);
    assert_eq!(
        cmos_read_u16(&mut c, REG_EXT_MEM_LOW, REG_EXT_MEM_HIGH),
        0x3412
    );

    c.reset();
    assert_eq!(
        cmos_read_u16(&mut c, REG_EXT_MEM_LOW, REG_EXT_MEM_HIGH),
        0x3412
    );
}
