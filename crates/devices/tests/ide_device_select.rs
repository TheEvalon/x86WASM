//! ATA device selection (DEV bit) with only Device 0 attached.
//!
//! # Spec refs
//!
//! - ATA/ATAPI-6 (T13/1410D r3b) §9.16.1 "Device 0 only configurations":
//!   with Device 1 selected and no Device 1 present, Device 0 shall
//!   1) complete a Device Control register write as if Device 0 was selected,
//!   2) complete a Command Block register write other than Command as if
//!      Device 0 was selected,
//!   3) **ignore** a write to the Command register except EXECUTE DEVICE
//!      DIAGNOSTIC,
//!   4) complete Control/Command Block reads other than Status/Alternate
//!      Status as if Device 0 was selected, and return `00h` for Status and
//!      Alternate Status (non-PACKET device).
//! - ATA/ATAPI-6 Table 18 "Device 1 is selected and Device 0 is responding for
//!   Device 1" — a Device register read returns the Device 0 Device register
//!   with the DEV bit set to one; a Command/Status register read returns `00h`.
//! - ATA/ATAPI-6 §5.2.9 INTRQ — "When the nIEN bit is set to one or the device
//!   is not selected, the INTRQ signal shall be released"; deselecting via the
//!   Device register releases INTRQ while interrupt pending stays set, and
//!   reselecting asserts it again.
//! - ATA/ATAPI-6 §9.12 Signature and persistence — a non-PACKET device places
//!   Sector Count `01h`, LBA Low `01h`, LBA Mid `00h`, LBA High `00h` after
//!   power-on/hardware/software reset and EXECUTE DEVICE DIAGNOSTIC.
//! - ATA/ATAPI-6 §8.11 EXECUTE DEVICE DIAGNOSTIC / Table 26 — diagnostic code
//!   `01h`; note 2: "If Device 1 is not present, the host may see the
//!   information from Device 0 even though Device 1 is selected."

use devices::{
    IdePrimary, IdeSecondary, PortDevice, ATA_CMD_IDENTIFY, ATA_CMD_READ_SECTORS, ATA_DC_NIEN,
    ATA_DC_SRST, ATA_DRIVE_LBA, ATA_DRIVE_SLAVE, ATA_ER_ABRT, ATA_SR_DRDY, ATA_SR_DRQ, ATA_SR_DSC,
    ATA_SR_ERR, IDE_PRIMARY_CTRL, IDE_PRIMARY_DATA, IDE_PRIMARY_DRIVE, IDE_PRIMARY_ERROR,
    IDE_PRIMARY_LBA_HI, IDE_PRIMARY_LBA_LO, IDE_PRIMARY_LBA_MID, IDE_PRIMARY_SECCOUNT,
    IDE_PRIMARY_STATUS, IDE_SECONDARY_CTRL, IDE_SECONDARY_DRIVE, IDE_SECONDARY_STATUS,
};

/// Device 0 selection value (DEV=0); bits 7/5 are the obsolete ATA-1..5 ones.
const DEV0: u32 = 0xA0;
/// Device 1 selection value (DEV=1).
const DEV1: u32 = 0xA0 | ATA_DRIVE_SLAVE as u32;
/// SMART (`0xB0`) — an ABRT-only opcode in this tree (not re-exported).
const CMD_SMART: u32 = 0xB0;
/// EXECUTE DEVICE DIAGNOSTIC (`0x90`). Spec: ATA/ATAPI-6 §8.11.
const CMD_DIAGNOSTIC: u32 = 0x90;
/// Diagnostic code "Device 0 passed". Spec: ATA/ATAPI-6 Table 26.
const DIAG_PASSED: u8 = 0x01;

fn ready_master() -> IdePrimary {
    let mut ide = IdePrimary::with_image(vec![0u8; 512 * 4]);
    // Clear nIEN so INTRQ is driven.
    ide.port_write(IDE_PRIMARY_CTRL, 1, 0);
    ide
}

