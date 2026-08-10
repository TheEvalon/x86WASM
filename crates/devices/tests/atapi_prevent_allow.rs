//! ATAPI PREVENT/ALLOW MEDIUM REMOVAL for CD-ROM capable PACKET devices.
//!
//! # Spec refs
//!
//! - SFF-8020i §10.8.11 PREVENT/ALLOW MEDIUM REMOVAL (`1Eh`) — Prevent bit;
//!   unlock on Prevent=0; hard reset clears lock.
//! - SFF-8020i Table 84 — eject while locked → NOT READY / ASC `53h`
//!   MEDIA REMOVAL PREVENTED (this model uses ASCQ `00h`).
//! - SFF-8020i §10.8.25 START/STOP UNIT — LoEj eject respects the lock.
//! - Round-5 type `1Fh` minimal PACKET path rejects the opcode.

use devices::{
    IdePrimary, PortDevice, ATA_CMD_DEVICE_RESET, ATA_CMD_PACKET, ATA_SR_ERR, IDE_PRIMARY_CTRL,
    IDE_PRIMARY_DATA, IDE_PRIMARY_LBA_HI, IDE_PRIMARY_LBA_MID, IDE_PRIMARY_STATUS,
};

const PACKET_BYTES: usize = 12;
const CMD_PREVENT_ALLOW: u8 = 0x1E;
const CMD_START_STOP: u8 = 0x1B;
const SENSE_NOT_READY: u8 = 0x02;
const SENSE_ILLEGAL_REQUEST: u8 = 0x05;
const ASC_INVALID_COMMAND: u8 = 0x20;
const ASC_MEDIUM_REMOVAL_PREVENTED: u8 = 0x53;
const CDROM_BLOCK: usize = 2048;

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

fn status(ide: &mut IdePrimary) -> u8 {
    ide.port_read(IDE_PRIMARY_CTRL, 1) as u8
}

fn prevent(ide: &mut IdePrimary, locked: bool) {
    start_packet(ide, 512);
    let mut p = [0u8; PACKET_BYTES];
    p[0] = CMD_PREVENT_ALLOW;
    p[4] = u8::from(locked);
    send_packet(ide, &p);
}

fn eject(ide: &mut IdePrimary) {
    start_packet(ide, 512);
    let mut p = [0u8; PACKET_BYTES];
    p[0] = CMD_START_STOP;
    p[4] = 0x02; // LoEj, !Start
    send_packet(ide, &p);
}

/// Spec: SFF-8020i §10.8.11 — Prevent=1 locks; subsequent eject fails.
#[test]
fn prevent_blocks_start_stop_eject() {
    let mut ide = cdrom_with(2);
    assert!(ide.atapi_medium_loaded());
    prevent(&mut ide, true);
    assert_eq!(status(&mut ide) & ATA_SR_ERR, 0);
    assert!(ide.atapi_removal_prevented());

    eject(&mut ide);
    assert_eq!(status(&mut ide) & ATA_SR_ERR, ATA_SR_ERR);
    let (key, asc, _) = ide.atapi_sense();
    assert_eq!(key, SENSE_NOT_READY);
    assert_eq!(asc, ASC_MEDIUM_REMOVAL_PREVENTED);
    assert!(ide.atapi_medium_loaded());
}

/// Spec: SFF-8020i — Prevent=0 unlocks; eject then succeeds.
#[test]
fn allow_then_eject_unloads() {
    let mut ide = cdrom_with(1);
    prevent(&mut ide, true);
    prevent(&mut ide, false);
    assert!(!ide.atapi_removal_prevented());
    eject(&mut ide);
    assert_eq!(status(&mut ide) & ATA_SR_ERR, 0);
    assert!(!ide.atapi_medium_loaded());
}

/// Spec: SFF-8020i §10.8.11 — hard / device reset clears the lock.
#[test]
fn device_reset_clears_prevent() {
    let mut ide = cdrom_with(1);
    prevent(&mut ide, true);
    assert!(ide.atapi_removal_prevented());
    ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_DEVICE_RESET));
    assert!(!ide.atapi_removal_prevented());
}

/// Unlocked eject still works (slice-2 regression).
#[test]
fn unlocked_eject_still_unloads() {
    let mut ide = cdrom_with(3);
    eject(&mut ide);
    assert!(!ide.atapi_medium_loaded());
}

/// Minimal type-`1Fh` device rejects PREVENT/ALLOW as unknown opcode.
#[test]
fn minimal_packet_rejects_prevent_allow() {
    let mut ide = IdePrimary::with_atapi_device();
    ide.port_write(IDE_PRIMARY_CTRL, 1, 0);
    prevent(&mut ide, true);
    let (key, asc, _) = ide.atapi_sense();
    assert_eq!(key, SENSE_ILLEGAL_REQUEST);
    assert_eq!(asc, ASC_INVALID_COMMAND);
}
