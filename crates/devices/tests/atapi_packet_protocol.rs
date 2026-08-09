//! ATAPI PACKET (`A0h`) protocol on a configured packet device.
//!
//! # Spec refs
//!
//! - ATA/ATAPI-6 (T13/1410D r3b) §8.21 PACKET — §8.21.2 "use prohibited for
//!   devices not implementing the PACKET Command feature set"; §8.21.4 inputs:
//!   the Features register DMA (bit 0) and OVL (bit 1) bits, and the Byte Count
//!   Limit in Cylinder Low / Cylinder High, "the maximum number of bytes that
//!   may be transferred in a single DRQ data block"; §8.21.5 normal outputs;
//!   §8.21.6 error outputs, where the Error register carries the Sense Key in
//!   bits (7:4) and ABRT in bit 2.
//! - ATA/ATAPI-6 §9.8 "PACKET command protocol" — the command-packet transfer
//!   under DRQ, the optional data transfer in Byte Count Limit sized blocks
//!   with one INTRQ per block, and command completion.
//! - ATA/ATAPI-6 §7.13 Interrupt Reason register (the Sector Count register on
//!   a PACKET device): bit 0 C/D, bit 1 I/O, bit 2 REL.
//! - ATA/ATAPI-6 §8.16.9 IDENTIFY PACKET DEVICE word 0 — bits (1:0) `00b`
//!   selects a 12-byte command packet; bits (6:5) `00b` selects the "DRQ within
//!   3 ms" response, which does **not** assert INTRQ for the command packet.
//! - SFF-8020i §10.8.4 INQUIRY (EVPD, page code, allocation length, and the
//!   36-byte standard INQUIRY data) and §10.8.24 TEST UNIT READY.

use devices::{
    IdePrimary, IdeSecondary, PortDevice, ATA_CMD_PACKET, ATA_DC_NIEN, ATA_ER_ABRT, ATA_SR_BSY,
    ATA_SR_DRDY, ATA_SR_DRQ, ATA_SR_ERR, IDE_PRIMARY_CTRL, IDE_PRIMARY_DATA, IDE_PRIMARY_ERROR,
    IDE_PRIMARY_LBA_HI, IDE_PRIMARY_LBA_MID, IDE_PRIMARY_SECCOUNT, IDE_PRIMARY_STATUS,
    IDE_SECONDARY_CTRL, IDE_SECONDARY_DATA, IDE_SECONDARY_LBA_HI, IDE_SECONDARY_LBA_MID,
    IDE_SECONDARY_SECCOUNT, IDE_SECONDARY_STATUS,
};

/// Interrupt Reason C/D. Spec: ATA/ATAPI-6 §7.13.
const IR_CD: u32 = 0x01;
/// Interrupt Reason I/O. Spec: ATA/ATAPI-6 §7.13.
const IR_IO: u32 = 0x02;
/// Interrupt Reason REL. Spec: ATA/ATAPI-6 §7.13.
const IR_REL: u32 = 0x04;
/// Features DMA bit. Spec: ATA/ATAPI-6 §8.21.4.
const FEATURE_DMA: u32 = 0x01;
/// Features OVL bit. Spec: ATA/ATAPI-6 §8.21.4.
const FEATURE_OVL: u32 = 0x02;
/// `TEST UNIT READY`. Spec: SFF-8020i §10.8.24.
const CMD_TEST_UNIT_READY: u8 = 0x00;
/// `INQUIRY`. Spec: SFF-8020i §10.8.4.
const CMD_INQUIRY: u8 = 0x12;
/// `READ (10)` — deliberately unimplemented. Spec: SFF-8020i §10.8.13.
const CMD_READ_10: u8 = 0x28;
/// Sense key ILLEGAL REQUEST. Spec: SFF-8020i "Sense Key Definitions".
const SENSE_ILLEGAL_REQUEST: u8 = 0x05;
/// ASC INVALID COMMAND OPERATION CODE. Spec: SFF-8020i.
const ASC_INVALID_COMMAND_OPERATION_CODE: u8 = 0x20;
/// ASC INVALID FIELD IN COMMAND PACKET. Spec: SFF-8020i.
const ASC_INVALID_FIELD_IN_CDB: u8 = 0x24;
/// Command packet length. Spec: ATA/ATAPI-6 §8.16.9 word 0 bits (1:0) = `00b`.
const PACKET_BYTES: usize = 12;
/// Standard INQUIRY data length. Spec: SFF-8020i §10.8.4.
const INQUIRY_BYTES: usize = 36;