/// §9.16.1(3): a Command register write with Device 1 selected is ignored, so
/// Device 0 keeps its ready state instead of being zeroed out. SeaBIOS relies
/// on Device 0 still answering after it probes Device 1.
#[test]
fn command_write_to_absent_device1_is_ignored() {
    let mut ide = ready_master();
    ide.port_write(IDE_PRIMARY_DRIVE, 1, DEV1);
    ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY));

    // Status/Alt Status read 00h while Device 1 is selected (§9.16.1(4)).
    assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
    assert_eq!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8, 0);

    // Device 0 is untouched: no DRQ, no ERR, still ready.
    ide.port_write(IDE_PRIMARY_DRIVE, 1, DEV0);
    assert_eq!(
        ide.port_read(IDE_PRIMARY_CTRL, 1) as u8,
        ATA_SR_DRDY | ATA_SR_DSC
    );
    assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
    assert!(!ide.irq_line());
}

/// §9.16.1(3): unsupported opcodes addressed to the absent Device 1 must not
/// abort on Device 0 (no ERR/ABRT, no INTRQ).
#[test]
fn unsupported_command_to_absent_device1_does_not_abort_device0() {
    let mut ide = ready_master();
    ide.port_write(IDE_PRIMARY_DRIVE, 1, DEV1);
    ide.port_write(IDE_PRIMARY_STATUS, 1, CMD_SMART);
    assert!(!ide.irq_line());

    ide.port_write(IDE_PRIMARY_DRIVE, 1, DEV0);
    let st = ide.port_read(IDE_PRIMARY_CTRL, 1) as u8;
    assert_eq!(st & ATA_SR_ERR, 0);
    assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);

    // Sanity: the same opcode on Device 0 still aborts honestly.
    ide.port_write(IDE_PRIMARY_STATUS, 1, CMD_SMART);
    assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
    assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_ER_ABRT);
}

/// §9.16.1(3) exception + §8.11 note 2 + §9.12: EXECUTE DEVICE DIAGNOSTIC is
/// the one command Device 0 executes while Device 1 is selected; it reports
/// diagnostic code `01h` and writes the non-PACKET signature.
#[test]
fn execute_device_diagnostic_runs_while_device1_selected() {
    let mut ide = ready_master();
    // Dirty the task file so the signature write is observable.
    ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 0x5A);
    ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0x5A);
    ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0x5A);
    ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0x5A);

    ide.port_write(IDE_PRIMARY_DRIVE, 1, DEV1);
    ide.port_write(IDE_PRIMARY_STATUS, 1, CMD_DIAGNOSTIC);

    // Reads other than Status complete as if Device 0 was selected (§9.16.1(4)).
    assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, DIAG_PASSED);
    assert_eq!(ide.port_read(IDE_PRIMARY_SECCOUNT, 1) as u8, 0x01);
    assert_eq!(ide.port_read(IDE_PRIMARY_LBA_LO, 1) as u8, 0x01);
    assert_eq!(ide.port_read(IDE_PRIMARY_LBA_MID, 1) as u8, 0x00);
    assert_eq!(ide.port_read(IDE_PRIMARY_LBA_HI, 1) as u8, 0x00);
    // Status still reads 00h while Device 1 is selected.
    assert_eq!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8, 0);

    ide.port_write(IDE_PRIMARY_DRIVE, 1, DEV0);
    assert_eq!(
        ide.port_read(IDE_PRIMARY_CTRL, 1) as u8,
        ATA_SR_DRDY | ATA_SR_DSC
    );
}

