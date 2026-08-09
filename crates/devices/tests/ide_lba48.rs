//! ATA 48-bit Address feature set: READ SECTOR(S) EXT / WRITE SECTOR(S) EXT.
//!
//! # Spec refs
//!
//! - ATA/ATAPI-6 (T13/1410D r3b) §6.20 "48-bit Address feature set" — the
//!   Features, Sector Count, LBA Low, LBA Mid and LBA High registers are each a
//!   two-byte deep FIFO; a write puts the value in "most recently written" and
//!   moves the old value to "previous content". Table 11 maps the pair to
//!   Sector Count (7:0)/(15:8) and LBA (7:0)(15:8)(23:16) / (31:24)(39:32)(47:40),
//!   requires the Device register LBA bit to be set, and marks Device bits 3:0
//!   reserved. The host reads "previous content" by setting the HOB bit (bit 7)
//!   of the Device Control register; "A write to any Command Block register
//!   shall cause the device to clear the HOB bit to zero".
//! - ATA/ATAPI-6 §8.35 READ SECTOR(S) EXT (`24h`) — PIO data-in, 1 to 65,536
//!   sectors, Sector Count `0000h` = 65,536; "The DRQ bit is always set to one
//!   prior to data transfer... The device shall interrupt for each DRQ block
//!   transferred." Error outputs: IDNF "if an address outside of the range of
//!   user-accessible addresses is requested if command aborted is not
//!   returned".
//! - ATA/ATAPI-6 §8.63 WRITE SECTOR(S) EXT (`34h`) — PIO data-out, same
//!   addressing, count and interrupt rules.
//! - ATA/ATAPI-6 §6.2.1 / §6.20 IDENTIFY DEVICE — word 83 bit 10 and word 86
//!   bit 10 report 48-bit Address feature set support; words (103:100) hold the
//!   48-bit user-addressable sector count; words (61:60) are capped at
//!   268,435,455.

use devices::{
    IdePrimary, PortDevice, ATA_CMD_IDENTIFY, ATA_CMD_READ_SECTORS_EXT, ATA_CMD_WRITE_SECTORS_EXT,
    ATA_DC_HOB, ATA_DRIVE_LBA, ATA_DRIVE_SLAVE, ATA_ER_ABRT, ATA_ER_IDNF, ATA_SR_DRDY, ATA_SR_DRQ,
    ATA_SR_DSC, ATA_SR_ERR, IDE_PRIMARY_CTRL, IDE_PRIMARY_DATA, IDE_PRIMARY_DRIVE,
    IDE_PRIMARY_ERROR, IDE_PRIMARY_LBA_HI, IDE_PRIMARY_LBA_LO, IDE_PRIMARY_LBA_MID,
    IDE_PRIMARY_SECCOUNT, IDE_PRIMARY_STATUS,
};

/// READ SECTOR(S) EXT. Spec: ATA/ATAPI-6 §8.35.1.
const CMD_READ_SECTORS_EXT: u32 = ATA_CMD_READ_SECTORS_EXT as u32;
/// WRITE SECTOR(S) EXT. Spec: ATA/ATAPI-6 §8.63.1.
const CMD_WRITE_SECTORS_EXT: u32 = ATA_CMD_WRITE_SECTORS_EXT as u32;
/// Device Control HOB (high order byte). Spec: ATA/ATAPI-6 §7.8.6 / §6.20.
const DC_HOB: u32 = ATA_DC_HOB as u32;
/// Error register IDNF (bit 4). Spec: ATA/ATAPI-6 §8.35.6 error outputs.
const ER_IDNF: u8 = ATA_ER_IDNF;
/// Device 0 with LBA addressing.
const DEV0_LBA: u32 = 0xA0 | ATA_DRIVE_LBA as u32;

const SECTOR: usize = 512;

fn drive(image_sectors: usize) -> IdePrimary {
    let mut img = vec![0u8; SECTOR * image_sectors];
    for lba in 0..image_sectors {
        img[lba * SECTOR] = lba as u8;
        img[lba * SECTOR + 1] = 0xA0 | (lba as u8);
    }
    let mut ide = IdePrimary::with_image(img);
    // Clear nIEN so INTRQ is observable.
    ide.port_write(IDE_PRIMARY_CTRL, 1, 0);
    ide
}