fn atapi() -> IdePrimary {
    let mut ide = IdePrimary::with_atapi_device();
    // Device Control nIEN clear so INTRQ reaches `irq_line`.
    ide.port_write(IDE_PRIMARY_CTRL, 1, 0);
    ide
}

/// Latch a Byte Count Limit and issue PACKET.
fn start_packet(ide: &mut IdePrimary, byte_count_limit: u16) {
    ide.port_write(IDE_PRIMARY_LBA_MID, 1, u32::from(byte_count_limit & 0xFF));
    ide.port_write(IDE_PRIMARY_LBA_HI, 1, u32::from(byte_count_limit >> 8));
    ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_PACKET));
}

/// Write a 12-byte command packet as six 16-bit Data register cycles.
fn send_packet(ide: &mut IdePrimary, packet: &[u8; PACKET_BYTES]) {
    for pair in packet.chunks(2) {
        let word = u32::from(pair[0]) | (u32::from(pair[1]) << 8);
        ide.port_write(IDE_PRIMARY_DATA, 2, word);
    }
}

fn packet_for(op: u8, allocation_length: u8) -> [u8; PACKET_BYTES] {
    let mut packet = [0u8; PACKET_BYTES];
    packet[0] = op;
    packet[4] = allocation_length;
    packet
}

/// Drain `len` bytes of a data-in block with 16-bit Data register reads.
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

fn byte_count(ide: &mut IdePrimary) -> u16 {
    let low = ide.port_read(IDE_PRIMARY_LBA_MID, 1) as u16;
    let high = ide.port_read(IDE_PRIMARY_LBA_HI, 1) as u16;
    (high << 8) | low
}

/// Spec: ATA/ATAPI-6 §9.8 — after PACKET the device sets DRQ with the Interrupt
/// Reason reporting C/D = 1, I/O = 0, REL = 0, and (with word 0 bits 6:5 = `00b`)
/// does **not** assert INTRQ for the command-packet phase.
#[test]
fn packet_enters_the_command_phase_without_an_interrupt() {
    let mut ide = atapi();
    start_packet(&mut ide, 512);

    assert_eq!(status(&mut ide) & ATA_SR_DRQ, ATA_SR_DRQ);
    assert_eq!(status(&mut ide) & ATA_SR_BSY, 0);
    assert_eq!(status(&mut ide) & ATA_SR_ERR, 0);
    assert_eq!(ide.port_read(IDE_PRIMARY_SECCOUNT, 1), IR_CD);
    assert_eq!(ide.port_read(IDE_PRIMARY_SECCOUNT, 1) & IR_IO, 0);
    assert_eq!(ide.port_read(IDE_PRIMARY_SECCOUNT, 1) & IR_REL, 0);
    assert!(!ide.irq_line(), "no INTRQ for the command packet DRQ");
}

/// Spec: ATA/ATAPI-6 §9.8 / §8.21.5 and SFF-8020i §10.8.24 — a non-data packet
/// command completes with BSY and DRQ clear, C/D = 1 and I/O = 1, no error, and
/// one INTRQ.
#[test]
fn test_unit_ready_completes_with_command_complete_and_intrq() {
    let mut ide = atapi();
    start_packet(&mut ide, 512);
    send_packet(&mut ide, &packet_for(CMD_TEST_UNIT_READY, 0));

    assert_eq!(status(&mut ide) & (ATA_SR_BSY | ATA_SR_DRQ | ATA_SR_ERR), 0);
    assert_eq!(status(&mut ide) & ATA_SR_DRDY, ATA_SR_DRDY);
    assert_eq!(ide.port_read(IDE_PRIMARY_SECCOUNT, 1), IR_CD | IR_IO);
    assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1), 0);
    assert!(ide.irq_line());

    // Spec: reading Status (not Alternate Status) clears the pending interrupt.
    ide.port_read(IDE_PRIMARY_STATUS, 1);
    assert!(!ide.irq_line());
}