/// §9.12: a software reset (Device Control SRST) also writes the signature,
/// and §9.16.1(1) makes that write complete as if Device 0 was selected.
#[test]
fn device_control_srst_completes_while_device1_selected() {
    let mut ide = ready_master();
    ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 0x33);
    ide.port_write(IDE_PRIMARY_DRIVE, 1, DEV1);

    ide.port_write(IDE_PRIMARY_CTRL, 1, u32::from(ATA_DC_SRST));
    ide.port_write(IDE_PRIMARY_CTRL, 1, 0);

    ide.port_write(IDE_PRIMARY_DRIVE, 1, DEV0);
    assert_eq!(
        ide.port_read(IDE_PRIMARY_CTRL, 1) as u8,
        ATA_SR_DRDY | ATA_SR_DSC
    );
    assert_eq!(ide.port_read(IDE_PRIMARY_SECCOUNT, 1) as u8, 0x01);
    assert_eq!(ide.port_read(IDE_PRIMARY_LBA_LO, 1) as u8, 0x01);
}

/// Table 18: with Device 1 selected, non-status Command Block reads return the
/// Device 0 content, and the Device register reads back with DEV set to one.
#[test]
fn device1_reads_return_device0_content_except_status() {
    let mut ide = ready_master();
    ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 0x12);
    ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0x34);
    ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0x56);
    ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0x78);

    ide.port_write(IDE_PRIMARY_DRIVE, 1, DEV1);
    assert_eq!(ide.port_read(IDE_PRIMARY_SECCOUNT, 1) as u8, 0x12);
    assert_eq!(ide.port_read(IDE_PRIMARY_LBA_LO, 1) as u8, 0x34);
    assert_eq!(ide.port_read(IDE_PRIMARY_LBA_MID, 1) as u8, 0x56);
    assert_eq!(ide.port_read(IDE_PRIMARY_LBA_HI, 1) as u8, 0x78);
    assert_ne!(
        ide.port_read(IDE_PRIMARY_DRIVE, 1) as u8 & ATA_DRIVE_SLAVE,
        0
    );
    assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
    assert_eq!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8, 0);

    // §9.16.1(2): a Command Block write with Device 1 selected lands in Device 0.
    ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 0x9A);
    ide.port_write(IDE_PRIMARY_DRIVE, 1, DEV0);
    assert_eq!(ide.port_read(IDE_PRIMARY_SECCOUNT, 1) as u8, 0x9A);
}

/// §5.2.9: deselecting Device 0 releases INTRQ without clearing interrupt
/// pending; reselecting Device 0 asserts INTRQ again.
#[test]
fn selecting_device1_releases_intrq_and_reselect_reasserts() {
    let mut ide = ready_master();
    ide.port_write(IDE_PRIMARY_DRIVE, 1, DEV0);
    ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY));
    assert!(ide.irq_line(), "IDENTIFY DRQ asserts INTRQ when nIEN=0");

    ide.port_write(IDE_PRIMARY_DRIVE, 1, DEV1);
    assert!(!ide.irq_line(), "deselected device releases INTRQ");
    // Reading the (00h) Status register of the absent device must not clear
    // Device 0 interrupt pending.
    assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);

    ide.port_write(IDE_PRIMARY_DRIVE, 1, DEV0);
    assert!(ide.irq_line(), "reselect reasserts pending INTRQ");
    let _ = ide.port_read(IDE_PRIMARY_STATUS, 1);
    assert!(!ide.irq_line(), "Status read clears interrupt pending");
}

/// Data port cycles addressed to the absent Device 1 must not consume Device 0
/// DRQ data. Table 18 only covers BSY=0/DRQ=0, so this tree documents the
/// safe model: the cycle is ignored and the Device 0 PIO stream is preserved.
#[test]
fn device1_data_port_access_does_not_consume_device0_pio() {
    let mut ide = ready_master();
    ide.port_write(IDE_PRIMARY_DRIVE, 1, DEV0);
    ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY));
    assert_ne!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);
    let word0 = ide.port_read(IDE_PRIMARY_DATA, 2) as u16;
    assert_eq!(word0, 0x0040);

    ide.port_write(IDE_PRIMARY_DRIVE, 1, DEV1);
    assert_eq!(ide.port_read(IDE_PRIMARY_DATA, 2), 0xFFFF_FFFF);
    ide.port_write(IDE_PRIMARY_DATA, 2, 0xDEAD);

    ide.port_write(IDE_PRIMARY_DRIVE, 1, DEV0);
    // Word 1 of IDENTIFY is the obsolete cylinder count (16383).
    assert_eq!(ide.port_read(IDE_PRIMARY_DATA, 2) as u16, 16383);
}

