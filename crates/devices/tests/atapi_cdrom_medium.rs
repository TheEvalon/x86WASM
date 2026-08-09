//! ATAPI CD-ROM medium: READ CAPACITY / READ (10) and TUR honesty.
//!
//! # Spec refs
//!
//! - ATA/ATAPI-6 §8.16.9 — IDENTIFY PACKET DEVICE word 0 bits (12:8) = `05h`
//!   CD-ROM only when the CD-ROM packet set exists; bit 7 RMB.
//! - SFF-8020i / MMC — READ CAPACITY (`25h`), READ (10) (`28h`), 2048-byte
//!   logical blocks; TEST UNIT READY with NOT READY / ASC `3Ah` when empty.
//! - SCSI Primary Commands — peripheral device type `05h`.

use devices::{
    IdePrimary, PortDevice, ATA_CMD_IDENTIFY_PACKET, ATA_CMD_PACKET, ATA_SR_DRDY, ATA_SR_DRQ,
    ATA_SR_ERR, IDE_PRIMARY_CTRL, IDE_PRIMARY_DATA, IDE_PRIMARY_ERROR, IDE_PRIMARY_LBA_HI,
    IDE_PRIMARY_LBA_MID, IDE_PRIMARY_SECCOUNT, IDE_PRIMARY_STATUS,
};

const IR_CD: u32 = 0x01;
const IR_IO: u32 = 0x02;
const PACKET_BYTES: usize = 12;
const CMD_TEST_UNIT_READY: u8 = 0x00;
const CMD_REQUEST_SENSE: u8 = 0x03;
const CMD_INQUIRY: u8 = 0x12;
const CMD_READ_CAPACITY: u8 = 0x25;
const CMD_READ_10: u8 = 0x28;
const SENSE_NOT_READY: u8 = 0x02;
const SENSE_ILLEGAL_REQUEST: u8 = 0x05;
const ASC_INVALID_COMMAND: u8 = 0x20;
const ASC_MEDIUM_NOT_PRESENT: u8 = 0x3A;
const ASC_LBA_OUT_OF_RANGE: u8 = 0x21;
const CDROM_BLOCK: usize = 2048;
const PERIPH_CDROM: u8 = 0x05;
const PERIPH_UNKNOWN: u8 = 0x1F;

fn cdrom_empty() -> IdePrimary {
    let mut ide = IdePrimary::with_atapi_cdrom();
    ide.port_write(IDE_PRIMARY_CTRL, 1, 0);
    ide
}

fn cdrom_with(blocks: usize, fill: u8) -> IdePrimary {
    let mut image = vec![fill; blocks * CDROM_BLOCK];
    // Distinct first bytes per block for READ (10) checks.
    for b in 0..blocks {
        image[b * CDROM_BLOCK] = b as u8;
    }
    let mut ide = IdePrimary::with_atapi_cdrom_image(image);
    ide.port_write(IDE_PRIMARY_CTRL, 1, 0);
    ide
}

fn start_packet(ide: &mut IdePrimary, byte_count_limit: u16) {
    ide.port_write(IDE_PRIMARY_LBA_MID, 1, u32::from(byte_count_limit & 0xFF));
    ide.port_write(IDE_PRIMARY_LBA_HI, 1, u32::from(byte_count_limit >> 8));
    ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_PACKET));
}

fn send_packet(ide: &mut IdePrimary, packet: &[u8; PACKET_BYTES]) {
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

fn status(ide: &mut IdePrimary) -> u8 {
    ide.port_read(IDE_PRIMARY_CTRL, 1) as u8
}

fn identify_word0(ide: &mut IdePrimary) -> u16 {
    ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY_PACKET));
    assert_eq!(status(ide) & ATA_SR_DRQ, ATA_SR_DRQ);
    let lo = ide.port_read(IDE_PRIMARY_DATA, 2) as u16;
    // Drain the rest of the 512-byte identify block.
    for _ in 1..256 {
        let _ = ide.port_read(IDE_PRIMARY_DATA, 2);
    }
    lo
}