/// Spec: SFF-8020i §10.8.4 — the 36-byte standard INQUIRY data, with a
/// peripheral device type that matches what IDENTIFY PACKET DEVICE claims.
#[test]
fn inquiry_returns_the_standard_thirty_six_byte_data() {
    let mut ide = atapi();
    start_packet(&mut ide, 512);
    send_packet(&mut ide, &packet_for(CMD_INQUIRY, INQUIRY_BYTES as u8));

    // Spec: §9.8 — data-in block: C/D = 0, I/O = 1, DRQ set, INTRQ asserted,
    // and the actual byte count in Cylinder Low/High.
    assert_eq!(ide.port_read(IDE_PRIMARY_SECCOUNT, 1), IR_IO);
    assert_eq!(status(&mut ide) & ATA_SR_DRQ, ATA_SR_DRQ);
    assert_eq!(byte_count(&mut ide), INQUIRY_BYTES as u16);
    assert!(ide.irq_line());
    ide.port_read(IDE_PRIMARY_STATUS, 1);

    let data = read_bytes(&mut ide, INQUIRY_BYTES);
    // Byte 0: qualifier 000b + peripheral device type 1Fh "unknown or no
    // device type" — this device is not a CD-ROM and does not say it is.
    assert_eq!(data[0], 0x1F);
    // Byte 1 bit 7 RMB clear: not removable, matching identify word 0 bit 7.
    assert_eq!(data[1] & 0x80, 0);
    // Byte 2: no ANSI version claimed.
    assert_eq!(data[2], 0x00);
    // Byte 3: response data format.
    assert_eq!(data[3] & 0x0F, 0x02);
    // Byte 4: additional length = 36 - 5.
    assert_eq!(data[4], 31);
    assert_eq!(&data[8..16], b"x86WASM ");
    assert_eq!(&data[16..32], b"ATAPI PACKET MIN");
    assert_eq!(&data[32..36], b"0001");

    // Spec: §9.8 — command completion after the last block.
    assert_eq!(status(&mut ide) & (ATA_SR_DRQ | ATA_SR_ERR), 0);
    assert_eq!(ide.port_read(IDE_PRIMARY_SECCOUNT, 1), IR_CD | IR_IO);
    assert!(ide.irq_line());
}

/// Spec: ATA/ATAPI-6 §8.21.4 / §9.8 — a transfer longer than the Byte Count
/// Limit is split into blocks of at most that many bytes, each reporting its own
/// byte count and asserting INTRQ.
#[test]
fn the_byte_count_limit_splits_the_transfer_into_drq_blocks() {
    let mut ide = atapi();
    start_packet(&mut ide, 16);
    send_packet(&mut ide, &packet_for(CMD_INQUIRY, INQUIRY_BYTES as u8));

    let mut collected = Vec::new();
    for expected in [16u16, 16, 4] {
        assert_eq!(ide.port_read(IDE_PRIMARY_SECCOUNT, 1), IR_IO);
        assert_eq!(status(&mut ide) & ATA_SR_DRQ, ATA_SR_DRQ);
        assert_eq!(byte_count(&mut ide), expected);
        assert!(ide.irq_line(), "INTRQ per DRQ block");
        ide.port_read(IDE_PRIMARY_STATUS, 1);
        collected.extend(read_bytes(&mut ide, usize::from(expected)));
    }

    assert_eq!(collected.len(), INQUIRY_BYTES);
    assert_eq!(collected[0], 0x1F);
    assert_eq!(&collected[16..32], b"ATAPI PACKET MIN");
    assert_eq!(status(&mut ide) & ATA_SR_DRQ, 0);
    assert_eq!(ide.port_read(IDE_PRIMARY_SECCOUNT, 1), IR_CD | IR_IO);
}

/// Spec: SFF-8020i — a packet command transfers no more than its allocation
/// length.
#[test]
fn the_allocation_length_truncates_the_transfer() {
    let mut ide = atapi();
    start_packet(&mut ide, 512);
    send_packet(&mut ide, &packet_for(CMD_INQUIRY, 5));

    assert_eq!(byte_count(&mut ide), 5);
    let data = read_bytes(&mut ide, 5);
    assert_eq!(data, vec![0x1F, 0x00, 0x00, 0x02, 31]);
    assert_eq!(status(&mut ide) & ATA_SR_DRQ, 0);
    assert_eq!(ide.port_read(IDE_PRIMARY_SECCOUNT, 1), IR_CD | IR_IO);
}

