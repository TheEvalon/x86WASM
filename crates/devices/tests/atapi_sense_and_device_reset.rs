//! ATAPI `REQUEST SENSE`, the sense-data model, and `DEVICE RESET` (`08h`).
//!
//! # Spec refs
//!
//! - SFF-8020i (ATA Packet Interface for CD-ROMs) §10.8.16 REQUEST SENSE — the
//!   18-byte fixed-format sense data: byte 0 response code `70h` "current
//!   error", byte 2 bits (3:0) sense key, byte 7 Additional Sense Length `0Ah`,
//!   byte 12 ASC, byte 13 ASCQ; the sense data describes the CHECK CONDITION
//!   that preceded it and is cleared once reported.
//! - SFF-8020i Sense Key / ASC / ASCQ definitions — sense key `5h` ILLEGAL
//!   REQUEST with ASC `20h` INVALID COMMAND OPERATION CODE and `24h` INVALID
//!   FIELD IN COMMAND PACKET.
//! - ATA/ATAPI-6 (T13/1410D r3b) §8.21.6 — the Error register on a PACKET
//!   device carries the Sense Key in bits (7:4) and ABRT in bit 2.
//! - ATA/ATAPI-6 §8.7 DEVICE RESET — mandatory for the PACKET Command feature
//!   set and "use prohibited" without it; §8.7.5 normal outputs put the
//!   diagnostic code in the Error register and the §9.12 signature in the
//!   Command Block; the command does not assert INTRQ.
//! - ATA/ATAPI-6 §6.8 / Table 29 — IDENTIFY PACKET DEVICE word 82 bit 9 "The
//!   DEVICE RESET command is supported", mirrored as enabled in word 85.

use devices::{
    IdePrimary, PortDevice, ATA_CMD_IDENTIFY_PACKET, ATA_CMD_PACKET, ATA_DIAG_PASSED, ATA_ER_ABRT,
    ATA_SR_DRDY, ATA_SR_DRQ, ATA_SR_ERR, IDE_PRIMARY_CTRL, IDE_PRIMARY_DATA, IDE_PRIMARY_DRIVE,
    IDE_PRIMARY_ERROR, IDE_PRIMARY_LBA_HI, IDE_PRIMARY_LBA_LO, IDE_PRIMARY_LBA_MID,
    IDE_PRIMARY_SECCOUNT, IDE_PRIMARY_STATUS,
};

/// DEVICE RESET. Spec: ATA/ATAPI-6 §8.7.
const CMD_DEVICE_RESET: u32 = 0x08;
/// `TEST UNIT READY`. Spec: SFF-8020i §10.8.24.
const CMD_TEST_UNIT_READY: u8 = 0x00;
/// `REQUEST SENSE`. Spec: SFF-8020i §10.8.16.
const CMD_REQUEST_SENSE: u8 = 0x03;
/// `INQUIRY`. Spec: SFF-8020i §10.8.4.
const CMD_INQUIRY: u8 = 0x12;
/// `READ (10)` — deliberately unimplemented. Spec: SFF-8020i §10.8.13.
const CMD_READ_10: u8 = 0x28;
/// Sense key ILLEGAL REQUEST. Spec: SFF-8020i.
const SENSE_ILLEGAL_REQUEST: u8 = 0x05;
/// Sense key NO SENSE. Spec: SFF-8020i.
const SENSE_NO_SENSE: u8 = 0x00;
/// ASC INVALID COMMAND OPERATION CODE. Spec: SFF-8020i.
const ASC_INVALID_COMMAND_OPERATION_CODE: u8 = 0x20;
/// ASC INVALID FIELD IN COMMAND PACKET. Spec: SFF-8020i.
const ASC_INVALID_FIELD_IN_CDB: u8 = 0x24;
/// Fixed-format sense data length. Spec: SFF-8020i §10.8.16.
const SENSE_BYTES: usize = 18;
/// Command packet length. Spec: ATA/ATAPI-6 §8.16.9 word 0 bits (1:0) = `00b`.
const PACKET_BYTES: usize = 12;

