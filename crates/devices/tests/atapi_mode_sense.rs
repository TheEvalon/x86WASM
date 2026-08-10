//! ATAPI MODE SENSE(6)/(10) for CD-ROM capable PACKET devices.
//!
//! # Spec refs
//!
//! - SFF-8020i §9.8.4 MODE SENSE (`5Ah`) — page control, page code, allocation
//!   length; unsupported page → ILLEGAL REQUEST / ASC `24h`.
//! - SFF-8020i §9.8.5 / Table 45 — mode parameter header (no block descriptors
//!   in this model); Table 52 Read Error Recovery page (`01h`).
//! - SFF-8020i Table 8 — MODE SENSE does **not** return NOT READY when empty.
//! - MMC / SPC MODE SENSE(6) (`1Ah`) — 4-byte header + page (same page `01h`).
//! - Round-5 type `1Fh` minimal PACKET path rejects these opcodes as unknown.

use devices::{
    IdePrimary, PortDevice, ATA_CMD_PACKET, ATA_SR_DRQ, ATA_SR_ERR, IDE_PRIMARY_CTRL,
    IDE_PRIMARY_DATA, IDE_PRIMARY_ERROR, IDE_PRIMARY_LBA_HI, IDE_PRIMARY_LBA_MID,
    IDE_PRIMARY_STATUS,
};

const PACKET_BYTES: usize = 12;
const CMD_MODE_SENSE_6: u8 = 0x1A;
const CMD_MODE_SENSE_10: u8 = 0x5A;
const CMD_READ_10: u8 = 0x28;
const SENSE_ILLEGAL_REQUEST: u8 = 0x05;
const ASC_INVALID_COMMAND: u8 = 0x20;
const ASC_INVALID_FIELD: u8 = 0x24;
const PAGE_ERROR_RECOVERY: u8 = 0x01;
const PAGE_LENGTH_ERROR_RECOVERY: u8 = 0x06;
const MEDIUM_NO_DISC: u8 = 0x70;
const MEDIUM_120MM_DATA: u8 = 0x01;
const CDROM_BLOCK: usize = 2048;

fn cdrom_empty() -> IdePrimary {
    let mut ide = IdePrimary::with_atapi_cdrom();
    ide.port_write(IDE_PRIMARY_CTRL, 1, 0);
    ide
}

