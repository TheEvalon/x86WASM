//! ATAPI READ TOC and START STOP UNIT for CD-ROM media.
//!
//! # Spec refs
//!
//! - SFF-8020i §9.8.20 READ TOC (`43h`) — format `00b` TOC data; single data
//!   track + lead-out from capacity; empty → NOT READY / ASC `3Ah`.
//! - SFF-8020i §9.8.26 START/STOP UNIT (`1Bh`) — LoEj/Start; eject unloads the
//!   medium so subsequent TUR reports NOT READY / `3Ah`.
//! - SFF-8020i Table 8 — READ TOC and START STOP may return NOT READY.

use devices::{
    IdePrimary, PortDevice, ATA_CMD_PACKET, ATA_SR_DRQ, ATA_SR_ERR, IDE_PRIMARY_CTRL,
    IDE_PRIMARY_DATA, IDE_PRIMARY_LBA_HI, IDE_PRIMARY_LBA_MID, IDE_PRIMARY_STATUS,
};

const PACKET_BYTES: usize = 12;
const CMD_TEST_UNIT_READY: u8 = 0x00;
const CMD_START_STOP_UNIT: u8 = 0x1B;
const CMD_READ_TOC: u8 = 0x43;
const SENSE_NOT_READY: u8 = 0x02;
const SENSE_ILLEGAL_REQUEST: u8 = 0x05;
const ASC_INVALID_COMMAND: u8 = 0x20;
const ASC_INVALID_FIELD: u8 = 0x24;
const ASC_MEDIUM_NOT_PRESENT: u8 = 0x3A;
const CDROM_BLOCK: usize = 2048;
const TOC_CTRL_DATA: u8 = 0x14; // ADR=1, Control=4 (data, copy prohibited)
const TRACK_LEAD_OUT: u8 = 0xAA;

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

fn tur(ide: &mut IdePrimary) -> (u8, u8) {
    start_packet(ide, 512);
    let mut p = [0u8; PACKET_BYTES];
    p[0] = CMD_TEST_UNIT_READY;
    send_packet(ide, &p);
    if status(ide) & ATA_SR_ERR == 0 {
        (0, 0)
    } else {
        let (key, asc, _) = ide.atapi_sense();
        (key, asc)
    }
}

/// Spec: SFF-8020i Table 112 — single data track + lead-out from capacity (LBA).
#[test]
fn read_toc_returns_single_data_track_and_lead_out() {
    let blocks = 100u32;
    let mut ide = cdrom_with(blocks as usize);
    start_packet(&mut ide, 512);
    let mut p = [0u8; PACKET_BYTES];
    p[0] = CMD_READ_TOC;
    p[2] = 0x00; // format 00 (MMC byte 2)
    p[6] = 0x00; // starting track 0 → first track
    p[7] = 0;
    p[8] = 24;
    send_packet(&mut ide, &p);
    assert_eq!(status(&mut ide) & ATA_SR_DRQ, ATA_SR_DRQ);
    let data = read_bytes(&mut ide, 20);
    assert_eq!(u16::from_be_bytes([data[0], data[1]]), 18);
    assert_eq!(data[2], 1); // first track
    assert_eq!(data[3], 1); // last track
    // Track 1 descriptor
    assert_eq!(data[5], TOC_CTRL_DATA);
    assert_eq!(data[6], 1);
    assert_eq!(&data[8..12], &0u32.to_be_bytes());
    // Lead-out
    assert_eq!(data[13], TOC_CTRL_DATA);
    assert_eq!(data[14], TRACK_LEAD_OUT);
    assert_eq!(&data[16..20], &blocks.to_be_bytes());
}

/// Empty tray: READ TOC is NOT READY / medium not present.
#[test]
fn read_toc_not_ready_when_empty() {
    let mut ide = IdePrimary::with_atapi_cdrom();
    ide.port_write(IDE_PRIMARY_CTRL, 1, 0);
    start_packet(&mut ide, 512);
    let mut p = [0u8; PACKET_BYTES];
    p[0] = CMD_READ_TOC;
    p[8] = 24;
    send_packet(&mut ide, &p);
    assert_eq!(status(&mut ide) & ATA_SR_ERR, ATA_SR_ERR);
    let (key, asc, _) = ide.atapi_sense();
    assert_eq!(key, SENSE_NOT_READY);
    assert_eq!(asc, ASC_MEDIUM_NOT_PRESENT);
}

/// Unsupported TOC format → INVALID FIELD.
#[test]
fn read_toc_rejects_unsupported_format() {
    let mut ide = cdrom_with(8);
    start_packet(&mut ide, 512);
    let mut p = [0u8; PACKET_BYTES];
    p[0] = CMD_READ_TOC;
    p[2] = 0x02; // raw TOC — not implemented
    p[8] = 24;
    send_packet(&mut ide, &p);
    assert_eq!(status(&mut ide) & ATA_SR_ERR, ATA_SR_ERR);
    let (key, asc, _) = ide.atapi_sense();
    assert_eq!(key, SENSE_ILLEGAL_REQUEST);
    assert_eq!(asc, ASC_INVALID_FIELD);
}

/// Spec: SFF-8020i §9.8.26 — LoEj=1 Start=0 ejects; TUR becomes NOT READY.
#[test]
fn start_stop_eject_makes_tur_not_ready() {
    let mut ide = cdrom_with(4);
    assert!(ide.atapi_medium_loaded());
    assert_eq!(tur(&mut ide), (0, 0));

    start_packet(&mut ide, 512);
    let mut p = [0u8; PACKET_BYTES];
    p[0] = CMD_START_STOP_UNIT;
    p[4] = 0x02; // LoEj=1, Start=0 → eject
    send_packet(&mut ide, &p);
    assert_eq!(status(&mut ide) & ATA_SR_ERR, 0);
    assert!(!ide.atapi_medium_loaded());

    let (key, asc) = tur(&mut ide);
    assert_eq!(key, SENSE_NOT_READY);
    assert_eq!(asc, ASC_MEDIUM_NOT_PRESENT);
}

/// Stop/start without LoEj leave medium loaded.
#[test]
fn start_stop_without_loej_keeps_medium() {
    let mut ide = cdrom_with(2);
    start_packet(&mut ide, 512);
    let mut stop = [0u8; PACKET_BYTES];
    stop[0] = CMD_START_STOP_UNIT;
    stop[4] = 0x00; // stop
    send_packet(&mut ide, &stop);
    assert!(ide.atapi_medium_loaded());

    start_packet(&mut ide, 512);
    let mut start = [0u8; PACKET_BYTES];
    start[0] = CMD_START_STOP_UNIT;
    start[4] = 0x01; // start
    send_packet(&mut ide, &start);
    assert!(ide.atapi_medium_loaded());
    assert_eq!(tur(&mut ide), (0, 0));
}

/// Minimal type-`1Fh` device rejects both opcodes.
#[test]
fn minimal_packet_rejects_toc_and_start_stop() {
    let mut ide = IdePrimary::with_atapi_device();
    ide.port_write(IDE_PRIMARY_CTRL, 1, 0);
    for op in [CMD_READ_TOC, CMD_START_STOP_UNIT] {
        start_packet(&mut ide, 512);
        let mut p = [0u8; PACKET_BYTES];
        p[0] = op;
        p[8] = 24;
        send_packet(&mut ide, &p);
        let (key, asc, _) = ide.atapi_sense();
        assert_eq!(key, SENSE_ILLEGAL_REQUEST, "op {op:#04x}");
        assert_eq!(asc, ASC_INVALID_COMMAND, "op {op:#04x}");
    }
}