/// Spec: SFF-8020i — an allocation length of zero transfers nothing and is not
/// an error, so the command completes with no data phase at all.
#[test]
fn a_zero_allocation_length_completes_without_a_data_phase() {
    let mut ide = atapi();
    start_packet(&mut ide, 512);
    send_packet(&mut ide, &packet_for(CMD_INQUIRY, 0));

    assert_eq!(status(&mut ide) & (ATA_SR_DRQ | ATA_SR_ERR), 0);
    assert_eq!(ide.port_read(IDE_PRIMARY_SECCOUNT, 1), IR_CD | IR_IO);
    assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1), 0);
}

/// Spec: ATA/ATAPI-6 §8.21.6 and SFF-8020i — an unimplemented operation code is
/// CHECK CONDITION with sense key ILLEGAL REQUEST and ASC `20h`; the Error
/// register carries the sense key in bits (7:4) with ABRT set.
#[test]
fn an_unimplemented_packet_command_reports_check_condition() {
    let mut ide = atapi();
    start_packet(&mut ide, 512);
    send_packet(&mut ide, &packet_for(CMD_READ_10, 0));

    assert_eq!(status(&mut ide) & ATA_SR_ERR, ATA_SR_ERR);
    assert_eq!(status(&mut ide) & ATA_SR_DRQ, 0);
    assert_eq!(
        ide.port_read(IDE_PRIMARY_ERROR, 1) as u8,
        (SENSE_ILLEGAL_REQUEST << 4) | ATA_ER_ABRT
    );
    assert_eq!(ide.port_read(IDE_PRIMARY_SECCOUNT, 1), IR_CD | IR_IO);
    assert_eq!(
        ide.atapi_sense(),
        (SENSE_ILLEGAL_REQUEST, ASC_INVALID_COMMAND_OPERATION_CODE, 0)
    );
    assert!(ide.irq_line());
}

/// Spec: SFF-8020i §10.8.4 — no vital product data pages are implemented, so
/// EVPD or a non-zero page code is an invalid command-packet field.
#[test]
fn inquiry_with_evpd_or_a_page_code_is_an_invalid_field() {
    for (byte1, byte2) in [(0x01u8, 0x00u8), (0x00, 0x83)] {
        let mut ide = atapi();
        start_packet(&mut ide, 512);
        let mut packet = packet_for(CMD_INQUIRY, INQUIRY_BYTES as u8);
        packet[1] = byte1;
        packet[2] = byte2;
        send_packet(&mut ide, &packet);

        assert_eq!(status(&mut ide) & ATA_SR_ERR, ATA_SR_ERR);
        assert_eq!(
            ide.atapi_sense(),
            (SENSE_ILLEGAL_REQUEST, ASC_INVALID_FIELD_IN_CDB, 0)
        );
    }
}

/// Spec: ATA/ATAPI-6 §8.21.4 — DMA and overlap are not implemented (IDENTIFY
/// PACKET DEVICE words 0 and 49 say so), so requesting either aborts the
/// command before any packet transfer.
#[test]
fn the_dma_and_overlap_feature_bits_abort_the_command() {
    for feature in [FEATURE_DMA, FEATURE_OVL, FEATURE_DMA | FEATURE_OVL] {
        let mut ide = atapi();
        ide.port_write(IDE_PRIMARY_ERROR, 1, feature);
        start_packet(&mut ide, 512);

        assert_eq!(status(&mut ide) & ATA_SR_ERR, ATA_SR_ERR);
        assert_eq!(status(&mut ide) & ATA_SR_DRQ, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_ER_ABRT);
        assert!(ide.irq_line());
    }
}

/// Model choice, recorded because ATA/ATAPI-6 §8.21.4 leaves it indeterminate
/// rather than defining it: an odd Byte Count Limit is rounded down to even, and
/// a limit that is zero — or rounds to zero — aborts instead of transferring an
/// indeterminate amount.
#[test]
fn the_byte_count_limit_is_rounded_down_and_zero_aborts() {
    let mut ide = atapi();
    start_packet(&mut ide, 17);
    send_packet(&mut ide, &packet_for(CMD_INQUIRY, INQUIRY_BYTES as u8));
    assert_eq!(byte_count(&mut ide), 16);

    for limit in [0u16, 1] {
        let mut ide = atapi();
        start_packet(&mut ide, limit);
        assert_eq!(status(&mut ide) & ATA_SR_ERR, ATA_SR_ERR, "limit {limit}");
        assert_eq!(status(&mut ide) & ATA_SR_DRQ, 0, "limit {limit}");
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_ER_ABRT);
    }
}