fn cdrom_with(blocks: usize) -> IdePrimary {
    let image = vec![0u8; blocks * CDROM_BLOCK];
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

/// Spec: SFF-8020i Table 8 — MODE SENSE succeeds on an empty tray.
#[test]
fn mode_sense10_returns_error_recovery_page_when_empty() {
    let mut ide = cdrom_empty();
    start_packet(&mut ide, 512);
    let mut p = [0u8; PACKET_BYTES];
    p[0] = CMD_MODE_SENSE_10;
    p[2] = PAGE_ERROR_RECOVERY;
    p[7] = 0;
    p[8] = 64;
    send_packet(&mut ide, &p);
    assert_eq!(status(&mut ide) & ATA_SR_DRQ, ATA_SR_DRQ);
    // 8-byte header + 8-byte page.
    let data = read_bytes(&mut ide, 16);
    let mode_len = u16::from_be_bytes([data[0], data[1]]);
    assert_eq!(mode_len, 14, "mode data length excludes itself");
    assert_eq!(data[2], MEDIUM_NO_DISC);
    assert_eq!(u16::from_be_bytes([data[6], data[7]]), 0, "no block descriptors");
    assert_eq!(data[8] & 0x3F, PAGE_ERROR_RECOVERY);
    assert_eq!(data[9], PAGE_LENGTH_ERROR_RECOVERY);
    assert_eq!(status(&mut ide) & ATA_SR_ERR, 0);
}

/// Loaded medium reports 120 mm data-only medium type.
#[test]
fn mode_sense10_reports_data_medium_type_when_loaded() {
    let mut ide = cdrom_with(2);
    start_packet(&mut ide, 512);
    let mut p = [0u8; PACKET_BYTES];
    p[0] = CMD_MODE_SENSE_10;
    p[2] = PAGE_ERROR_RECOVERY;
    p[8] = 64;
    send_packet(&mut ide, &p);
    let data = read_bytes(&mut ide, 16);
    assert_eq!(data[2], MEDIUM_120MM_DATA);
    assert_eq!(data[8] & 0x3F, PAGE_ERROR_RECOVERY);
}

/// Spec: MMC MODE SENSE(6) — 4-byte header + page `01h`.
#[test]
fn mode_sense6_returns_error_recovery_page() {
    let mut ide = cdrom_with(1);
    start_packet(&mut ide, 512);
    let mut p = [0u8; PACKET_BYTES];
    p[0] = CMD_MODE_SENSE_6;
    p[2] = PAGE_ERROR_RECOVERY;
    p[4] = 64;
    send_packet(&mut ide, &p);
    assert_eq!(status(&mut ide) & ATA_SR_DRQ, ATA_SR_DRQ);
    let data = read_bytes(&mut ide, 12);
    assert_eq!(data[0], 11, "mode data length excludes itself");
    assert_eq!(data[1], MEDIUM_120MM_DATA);
    assert_eq!(data[3], 0, "no block descriptors");
    assert_eq!(data[4] & 0x3F, PAGE_ERROR_RECOVERY);
    assert_eq!(data[5], PAGE_LENGTH_ERROR_RECOVERY);
}

/// Unknown page code → ILLEGAL REQUEST / INVALID FIELD IN CDB.
#[test]
fn mode_sense10_rejects_unknown_page() {
    let mut ide = cdrom_empty();
    start_packet(&mut ide, 512);
    let mut p = [0u8; PACKET_BYTES];
    p[0] = CMD_MODE_SENSE_10;
    p[2] = 0x2A; // capabilities — not implemented in this slice
    p[8] = 64;
    send_packet(&mut ide, &p);
    assert_eq!(status(&mut ide) & ATA_SR_ERR, ATA_SR_ERR);
    assert_eq!(
        ide.port_read(IDE_PRIMARY_ERROR, 1) >> 4,
        u32::from(SENSE_ILLEGAL_REQUEST)
    );
    let (key, asc, _) = ide.atapi_sense();
    assert_eq!(key, SENSE_ILLEGAL_REQUEST);
    assert_eq!(asc, ASC_INVALID_FIELD);
}

/// Minimal type-`1Fh` device still rejects MODE SENSE as unknown opcode.
#[test]
fn minimal_packet_device_rejects_mode_sense() {
    let mut ide = IdePrimary::with_atapi_device();
    ide.port_write(IDE_PRIMARY_CTRL, 1, 0);
    start_packet(&mut ide, 512);
    let mut p = [0u8; PACKET_BYTES];
    p[0] = CMD_MODE_SENSE_10;
    p[2] = PAGE_ERROR_RECOVERY;
    p[8] = 64;
    send_packet(&mut ide, &p);
    assert_eq!(status(&mut ide) & ATA_SR_ERR, ATA_SR_ERR);
    let (key, asc, _) = ide.atapi_sense();
    assert_eq!(key, SENSE_ILLEGAL_REQUEST);
    assert_eq!(asc, ASC_INVALID_COMMAND);

    // Sanity: READ (10) remains invalid on the minimal path too.
    start_packet(&mut ide, 512);
    let mut r = [0u8; PACKET_BYTES];
    r[0] = CMD_READ_10;
    r[8] = 1;
    send_packet(&mut ide, &r);
    let (key, asc, _) = ide.atapi_sense();
    assert_eq!(key, SENSE_ILLEGAL_REQUEST);
    assert_eq!(asc, ASC_INVALID_COMMAND);
}

/// Allocation length truncates the mode parameter list.
#[test]
fn mode_sense6_honors_allocation_length() {
    let mut ide = cdrom_empty();
    start_packet(&mut ide, 512);
    let mut p = [0u8; PACKET_BYTES];
    p[0] = CMD_MODE_SENSE_6;
    p[2] = PAGE_ERROR_RECOVERY;
    p[4] = 4; // header only
    send_packet(&mut ide, &p);
    let data = read_bytes(&mut ide, 4);
    assert_eq!(data.len(), 4);
    assert_eq!(data[0], 11); // full available length still reported in header byte
    assert_eq!(data[1], MEDIUM_NO_DISC);
    assert_eq!(status(&mut ide) & ATA_SR_ERR, 0);
}