/// Minimal PACKET device stays type `1Fh` and rejects READ (10).
#[test]
fn minimal_packet_device_stays_unknown_type_without_read() {
    let mut ide = IdePrimary::with_atapi_device();
    ide.port_write(IDE_PRIMARY_CTRL, 1, 0);
    let word0 = identify_word0(&mut ide);
    assert_eq!((word0 >> 8) & 0x1F, u16::from(PERIPH_UNKNOWN));

    start_packet(&mut ide, 512);
    let mut pkt = [0u8; PACKET_BYTES];
    pkt[0] = CMD_READ_10;
    pkt[8] = 1;
    send_packet(&mut ide, &pkt);
    assert_eq!(status(&mut ide) & ATA_SR_ERR, ATA_SR_ERR);
    assert_eq!(
        ide.port_read(IDE_PRIMARY_ERROR, 1) >> 4,
        u32::from(SENSE_ILLEGAL_REQUEST)
    );
    let (key, asc, _) = ide.atapi_sense();
    assert_eq!(key, SENSE_ILLEGAL_REQUEST);
    assert_eq!(asc, ASC_INVALID_COMMAND);
}

/// Spec: ATA/ATAPI-6 §8.16.9 — CD-ROM capable device reports type `05h` and RMB.
#[test]
fn cdrom_identify_reports_type_05h_even_when_empty() {
    let mut ide = cdrom_empty();
    let word0 = identify_word0(&mut ide);
    assert_eq!((word0 >> 8) & 0x1F, u16::from(PERIPH_CDROM));
    assert_ne!(word0 & (1 << 7), 0, "RMB set for CD-ROM");
    assert!(ide.is_atapi_cdrom());
    assert!(!ide.atapi_medium_loaded());
}

/// Spec: SFF-8020i §10.8.24 — empty CD-ROM is NOT READY / medium not present.
#[test]
fn test_unit_ready_not_ready_when_cdrom_empty() {
    let mut ide = cdrom_empty();
    start_packet(&mut ide, 512);
    let mut p = [0u8; PACKET_BYTES];
    p[0] = CMD_TEST_UNIT_READY;
    send_packet(&mut ide, &p);
    assert_eq!(status(&mut ide) & ATA_SR_ERR, ATA_SR_ERR);
    assert_eq!(
        ide.port_read(IDE_PRIMARY_ERROR, 1) >> 4,
        u32::from(SENSE_NOT_READY)
    );
    let (key, asc, _) = ide.atapi_sense();
    assert_eq!(key, SENSE_NOT_READY);
    assert_eq!(asc, ASC_MEDIUM_NOT_PRESENT);
}

/// Loaded medium: TUR completes GOOD.
#[test]
fn test_unit_ready_good_when_medium_loaded() {
    let mut ide = cdrom_with(2, 0xAA);
    start_packet(&mut ide, 512);
    let mut p = [0u8; PACKET_BYTES];
    p[0] = CMD_TEST_UNIT_READY;
    send_packet(&mut ide, &p);
    assert_eq!(status(&mut ide) & ATA_SR_ERR, 0);
    assert_eq!(status(&mut ide) & ATA_SR_DRDY, ATA_SR_DRDY);
    assert_eq!(ide.port_read(IDE_PRIMARY_SECCOUNT, 1), IR_CD | IR_IO);
}

/// Spec: MMC READ CAPACITY — last LBA and 2048-byte block length, big-endian.
#[test]
fn read_capacity_returns_last_lba_and_block_size() {
    let mut ide = cdrom_with(4, 0);
    start_packet(&mut ide, 512);
    let mut p = [0u8; PACKET_BYTES];
    p[0] = CMD_READ_CAPACITY;
    send_packet(&mut ide, &p);
    assert_eq!(status(&mut ide) & ATA_SR_DRQ, ATA_SR_DRQ);
    let data = read_bytes(&mut ide, 8);
    assert_eq!(&data[0..4], &3u32.to_be_bytes());
    assert_eq!(&data[4..8], &(CDROM_BLOCK as u32).to_be_bytes());
    assert_eq!(status(&mut ide) & ATA_SR_ERR, 0);
}