fn atapi() -> IdePrimary {
    let mut ide = IdePrimary::with_atapi_device();
    ide.port_write(IDE_PRIMARY_CTRL, 1, 0);
    ide
}

fn packet_for(op: u8, allocation_length: u8) -> [u8; PACKET_BYTES] {
    let mut packet = [0u8; PACKET_BYTES];
    packet[0] = op;
    packet[4] = allocation_length;
    packet
}

fn run_packet(ide: &mut IdePrimary, packet: &[u8; PACKET_BYTES]) {
    ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0x00);
    ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0x02);
    ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_PACKET));
    for pair in packet.chunks(2) {
        let word = u32::from(pair[0]) | (u32::from(pair[1]) << 8);
        ide.port_write(IDE_PRIMARY_DATA, 2, word);
    }
}

fn read_bytes(ide: &mut IdePrimary, len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    while out.len() < len {
        let word = ide.port_read(IDE_PRIMARY_DATA, 2);
        out.push((word & 0xFF) as u8);
        if out.len() < len {
            out.push(((word >> 8) & 0xFF) as u8);
        }
    }
    out
}

fn request_sense(ide: &mut IdePrimary, allocation_length: u8) -> Vec<u8> {
    run_packet(ide, &packet_for(CMD_REQUEST_SENSE, allocation_length));
    let count = usize::from(allocation_length).min(SENSE_BYTES);
    read_bytes(ide, count)
}

fn status(ide: &mut IdePrimary) -> u8 {
    ide.port_read(IDE_PRIMARY_CTRL, 1) as u8
}

fn signature(ide: &mut IdePrimary) -> (u8, u8, u8, u8) {
    (
        ide.port_read(IDE_PRIMARY_SECCOUNT, 1) as u8,
        ide.port_read(IDE_PRIMARY_LBA_LO, 1) as u8,
        ide.port_read(IDE_PRIMARY_LBA_MID, 1) as u8,
        ide.port_read(IDE_PRIMARY_LBA_HI, 1) as u8,
    )
}

/// The whole point of the sense model: an unimplemented packet command no
/// longer fails silently, it reports *why*.
///
/// Spec: SFF-8020i §10.8.16 and ATA/ATAPI-6 §8.21.6.
#[test]
fn the_check_condition_to_request_sense_cycle_reports_the_reason() {
    let mut ide = atapi();
    run_packet(&mut ide, &packet_for(CMD_READ_10, 0));
    assert_eq!(status(&mut ide) & ATA_SR_ERR, ATA_SR_ERR);
    assert_eq!(
        ide.port_read(IDE_PRIMARY_ERROR, 1) as u8,
        (SENSE_ILLEGAL_REQUEST << 4) | ATA_ER_ABRT
    );

    let sense = request_sense(&mut ide, SENSE_BYTES as u8);
    assert_eq!(sense[2] & 0x0F, SENSE_ILLEGAL_REQUEST);
    assert_eq!(sense[12], ASC_INVALID_COMMAND_OPERATION_CODE);
    assert_eq!(sense[13], 0x00);
    // REQUEST SENSE itself succeeds.
    assert_eq!(status(&mut ide) & ATA_SR_ERR, 0);
}

/// Spec: SFF-8020i §10.8.16 — the fixed-format sense data layout.
#[test]
fn the_sense_data_is_the_eighteen_byte_fixed_format() {
    let mut ide = atapi();
    run_packet(&mut ide, &packet_for(CMD_READ_10, 0));

    let sense = request_sense(&mut ide, SENSE_BYTES as u8);
    assert_eq!(sense.len(), SENSE_BYTES);
    // Byte 0: response code 70h "current error", VALID (bit 7) cleared because
    // there is no valid information field.
    assert_eq!(sense[0], 0x70);
    assert_eq!(sense[1], 0x00, "segment number");
    // Byte 2 bits 7:5 FILEMARK / EOM / ILI are all clear.
    assert_eq!(sense[2] & 0xE0, 0x00);
    assert_eq!(&sense[3..7], &[0, 0, 0, 0], "information field");
    // Byte 7: additional sense length 0Ah, so 8 + 10 = 18 bytes total.
    assert_eq!(sense[7], 0x0A);
    assert_eq!(&sense[8..12], &[0, 0, 0, 0], "command-specific information");
    assert_eq!(sense[14], 0x00, "field replaceable unit code");
    assert_eq!(&sense[15..18], &[0, 0, 0], "sense-key specific");
}