/// Spec: ATA/ATAPI-6 §8.21.2 — "use prohibited for devices not implementing the
/// PACKET Command feature set".
#[test]
fn packet_is_still_aborted_on_an_ata_disk() {
    let mut ide = IdePrimary::with_image(vec![0u8; 512 * 4]);
    ide.port_write(IDE_PRIMARY_CTRL, 1, 0);
    start_packet(&mut ide, 512);

    assert_eq!(status(&mut ide) & ATA_SR_ERR, ATA_SR_ERR);
    assert_eq!(status(&mut ide) & ATA_SR_DRQ, 0);
    assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_ER_ABRT);
}

/// Spec: ATA/ATAPI-6 §9.10 / §9.11 — a software reset ends any command in
/// progress and restores the PACKET-device reset state.
#[test]
fn a_software_reset_drops_an_in_progress_packet_transfer() {
    let mut ide = atapi();
    start_packet(&mut ide, 16);
    send_packet(&mut ide, &packet_for(CMD_INQUIRY, INQUIRY_BYTES as u8));
    assert_eq!(status(&mut ide) & ATA_SR_DRQ, ATA_SR_DRQ);

    // SRST high then low.
    ide.port_write(IDE_PRIMARY_CTRL, 1, u32::from(devices::ATA_DC_SRST));
    ide.port_write(IDE_PRIMARY_CTRL, 1, 0);

    // Spec: §9.10 — a PACKET device clears Status bits 6,5,4,3,2 and 0.
    assert_eq!(status(&mut ide), 0);
    // Spec: §9.12 — the PACKET signature is back in the Command Block.
    assert_eq!(ide.port_read(IDE_PRIMARY_SECCOUNT, 1), 0x01);
    assert_eq!(byte_count(&mut ide), 0xEB14);
    // The Data register no longer feeds the abandoned transfer.
    assert_eq!(ide.port_read(IDE_PRIMARY_DATA, 2), 0xFFFF_FFFF);
}

/// Spec: ATA/ATAPI-6 §5.2.9 — nIEN gates INTRQ, so a packet transfer runs
/// identically with interrupts masked, just without the line asserting.
#[test]
fn nien_masks_the_packet_interrupts_without_changing_the_transfer() {
    let mut ide = IdePrimary::with_atapi_device();
    ide.port_write(IDE_PRIMARY_CTRL, 1, u32::from(ATA_DC_NIEN));
    start_packet(&mut ide, 512);
    send_packet(&mut ide, &packet_for(CMD_INQUIRY, INQUIRY_BYTES as u8));

    assert_eq!(byte_count(&mut ide), INQUIRY_BYTES as u16);
    assert!(!ide.irq_line());
    let data = read_bytes(&mut ide, INQUIRY_BYTES);
    assert_eq!(data[0], 0x1F);
    assert!(!ide.irq_line());
    assert_eq!(ide.port_read(IDE_PRIMARY_SECCOUNT, 1), IR_CD | IR_IO);
}

/// The secondary channel is the same engine on remapped ports.
///
/// Spec: OSDev ATA PIO — secondary command block `0x170`–`0x177`, control
/// `0x376`.
#[test]
fn the_secondary_channel_runs_the_same_packet_protocol() {
    let mut ide = IdeSecondary::with_atapi_device();
    ide.port_write(IDE_SECONDARY_CTRL, 1, 0);
    ide.port_write(IDE_SECONDARY_LBA_MID, 1, 0x00);
    ide.port_write(IDE_SECONDARY_LBA_HI, 1, 0x02);
    ide.port_write(IDE_SECONDARY_STATUS, 1, u32::from(ATA_CMD_PACKET));
    assert_eq!(ide.port_read(IDE_SECONDARY_SECCOUNT, 1), IR_CD);

    let packet = packet_for(CMD_INQUIRY, INQUIRY_BYTES as u8);
    for pair in packet.chunks(2) {
        let word = u32::from(pair[0]) | (u32::from(pair[1]) << 8);
        ide.port_write(IDE_SECONDARY_DATA, 2, word);
    }
    assert_eq!(ide.port_read(IDE_SECONDARY_SECCOUNT, 1), IR_IO);
    assert!(ide.irq_line(), "secondary channel INTRQ is IRQ15");

    let first = ide.port_read(IDE_SECONDARY_DATA, 2);
    assert_eq!(first & 0xFF, 0x1F);
}