/// Program the two-deep FIFOs for a 48-bit command (§6.20 Table 11): the
/// high-order byte is written first, then the low-order byte.
fn program_ext(ide: &mut IdePrimary, lba: u64, count: u16) {
    ide.port_write(IDE_PRIMARY_DRIVE, 1, DEV0_LBA);
    ide.port_write(IDE_PRIMARY_SECCOUNT, 1, u32::from(count >> 8));
    ide.port_write(IDE_PRIMARY_SECCOUNT, 1, u32::from(count & 0xFF));
    ide.port_write(IDE_PRIMARY_LBA_LO, 1, ((lba >> 24) & 0xFF) as u32);
    ide.port_write(IDE_PRIMARY_LBA_LO, 1, (lba & 0xFF) as u32);
    ide.port_write(IDE_PRIMARY_LBA_MID, 1, ((lba >> 32) & 0xFF) as u32);
    ide.port_write(IDE_PRIMARY_LBA_MID, 1, ((lba >> 8) & 0xFF) as u32);
    ide.port_write(IDE_PRIMARY_LBA_HI, 1, ((lba >> 40) & 0xFF) as u32);
    ide.port_write(IDE_PRIMARY_LBA_HI, 1, ((lba >> 16) & 0xFF) as u32);
}

fn drain_sector(ide: &mut IdePrimary) -> Vec<u16> {
    (0..SECTOR / 2)
        .map(|_| ide.port_read(IDE_PRIMARY_DATA, 2) as u16)
        .collect()
}

/// §8.35: READ SECTOR(S) EXT presents one DRQ block per sector and interrupts
/// for each block; completion clears DRQ with no error.
#[test]
fn read_sectors_ext_two_sector_pio_with_irq_per_block() {
    let mut ide = drive(4);
    program_ext(&mut ide, 1, 2);
    ide.port_write(IDE_PRIMARY_STATUS, 1, CMD_READ_SECTORS_EXT);

    assert!(ide.irq_line(), "INTRQ for the first DRQ block");
    let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
    assert_ne!(st & ATA_SR_DRQ, 0);
    assert_eq!(st & ATA_SR_ERR, 0);
    assert!(!ide.irq_line(), "Status read clears interrupt pending");

    let s1 = drain_sector(&mut ide);
    assert_eq!(s1[0], 0xA101);
    assert!(ide.irq_line(), "INTRQ for the second DRQ block");
    assert_ne!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);

    let s2 = drain_sector(&mut ide);
    assert_eq!(s2[0], 0xA202);
    assert!(ide.irq_line(), "INTRQ on command completion");
    let done = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
    assert_eq!(done, ATA_SR_DRDY | ATA_SR_DSC);
    assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
}

/// §6.20 Table 11: Device register bits 3:0 are reserved for 48-bit commands
/// and must not contribute to the address (unlike the LBA28 path).
#[test]
fn read_sectors_ext_ignores_device_register_low_nibble() {
    let mut ide = drive(4);
    program_ext(&mut ide, 2, 1);
    // Dirty bits 3:0 without touching the two-deep FIFOs' meaning.
    ide.port_write(IDE_PRIMARY_DRIVE, 1, DEV0_LBA | 0x0F);
    ide.port_write(IDE_PRIMARY_STATUS, 1, CMD_READ_SECTORS_EXT);
    assert_ne!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);
    assert_eq!(drain_sector(&mut ide)[0], 0xA202);
}

/// §6.20: HOB=1 reads the "previous content" half of the FIFO; a Command Block
/// register write clears HOB back to zero.
#[test]
fn hob_reads_previous_content_and_register_write_clears_hob() {
    let mut ide = drive(4);
    ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 0x11);
    ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 0x22);
    ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0x33);
    ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0x44);
    ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0x55);
    ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0x66);
    ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0x77);
    ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0x88);

    assert_eq!(ide.port_read(IDE_PRIMARY_SECCOUNT, 1) as u8, 0x22);
    assert_eq!(ide.port_read(IDE_PRIMARY_LBA_LO, 1) as u8, 0x44);
    assert_eq!(ide.port_read(IDE_PRIMARY_LBA_MID, 1) as u8, 0x66);
    assert_eq!(ide.port_read(IDE_PRIMARY_LBA_HI, 1) as u8, 0x88);

    ide.port_write(IDE_PRIMARY_CTRL, 1, DC_HOB);
    assert_eq!(ide.port_read(IDE_PRIMARY_SECCOUNT, 1) as u8, 0x11);
    assert_eq!(ide.port_read(IDE_PRIMARY_LBA_LO, 1) as u8, 0x33);
    assert_eq!(ide.port_read(IDE_PRIMARY_LBA_MID, 1) as u8, 0x55);
    assert_eq!(ide.port_read(IDE_PRIMARY_LBA_HI, 1) as u8, 0x77);

    // "A write to any Command Block register shall cause the device to clear
    // the HOB bit to zero in the Device Control register."
    ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 0x99);
    assert_eq!(
        ide.port_read(IDE_PRIMARY_SECCOUNT, 1) as u8,
        0x99,
        "HOB cleared by the register write"
    );
    assert_eq!(ide.port_read(IDE_PRIMARY_LBA_LO, 1) as u8, 0x44);
}