/// Spec: MMC READ (10) — transfer logical blocks from the attached image.
#[test]
fn read10_returns_requested_blocks() {
    let mut ide = cdrom_with(3, 0x00);
    start_packet(&mut ide, 4096);
    let mut p = [0u8; PACKET_BYTES];
    p[0] = CMD_READ_10;
    p[5] = 1; // LBA 1
    p[8] = 2; // two blocks
    send_packet(&mut ide, &p);
    let data = read_bytes(&mut ide, 2 * CDROM_BLOCK);
    assert_eq!(data[0], 1);
    assert_eq!(data[CDROM_BLOCK], 2);
    assert_eq!(status(&mut ide) & ATA_SR_ERR, 0);
}

/// Out-of-range LBA → ILLEGAL REQUEST / ASC 21h.
#[test]
fn read10_rejects_lba_out_of_range() {
    let mut ide = cdrom_with(1, 0);
    start_packet(&mut ide, 512);
    let mut p = [0u8; PACKET_BYTES];
    p[0] = CMD_READ_10;
    p[5] = 1;
    p[8] = 1;
    send_packet(&mut ide, &p);
    assert_eq!(status(&mut ide) & ATA_SR_ERR, ATA_SR_ERR);
    let (key, asc, _) = ide.atapi_sense();
    assert_eq!(key, SENSE_ILLEGAL_REQUEST);
    assert_eq!(asc, ASC_LBA_OUT_OF_RANGE);
}

/// Empty CD-ROM: READ CAPACITY is NOT READY, not a fake capacity.
#[test]
fn read_capacity_not_ready_when_empty() {
    let mut ide = cdrom_empty();
    start_packet(&mut ide, 512);
    let mut p = [0u8; PACKET_BYTES];
    p[0] = CMD_READ_CAPACITY;
    send_packet(&mut ide, &p);
    assert_eq!(status(&mut ide) & ATA_SR_ERR, ATA_SR_ERR);
    let (key, asc, _) = ide.atapi_sense();
    assert_eq!(key, SENSE_NOT_READY);
    assert_eq!(asc, ASC_MEDIUM_NOT_PRESENT);
}

/// INQUIRY on a CD-ROM capable device matches identify type `05h`.
#[test]
fn inquiry_on_cdrom_reports_type_05h_and_rmb() {
    let mut ide = cdrom_empty();
    start_packet(&mut ide, 512);
    let mut p = [0u8; PACKET_BYTES];
    p[0] = CMD_INQUIRY;
    p[4] = 36;
    send_packet(&mut ide, &p);
    let data = read_bytes(&mut ide, 36);
    assert_eq!(data[0], PERIPH_CDROM);
    assert_eq!(data[1] & 0x80, 0x80);
}

/// REQUEST SENSE after empty TUR reports and clears the medium-not-present sense.
#[test]
fn request_sense_reports_medium_not_present_after_tur() {
    let mut ide = cdrom_empty();
    start_packet(&mut ide, 512);
    let mut tur = [0u8; PACKET_BYTES];
    tur[0] = CMD_TEST_UNIT_READY;
    send_packet(&mut ide, &tur);
    ide.port_read(IDE_PRIMARY_STATUS, 1);

    start_packet(&mut ide, 512);
    let mut rs = [0u8; PACKET_BYTES];
    rs[0] = CMD_REQUEST_SENSE;
    rs[4] = 18;
    send_packet(&mut ide, &rs);
    let data = read_bytes(&mut ide, 18);
    assert_eq!(data[2] & 0x0F, SENSE_NOT_READY);
    assert_eq!(data[12], ASC_MEDIUM_NOT_PRESENT);
    assert_eq!(ide.atapi_sense(), (0, 0, 0));
}