/// A READ SECTORS aimed at the absent Device 1 must not start a transfer and
/// must not disturb the Device 0 task file / status.
#[test]
fn read_sectors_to_absent_device1_starts_no_transfer() {
    let mut img = vec![0u8; 512 * 2];
    img[0] = 0xDE;
    img[1] = 0xAD;
    let mut ide = IdePrimary::with_image(img);
    ide.port_write(IDE_PRIMARY_CTRL, 1, 0);

    ide.port_write(IDE_PRIMARY_DRIVE, 1, DEV1 | u32::from(ATA_DRIVE_LBA));
    ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 1);
    ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0);
    ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0);
    ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0);
    ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_SECTORS));
    assert!(!ide.irq_line());

    ide.port_write(IDE_PRIMARY_DRIVE, 1, DEV0 | u32::from(ATA_DRIVE_LBA));
    let st = ide.port_read(IDE_PRIMARY_CTRL, 1) as u8;
    assert_eq!(st & ATA_SR_DRQ, 0, "no DRQ from an ignored command");
    assert_eq!(st & ATA_SR_ERR, 0);
    // The same command on Device 0 works.
    ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_SECTORS));
    assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
    assert_eq!(ide.port_read(IDE_PRIMARY_DATA, 2) as u16, 0xADDE);
}

/// nIEN written with Device 1 selected still gates Device 0 INTRQ (§9.16.1(1)).
#[test]
fn nien_write_while_device1_selected_gates_device0_intrq() {
    let mut ide = ready_master();
    ide.port_write(IDE_PRIMARY_DRIVE, 1, DEV1);
    ide.port_write(IDE_PRIMARY_CTRL, 1, u32::from(ATA_DC_NIEN));
    ide.port_write(IDE_PRIMARY_DRIVE, 1, DEV0);
    ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY));
    assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
    assert!(!ide.irq_line(), "nIEN set while DEV=1 still masks INTRQ");
}

/// The secondary channel shares the same register file and therefore the same
/// §9.16.1 behavior.
#[test]
fn secondary_channel_ignores_command_to_absent_device1() {
    let mut ide = IdeSecondary::with_image(vec![0u8; 512 * 2]);
    ide.port_write(IDE_SECONDARY_CTRL, 1, 0);
    ide.port_write(IDE_SECONDARY_DRIVE, 1, DEV1);
    ide.port_write(IDE_SECONDARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY));
    assert_eq!(ide.port_read(IDE_SECONDARY_STATUS, 1) as u8, 0);
    assert!(!ide.irq_line());

    ide.port_write(IDE_SECONDARY_DRIVE, 1, DEV0);
    assert_eq!(
        ide.port_read(IDE_SECONDARY_CTRL, 1) as u8,
        ATA_SR_DRDY | ATA_SR_DSC
    );
    ide.port_write(IDE_SECONDARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY));
    assert_ne!(ide.port_read(IDE_SECONDARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
    assert!(ide.irq_line());
}

/// An empty channel (no Device 0 either) keeps reading `00h` for both devices.
#[test]
fn empty_channel_reads_zero_for_both_devices() {
    let mut ide = IdePrimary::new();
    for dev in [DEV0, DEV1] {
        ide.port_write(IDE_PRIMARY_DRIVE, 1, dev);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY));
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8, 0);
        assert!(!ide.irq_line());
    }
}