/// §8.35.6 / §6.2.2: an address at or beyond the user-addressable range is
/// reported with IDNF and ERR, and no DRQ block is presented.
#[test]
fn read_sectors_ext_out_of_range_sets_idnf() {
    let mut ide = drive(4);
    program_ext(&mut ide, 4, 1);
    ide.port_write(IDE_PRIMARY_STATUS, 1, CMD_READ_SECTORS_EXT);
    let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
    assert_ne!(st & ATA_SR_ERR, 0);
    assert_eq!(st & ATA_SR_DRQ, 0);
    assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ER_IDNF);
}

/// A range that starts in bounds but spills past the last sector is also IDNF
/// (checked before the transfer starts, so no partial DRQ block appears).
#[test]
fn read_sectors_ext_partial_spill_sets_idnf() {
    let mut ide = drive(4);
    program_ext(&mut ide, 3, 2);
    ide.port_write(IDE_PRIMARY_STATUS, 1, CMD_READ_SECTORS_EXT);
    let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
    assert_ne!(st & ATA_SR_ERR, 0);
    assert_eq!(st & ATA_SR_DRQ, 0);
    assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ER_IDNF);
}

/// §6.20 Table 11: the high-order LBA bytes really do address; a non-zero
/// LBA(47:24) is far past a small image and reports IDNF instead of aliasing
/// onto a low sector.
#[test]
fn read_sectors_ext_high_order_bytes_are_addressed() {
    let mut ide = drive(4);
    program_ext(&mut ide, 0x0000_0100_0000_0001, 1);
    ide.port_write(IDE_PRIMARY_STATUS, 1, CMD_READ_SECTORS_EXT);
    let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
    assert_ne!(st & ATA_SR_ERR, 0);
    assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ER_IDNF);
}

/// §8.35.8: "A sector count of 0000h requests 65,536 sectors." On a small image
/// that is out of range → IDNF (and must not be read as "one sector").
#[test]
fn sector_count_zero_requests_65536_sectors() {
    let mut ide = drive(4);
    program_ext(&mut ide, 0, 0);
    ide.port_write(IDE_PRIMARY_STATUS, 1, CMD_READ_SECTORS_EXT);
    let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
    assert_ne!(st & ATA_SR_ERR, 0, "65,536 sectors do not fit in 4 sectors");
    assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ER_IDNF);
}

/// §6.20: "The 48-bit Address feature set operates in LBA only." A CHS-style
/// Device register (LBA bit clear) aborts.
#[test]
fn ext_commands_require_the_lba_bit() {
    for cmd in [CMD_READ_SECTORS_EXT, CMD_WRITE_SECTORS_EXT] {
        let mut ide = drive(4);
        program_ext(&mut ide, 0, 1);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, cmd);
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_ne!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_ER_ABRT);
    }
}

/// §8.63: WRITE SECTOR(S) EXT accepts one DRQ block per sector and commits to
/// media; a READ SECTOR(S) EXT round-trips the data.
#[test]
fn write_sectors_ext_round_trips_two_sectors() {
    let mut ide = drive(4);
    program_ext(&mut ide, 2, 2);
    ide.port_write(IDE_PRIMARY_STATUS, 1, CMD_WRITE_SECTORS_EXT);
    assert!(ide.irq_line());
    assert_ne!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);

    for word in 0..SECTOR / 2 {
        ide.port_write(IDE_PRIMARY_DATA, 2, 0x1000 + word as u32);
    }
    assert_ne!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);
    for word in 0..SECTOR / 2 {
        ide.port_write(IDE_PRIMARY_DATA, 2, 0x2000 + word as u32);
    }
    let done = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
    assert_eq!(done, ATA_SR_DRDY | ATA_SR_DSC);
    assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);

    program_ext(&mut ide, 2, 2);
    ide.port_write(IDE_PRIMARY_STATUS, 1, CMD_READ_SECTORS_EXT);
    let s1 = drain_sector(&mut ide);
    let s2 = drain_sector(&mut ide);
    assert_eq!(s1[0], 0x1000);
    assert_eq!(s1[255], 0x10FF);
    assert_eq!(s2[0], 0x2000);
    assert_eq!(s2[255], 0x20FF);
}