/// Spec: SFF-8020i §10.8.16 — sense data is valid until it is reported, so a
/// second REQUEST SENSE with nothing new to report reads NO SENSE.
#[test]
fn request_sense_clears_the_sense_data() {
    let mut ide = atapi();
    run_packet(&mut ide, &packet_for(CMD_READ_10, 0));
    assert_eq!(
        ide.atapi_sense(),
        (SENSE_ILLEGAL_REQUEST, ASC_INVALID_COMMAND_OPERATION_CODE, 0)
    );

    let first = request_sense(&mut ide, SENSE_BYTES as u8);
    assert_eq!(first[2] & 0x0F, SENSE_ILLEGAL_REQUEST);
    assert_eq!(ide.atapi_sense(), (SENSE_NO_SENSE, 0, 0));

    let second = request_sense(&mut ide, SENSE_BYTES as u8);
    assert_eq!(second[2] & 0x0F, SENSE_NO_SENSE);
    assert_eq!(second[12], 0x00);
}

/// Spec: SFF-8020i — the allocation length in packet byte 4 bounds the
/// transfer; nothing is reported, and therefore nothing is cleared, when it is
/// zero.
#[test]
fn a_short_allocation_length_truncates_and_zero_preserves_the_sense_data() {
    let mut ide = atapi();
    run_packet(&mut ide, &packet_for(CMD_READ_10, 0));

    let short = request_sense(&mut ide, 8);
    assert_eq!(short.len(), 8);
    assert_eq!(short[2] & 0x0F, SENSE_ILLEGAL_REQUEST);
    assert_eq!(ide.atapi_sense(), (SENSE_NO_SENSE, 0, 0));

    // A fresh CHECK CONDITION, then a zero-length REQUEST SENSE.
    let mut packet = packet_for(CMD_INQUIRY, 36);
    packet[1] = 0x01; // EVPD
    run_packet(&mut ide, &packet);
    assert_eq!(
        ide.atapi_sense(),
        (SENSE_ILLEGAL_REQUEST, ASC_INVALID_FIELD_IN_CDB, 0)
    );

    run_packet(&mut ide, &packet_for(CMD_REQUEST_SENSE, 0));
    assert_eq!(status(&mut ide) & (ATA_SR_DRQ | ATA_SR_ERR), 0);
    assert_eq!(
        ide.atapi_sense(),
        (SENSE_ILLEGAL_REQUEST, ASC_INVALID_FIELD_IN_CDB, 0),
        "nothing was reported, so nothing is cleared"
    );
}

/// A command that succeeds does not disturb sense data that has not yet been
/// read; only REQUEST SENSE clears it.
///
/// Spec: SFF-8020i §10.8.16.
#[test]
fn a_successful_command_leaves_pending_sense_data_alone() {
    let mut ide = atapi();
    run_packet(&mut ide, &packet_for(CMD_READ_10, 0));
    run_packet(&mut ide, &packet_for(CMD_TEST_UNIT_READY, 0));
    assert_eq!(status(&mut ide) & ATA_SR_ERR, 0);

    let sense = request_sense(&mut ide, SENSE_BYTES as u8);
    assert_eq!(sense[2] & 0x0F, SENSE_ILLEGAL_REQUEST);
    assert_eq!(sense[12], ASC_INVALID_COMMAND_OPERATION_CODE);
}

