//! 82077AA READ ID track ID-field scan.
//!
//! # Spec refs
//!
//! - Intel 82077AA CHMOS Single-Chip Floppy Disk Controller, READ ID (`0x0A`
//!   with MFM in bit 6; Table 5-1 / §5.1.8): "The READ ID command is used to
//!   find the present position of the recording heads. The 82077AA stores the
//!   values from the first ID Field it is able to read into its registers. If
//!   the 82077AA does not find an ID Address Mark on the diskette after the
//!   second occurrence of a pulse on the INDX# pin, then it sets the IC code in
//!   Status Register 0 to '01' (Abnormal termination), sets the MA bit in
//!   Status Register 1 to '1', and terminates the command."
//! - Intel 82077AA §6.2 Status Register 1 — bit 0 MA (Missing Address Mark),
//!   bit 2 ND (No Data / sector not found).
//! - One parameter byte (HD|US) and a 7-byte result ST0/ST1/ST2/C/H/R/N; READ
//!   ID is one of the control commands that generates an interrupt on
//!   completion (DOR bit 3 DMA/IRQ enable → ISA IRQ6).
//! - IBM PC / OSDev Floppy Disk — 1.44MB MFM geometry: 80 cylinders, 2 heads,
//!   18 sectors/track, N=2 (512-byte sectors).

use devices::{
    Fdc82077, PortDevice, FDC_1440_CYLINDERS, FDC_1440_IMAGE_SIZE, FDC_1440_SECTORS_PER_TRACK,
    FDC_CMD_READ_ID_MFM, FDC_CMD_SEEK, FDC_CMD_SENSE_INT, FDC_DOR, FDC_DOR_DMA_IRQ,
    FDC_DOR_RESET_N, FDC_DSR_SOFTWARE_RESET, FDC_FIFO, FDC_MSR, FDC_SECTOR_N, FDC_ST0_HEAD,
    FDC_ST0_IC_ABNORMAL, FDC_ST0_IC_NORMAL, FDC_ST1_MA, FDC_ST1_ND,
};

/// READ ID with the MFM modifier. Spec: Intel 82077AA Table 5-1 (`MFM|01010`).
const CMD_READ_ID_MFM: u32 = FDC_CMD_READ_ID_MFM as u32;
/// ST1 bit0 MA — Missing Address Mark. Spec: Intel 82077AA §6.2.
const ST1_MA: u8 = FDC_ST1_MA;
/// DSR software reset (self-clearing bit7). Spec: Intel 82077AA §2.1.5.
const DSR_SOFTWARE_RESET: u32 = FDC_DSR_SOFTWARE_RESET as u32;
/// Data Rate Select / Main Status port (`0x3F4`).
const PORT_MSR_DSR: u16 = FDC_MSR;

fn running_fdc(media: bool) -> Fdc82077 {
    let mut f = if media {
        Fdc82077::with_image(vec![0u8; FDC_1440_IMAGE_SIZE])
    } else {
        Fdc82077::new()
    };
    f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
    f
}

/// Seek the selected unit to `cylinder` and consume the Seek End interrupt.
fn seek(f: &mut Fdc82077, head_unit: u8, cylinder: u8) {
    f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SEEK));
    f.port_write(FDC_FIFO, 1, u32::from(head_unit));
    f.port_write(FDC_FIFO, 1, u32::from(cylinder));
    f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_INT));
    let _ = f.port_read(FDC_FIFO, 1);
    let _ = f.port_read(FDC_FIFO, 1);
}

/// Issue one READ ID and return its seven result bytes.
fn read_id(f: &mut Fdc82077, head_unit: u8) -> [u8; 7] {
    f.port_write(FDC_FIFO, 1, CMD_READ_ID_MFM);
    f.port_write(FDC_FIFO, 1, u32::from(head_unit));
    assert!(f.irq_line(), "READ ID interrupts on completion");
    let mut out = [0u8; 7];
    for byte in out.iter_mut() {
        *byte = f.port_read(FDC_FIFO, 1) as u8;
    }
    assert!(!f.irq_line(), "first result byte clears IRQ6");
    out
}

/// §5.1.8: READ ID reports "the present position of the recording heads", so
/// successive commands see successive ID fields as the diskette rotates.
#[test]
fn successive_read_id_returns_successive_sector_ids() {
    let mut f = running_fdc(true);
    seek(&mut f, 0x00, 5);

    let spt = FDC_1440_SECTORS_PER_TRACK;
    for i in 0..(u16::from(spt) * 2) {
        let res = read_id(&mut f, 0x00);
        let expected_r = (i % u16::from(spt)) as u8 + 1;
        assert_eq!(res[0], FDC_ST0_IC_NORMAL, "ST0 = IC=00 | H=0 | US=0");
        assert_eq!(res[1], 0, "ST1 clear on a good ID field");
        assert_eq!(res[2], 0, "ST2 clear");
        assert_eq!(res[3], 5, "C = present cylinder");
        assert_eq!(res[4], 0, "H = head from the HD bit");
        assert_eq!(res[5], expected_r, "R advances with rotation (pass {i})");
        assert_eq!(res[6], FDC_SECTOR_N, "N = 2 (512-byte sectors)");
    }
}