/// §8.63.6: out-of-range WRITE SECTOR(S) EXT reports IDNF and writes nothing.
#[test]
fn write_sectors_ext_out_of_range_sets_idnf_and_writes_nothing() {
    let mut ide = drive(4);
    let before = ide.image.clone();
    program_ext(&mut ide, 4, 1);
    ide.port_write(IDE_PRIMARY_STATUS, 1, CMD_WRITE_SECTORS_EXT);
    let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
    assert_ne!(st & ATA_SR_ERR, 0);
    assert_eq!(st & ATA_SR_DRQ, 0);
    assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ER_IDNF);
    assert_eq!(ide.image, before);
}

/// §6.20 / §6.2.1: IDENTIFY DEVICE must only advertise 48-bit addressing when
/// the EXT commands work; words (103:100) carry the 48-bit capacity.
#[test]
fn identify_reports_lba48_support_and_capacity() {
    let mut ide = drive(4);
    ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
    ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY));
    let words: Vec<u16> = (0..256)
        .map(|_| ide.port_read(IDE_PRIMARY_DATA, 2) as u16)
        .collect();

    assert_eq!(words[83] & (1 << 10), 1 << 10, "word 83 bit10 = 48-bit");
    assert_eq!(words[86] & (1 << 10), 1 << 10, "word 86 bit10 = 48-bit");
    assert_eq!(words[83] & (1 << 14), 1 << 14, "word 83 bit14 shall be one");
    assert_eq!(words[60], 4);
    assert_eq!(words[61], 0);
    assert_eq!(words[100], 4);
    assert_eq!(words[101], 0);
    assert_eq!(words[102], 0);
    assert_eq!(words[103], 0);
}

/// The 48-bit commands obey ATA/ATAPI-6 §9.16.1 like every other command: a
/// Command register write while the absent Device 1 is selected is ignored.
#[test]
fn ext_command_to_absent_device1_is_ignored() {
    let mut ide = drive(4);
    program_ext(&mut ide, 0, 1);
    ide.port_write(IDE_PRIMARY_DRIVE, 1, DEV0_LBA | u32::from(ATA_DRIVE_SLAVE));
    ide.port_write(IDE_PRIMARY_STATUS, 1, CMD_READ_SECTORS_EXT);
    assert!(!ide.irq_line());
    ide.port_write(IDE_PRIMARY_DRIVE, 1, DEV0_LBA);
    let st = ide.port_read(IDE_PRIMARY_CTRL, 1) as u8;
    assert_eq!(st, ATA_SR_DRDY | ATA_SR_DSC);
}

/// A software reset clears the "previous content" halves along with the rest of
/// the task file, so a stale HOB byte cannot leak into the next command.
#[test]
fn software_reset_clears_previous_content_and_hob() {
    let mut ide = drive(4);
    ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0x7E);
    ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0x01);
    ide.port_write(IDE_PRIMARY_CTRL, 1, DC_HOB);
    assert_eq!(ide.port_read(IDE_PRIMARY_LBA_LO, 1) as u8, 0x7E);

    ide.port_write(IDE_PRIMARY_CTRL, 1, DC_HOB | 0x04); // SRST
    ide.port_write(IDE_PRIMARY_CTRL, 1, 0);
    ide.port_write(IDE_PRIMARY_CTRL, 1, DC_HOB);
    assert_eq!(ide.port_read(IDE_PRIMARY_LBA_LO, 1) as u8, 0x00);
    ide.port_write(IDE_PRIMARY_CTRL, 1, 0);
    // §9.12 signature is still in the "most recently written" half.
    assert_eq!(ide.port_read(IDE_PRIMARY_LBA_LO, 1) as u8, 0x01);
}