/// Spec: ATA/ATAPI-6 §8.7 / §8.7.5 — DEVICE RESET puts the diagnostic code in
/// the Error register and the §9.12 PACKET signature in the Command Block, and
/// **does not assert INTRQ**.
#[test]
fn device_reset_writes_the_signature_without_an_interrupt() {
    let mut ide = atapi();
    // Overwrite the signature and select the device so the effect is visible.
    ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
    ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0x55);
    ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0xAA);
    assert_eq!(signature(&mut ide), (0x01, 0x01, 0x55, 0xAA));

    ide.port_write(IDE_PRIMARY_STATUS, 1, CMD_DEVICE_RESET);

    assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_DIAG_PASSED);
    assert_eq!(signature(&mut ide), (0x01, 0x01, 0x14, 0xEB));
    // Spec: §9.10 — a PACKET device clears Status bits 6,5,4,3,2 and 0.
    assert_eq!(status(&mut ide), 0x00);
    assert!(
        !ide.irq_line(),
        "DEVICE RESET completes without asserting INTRQ"
    );
}

/// Spec: ATA/ATAPI-6 §8.7 — DEVICE RESET ends any command in progress.
#[test]
fn device_reset_drops_an_in_progress_packet_transfer_and_pending_sense() {
    let mut ide = atapi();
    run_packet(&mut ide, &packet_for(CMD_READ_10, 0));
    assert_eq!(ide.atapi_sense().0, SENSE_ILLEGAL_REQUEST);

    // Enter a data-in phase, then reset in the middle of it.
    run_packet(&mut ide, &packet_for(CMD_INQUIRY, 36));
    assert_eq!(status(&mut ide) & ATA_SR_DRQ, ATA_SR_DRQ);

    ide.port_write(IDE_PRIMARY_STATUS, 1, CMD_DEVICE_RESET);
    assert_eq!(status(&mut ide) & ATA_SR_DRQ, 0);
    assert_eq!(ide.port_read(IDE_PRIMARY_DATA, 2), 0xFFFF_FFFF);
    assert_eq!(ide.atapi_sense(), (SENSE_NO_SENSE, 0, 0));
}

/// Spec: ATA/ATAPI-6 §8.7.2 — "use prohibited for devices not implementing the
/// PACKET Command feature set".
#[test]
fn device_reset_is_aborted_on_an_ata_disk() {
    let mut ide = IdePrimary::with_image(vec![0u8; 512 * 4]);
    ide.port_write(IDE_PRIMARY_CTRL, 1, 0);
    ide.port_write(IDE_PRIMARY_STATUS, 1, CMD_DEVICE_RESET);

    assert_eq!(status(&mut ide) & ATA_SR_ERR, ATA_SR_ERR);
    assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_ER_ABRT);
    // Spec: §9.12 — an ATA disk keeps the non-PACKET signature.
    assert_eq!(signature(&mut ide), (0x01, 0x01, 0x00, 0x00));
}

/// Spec: ATA/ATAPI-6 Table 29 — word 82 bit 9 reports DEVICE RESET support and
/// word 85 bit 9 reports it enabled. Round 3 left both clear because `08h` was
/// unimplemented; now the claim is truthful.
#[test]
fn identify_packet_device_now_claims_device_reset() {
    let mut ide = atapi();
    ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
    ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY_PACKET));
    assert_eq!(status(&mut ide) & ATA_SR_DRQ, ATA_SR_DRQ);

    let block = read_bytes(&mut ide, 512);
    let word = |index: usize| u16::from(block[index * 2]) | (u16::from(block[index * 2 + 1]) << 8);

    assert_eq!(word(82), (1 << 4) | (1 << 9));
    assert_eq!(word(85), (1 << 4) | (1 << 9));
    // Word 0 is unchanged: still `1Fh`, still not a CD-ROM.
    assert_eq!(word(0), 0x9F00);
    assert_eq!(status(&mut ide) & ATA_SR_DRDY, ATA_SR_DRDY);
}