/// The HD bit selects the side; ST0 H and the result H follow it, and each
/// side keeps scanning the same rotational position sequence.
#[test]
fn read_id_reports_selected_head_in_st0_and_result() {
    let mut f = running_fdc(true);
    seek(&mut f, 0x04, 3); // head 1, unit 0

    let res = read_id(&mut f, 0x04);
    assert_eq!(res[0], FDC_ST0_IC_NORMAL | FDC_ST0_HEAD);
    assert_eq!(res[3], 3);
    assert_eq!(res[4], 1, "H = 1");
    assert_eq!(res[5], 1);

    let res = read_id(&mut f, 0x04);
    assert_eq!(res[5], 2, "rotation continues");
}

/// §5.1.8: with the head parked past the last formatted cylinder there is no
/// ID Address Mark to read, so the command terminates abnormally with MA set.
#[test]
fn read_id_beyond_formatted_cylinders_sets_missing_address_mark() {
    let mut f = running_fdc(true);
    seek(&mut f, 0x00, FDC_1440_CYLINDERS + 10);

    let res = read_id(&mut f, 0x00);
    assert_eq!(res[0], FDC_ST0_IC_ABNORMAL, "IC=01 abnormal termination");
    assert_eq!(
        res[1],
        ST1_MA | FDC_ST1_ND,
        "ST1 MA (no ID Address Mark) + ND (no ID field read)"
    );
    assert_eq!(res[2], 0);
    assert_eq!(&res[3..], &[0, 0, 0, 0], "no ID values to report");
}

/// No media at all is the same "no ID Address Mark after two index pulses"
/// outcome.
#[test]
fn read_id_without_media_sets_missing_address_mark() {
    let mut f = running_fdc(false);
    let res = read_id(&mut f, 0x00);
    assert_eq!(res[0], FDC_ST0_IC_ABNORMAL);
    assert_eq!(res[1], ST1_MA | FDC_ST1_ND);
    assert_eq!(&res[3..], &[0, 0, 0, 0]);
}

/// A failed READ ID reads no ID field, so it must not advance the scan.
#[test]
fn failed_read_id_does_not_advance_the_scan() {
    let mut f = running_fdc(true);
    seek(&mut f, 0x00, 0);
    assert_eq!(read_id(&mut f, 0x00)[5], 1);
    assert_eq!(read_id(&mut f, 0x00)[5], 2);

    f.eject();
    for _ in 0..3 {
        assert_eq!(read_id(&mut f, 0x00)[1], ST1_MA | FDC_ST1_ND);
    }
    f.attach_image(vec![0u8; FDC_1440_IMAGE_SIZE])
        .expect("1.44MB image");
    assert_eq!(
        read_id(&mut f, 0x00)[5],
        3,
        "scan resumes where the last good ID field left off"
    );
}

/// Seeking does not stop the diskette: the rotational scan continues across a
/// Seek, but a software reset restarts it at the first sector ID.
#[test]
fn scan_survives_seek_and_restarts_on_software_reset() {
    let mut f = running_fdc(true);
    seek(&mut f, 0x00, 1);
    assert_eq!(read_id(&mut f, 0x00)[5], 1);
    assert_eq!(read_id(&mut f, 0x00)[5], 2);

    seek(&mut f, 0x00, 40);
    let res = read_id(&mut f, 0x00);
    assert_eq!(res[3], 40, "C follows the new present cylinder");
    assert_eq!(res[5], 3, "rotation is not restarted by a seek");

    f.port_write(PORT_MSR_DSR, 1, DSR_SOFTWARE_RESET);
    // Drain the four queued post-reset polling statuses.
    for _ in 0..4 {
        f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_INT));
        let _ = f.port_read(FDC_FIFO, 1);
        let _ = f.port_read(FDC_FIFO, 1);
    }
    assert_eq!(read_id(&mut f, 0x00)[5], 1, "software reset restarts scan");
}

/// Each drive spins independently, so the rotational position is per unit.
#[test]
fn scan_position_is_per_drive() {
    let mut f = running_fdc(true);
    assert_eq!(read_id(&mut f, 0x00)[5], 1);
    assert_eq!(read_id(&mut f, 0x00)[5], 2);
    let unit1 = read_id(&mut f, 0x01);
    assert_eq!(unit1[0], FDC_ST0_IC_NORMAL | 0x01, "ST0 US = 1");
    assert_eq!(unit1[5], 1, "unit 1 keeps its own rotational position");
    assert_eq!(read_id(&mut f, 0x00)[5], 3);
}

/// A hardware reset also restarts the scan.
#[test]
fn hardware_reset_restarts_the_scan() {
    let mut f = running_fdc(true);
    assert_eq!(read_id(&mut f, 0x00)[5], 1);
    assert_eq!(read_id(&mut f, 0x00)[5], 2);
    f.reset();
    f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
    assert_eq!(read_id(&mut f, 0x00)[5], 1);
}
