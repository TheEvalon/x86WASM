//! 82077AA Configure EIS — implied seek before read/write commands.
//!
//! # Spec refs
//!
//! - Intel 82077AA CHMOS Single-Chip Floppy Disk Controller, Configure
//!   (`0x13`, §5.2.7): parameter byte 1 is `0|EIS|EFIFO|POLL|FIFOTHR`.
//!   "EIS — Enable Implied Seek. When set to '1', the 82077AA will perform a
//!   Seek operation before executing a Read or Write command." The Configure
//!   default is "EIS — No Implied Seeks", and the datasheet notes that "if
//!   implied seek is not enabled, the read and write commands should be
//!   preceded by: 1) Seek command — Step to the proper track, 2) Sense
//!   Interrupt Status — Terminate the Seek command, 3) Read ID — Verify head
//!   is on proper track".
//! - Intel 82077AA Seek (§5.2.8) — a Seek leaves the selected unit's Present
//!   Cylinder Number equal to the target cylinder; PCN is visible through
//!   DUMPREG (§5.3.3) and through Sense Drive Status ST3 T0 (§6.4 bit 4).
//! - Intel 82077AA Table 5-1 — READ DATA / READ TRACK / READ DELETED DATA /
//!   VERIFY / WRITE DATA / WRITE DELETED DATA / SCAN carry a C parameter;
//!   FORMAT TRACK (HD|US, N, SC, GPL, D) does not.
//! - Intel 82077AA §5.3.2 — a DOR/DSR software reset restores the Configure
//!   EIS default when LOCK is clear.

use devices::{
    Fdc82077, PortDevice, FDC_1440_CYLINDERS, FDC_1440_IMAGE_SIZE, FDC_CMD_CONFIGURE, FDC_CMD_MFM,
    FDC_CMD_READ_DATA, FDC_CMD_SENSE_DRIVE_STATUS, FDC_CMD_WRITE_DATA, FDC_DOR, FDC_DOR_DMA_IRQ,
    FDC_DOR_RESET_N, FDC_FIFO, FDC_SECTOR_N, FDC_ST0_IC_ABNORMAL, FDC_ST0_IC_NORMAL, FDC_ST1_ND,
    FDC_ST3_TRACK0,
};

/// DUMPREG. Spec: Intel 82077AA §5.2.10 / Table 5-1.
const CMD_DUMPREG: u32 = 0x0E;
/// READ TRACK with MFM. Spec: Intel 82077AA §5.1.3 / Table 5-1 (`MFM|00010`).
const CMD_READ_TRACK_MFM: u32 = 0x42;
/// VERIFY with MFM. Spec: Intel 82077AA Table 5-1 (`MFM|10110`).
const CMD_VERIFY_MFM: u32 = 0x56;
/// FORMAT TRACK with MFM. Spec: Intel 82077AA §5.1.7 / Table 5-1 (`MFM|01101`).
const CMD_FORMAT_TRACK_MFM: u32 = 0x4D;
/// READ ID with MFM. Spec: Intel 82077AA §5.1.8 / Table 5-1 (`MFM|01010`).
const CMD_READ_ID_MFM: u32 = 0x4A;
/// Configure byte1 EIS (bit 6). Spec: Intel 82077AA §5.2.7.
const CONFIG_EIS: u32 = 0x40;
/// Configure byte1 reset value (EFIFO set, EIS clear). Spec: §5.2.7.
const CONFIG_RESET: u32 = 0x20;
/// ST1 bit0 MA — Missing Address Mark. Spec: Intel 82077AA §6.2.
const ST1_MA: u8 = 0x01;

fn running_fdc() -> Fdc82077 {
    let mut f = Fdc82077::with_image(vec![0u8; FDC_1440_IMAGE_SIZE]);
    f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
    f
}

fn configure(f: &mut Fdc82077, byte1: u32) {
    f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_CONFIGURE));
    f.port_write(FDC_FIFO, 1, 0x00);
    f.port_write(FDC_FIFO, 1, byte1);
    f.port_write(FDC_FIFO, 1, 0x00); // PRETRK
}

/// Run an eight-parameter transfer command and return its 7-byte result.
fn transfer(f: &mut Fdc82077, cmd: u32, head_unit: u8, c: u8, h: u8, r: u8, eot: u8) -> [u8; 7] {
    f.port_write(FDC_FIFO, 1, cmd);
    for p in [head_unit, c, h, r, FDC_SECTOR_N, eot, 0x1B, 0xFF] {
        f.port_write(FDC_FIFO, 1, u32::from(p));
    }
    let mut out = [0u8; 7];
    for byte in out.iter_mut() {
        *byte = f.port_read(FDC_FIFO, 1) as u8;
    }
    out
}

/// Present Cylinder Numbers PCN0–PCN3 via DUMPREG (§5.3.3 result bytes 0–3).
fn pcn(f: &mut Fdc82077) -> [u8; 4] {
    f.port_write(FDC_FIFO, 1, CMD_DUMPREG);
    let mut all = [0u8; 10];
    for byte in all.iter_mut() {
        *byte = f.port_read(FDC_FIFO, 1) as u8;
    }
    [all[0], all[1], all[2], all[3]]
}

/// Sense Drive Status ST3 for `head_unit` (§5.2.5 / §6.4).
fn st3(f: &mut Fdc82077, head_unit: u8) -> u8 {
    f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_SENSE_DRIVE_STATUS));
    f.port_write(FDC_FIFO, 1, u32::from(head_unit));
    f.port_read(FDC_FIFO, 1) as u8
}

/// §5.2.7: with EIS set the controller performs a Seek before the read, so the
/// selected unit's PCN ends up at the command's C parameter.
#[test]
fn eis_set_performs_implied_seek_before_read_data() {
    let mut f = running_fdc();
    configure(&mut f, CONFIG_RESET | CONFIG_EIS);
    assert_eq!(pcn(&mut f)[0], 0, "starts at track 0");
    assert_ne!(st3(&mut f, 0x00) & FDC_ST3_TRACK0, 0);

    let res = transfer(
        &mut f,
        u32::from(FDC_CMD_MFM | FDC_CMD_READ_DATA),
        0x00,
        7,
        0,
        1,
        1,
    );
    assert_eq!(res[0], FDC_ST0_IC_NORMAL, "read still succeeds");
    assert_eq!(pcn(&mut f)[0], 7, "implied seek moved the head to C");
    assert_eq!(
        st3(&mut f, 0x00) & FDC_ST3_TRACK0,
        0,
        "ST3 T0 clear away from track 0"
    );
}

/// §5.2.7 default: "EIS — No Implied Seeks". The host stays responsible for an
/// explicit Seek, so PCN must not move.
#[test]
fn eis_clear_does_not_move_the_head() {
    let mut f = running_fdc();
    configure(&mut f, CONFIG_RESET);
    let res = transfer(
        &mut f,
        u32::from(FDC_CMD_MFM | FDC_CMD_READ_DATA),
        0x00,
        7,
        0,
        1,
        1,
    );
    assert_eq!(res[0], FDC_ST0_IC_NORMAL);
    assert_eq!(pcn(&mut f)[0], 0, "no implied seek without EIS");
    assert_ne!(st3(&mut f, 0x00) & FDC_ST3_TRACK0, 0);
}

/// The Configure reset default already has EIS clear, so a controller that
/// never issued Configure behaves the same way.
#[test]
fn configure_default_has_no_implied_seek() {
    let mut f = running_fdc();
    let res = transfer(
        &mut f,
        u32::from(FDC_CMD_MFM | FDC_CMD_READ_DATA),
        0x00,
        9,
        0,
        1,
        1,
    );
    assert_eq!(res[0], FDC_ST0_IC_NORMAL);
    assert_eq!(pcn(&mut f)[0], 0);
}

/// Table 5-1: WRITE DATA, READ TRACK and VERIFY all carry a C parameter, so
/// EIS seeks for them too.
#[test]
fn implied_seek_covers_write_data_read_track_and_verify() {
    for (cmd, cylinder) in [
        (u32::from(FDC_CMD_MFM | FDC_CMD_WRITE_DATA), 11u8),
        (CMD_READ_TRACK_MFM, 22),
        (CMD_VERIFY_MFM, 33),
    ] {
        let mut f = running_fdc();
        configure(&mut f, CONFIG_RESET | CONFIG_EIS);
        let _ = transfer(&mut f, cmd, 0x00, cylinder, 0, 1, 1);
        assert_eq!(
            pcn(&mut f)[0],
            cylinder,
            "command {cmd:#04X} should imply a seek"
        );
    }
}

/// Table 5-1: FORMAT TRACK's parameters are HD|US, N, SC, GPL, D — there is no
/// cylinder to seek to, so EIS must not move the head.
#[test]
fn format_track_has_no_cylinder_parameter_and_no_implied_seek() {
    let mut f = running_fdc();
    configure(&mut f, CONFIG_RESET | CONFIG_EIS);
    f.port_write(FDC_FIFO, 1, CMD_FORMAT_TRACK_MFM);
    for p in [0x00u8, FDC_SECTOR_N, 18, 0x54, 0xF6] {
        f.port_write(FDC_FIFO, 1, u32::from(p));
    }
    for _ in 0..7 {
        let _ = f.port_read(FDC_FIFO, 1);
    }
    assert_eq!(pcn(&mut f)[0], 0, "FORMAT TRACK carries no C parameter");
}

/// The seek is mechanical and happens before the transfer is attempted, so it
/// still moves the head when the transfer then fails. Parking the head past
/// the last formatted cylinder is visible to READ ID as MA|ND (§5.1.8).
#[test]
fn implied_seek_happens_even_when_the_transfer_fails() {
    let mut f = running_fdc();
    configure(&mut f, CONFIG_RESET | CONFIG_EIS);
    let target = FDC_1440_CYLINDERS + 10;

    let res = transfer(
        &mut f,
        u32::from(FDC_CMD_MFM | FDC_CMD_READ_DATA),
        0x00,
        target,
        0,
        1,
        1,
    );
    assert_eq!(res[0], FDC_ST0_IC_ABNORMAL, "cylinder is off the media");
    assert_eq!(res[1], FDC_ST1_ND);
    assert_eq!(pcn(&mut f)[0], target, "the head still stepped");

    f.port_write(FDC_FIFO, 1, CMD_READ_ID_MFM);
    f.port_write(FDC_FIFO, 1, 0x00);
    assert_eq!(f.port_read(FDC_FIFO, 1) as u8, FDC_ST0_IC_ABNORMAL);
    assert_eq!(
        f.port_read(FDC_FIFO, 1) as u8,
        ST1_MA | FDC_ST1_ND,
        "no ID Address Mark past the formatted cylinders"
    );
}

/// Only the unit selected by the US bits of the first parameter moves.
#[test]
fn implied_seek_moves_only_the_selected_unit() {
    let mut f = running_fdc();
    configure(&mut f, CONFIG_RESET | CONFIG_EIS);
    let _ = transfer(
        &mut f,
        u32::from(FDC_CMD_MFM | FDC_CMD_READ_DATA),
        0x02,
        15,
        0,
        1,
        1,
    );
    assert_eq!(pcn(&mut f), [0, 0, 15, 0], "only PCN2 moved");
}

/// §5.2.7: an implied seek is part of the command, so it produces no extra
/// interrupt — the single IRQ6 is the command's completion interrupt and it is
/// cleared by the first result byte.
#[test]
fn implied_seek_adds_no_extra_interrupt() {
    let mut f = running_fdc();
    configure(&mut f, CONFIG_RESET | CONFIG_EIS);
    f.port_write(FDC_FIFO, 1, u32::from(FDC_CMD_MFM | FDC_CMD_READ_DATA));
    for (i, p) in [0x00u8, 5, 0, 1, FDC_SECTOR_N, 1, 0x1B, 0xFF]
        .into_iter()
        .enumerate()
    {
        f.port_write(FDC_FIFO, 1, u32::from(p));
        if i < 7 {
            assert!(!f.irq_line(), "no interrupt during the parameter phase");
        }
    }
    assert!(f.irq_line(), "one completion interrupt");
    let _ = f.port_read(FDC_FIFO, 1);
    assert!(!f.irq_line(), "cleared by the first result byte");
    for _ in 0..6 {
        let _ = f.port_read(FDC_FIFO, 1);
    }
    assert_eq!(pcn(&mut f)[0], 5);
}

/// §5.3.2: an unlocked software reset restores the Configure defaults, so EIS
/// stops applying until the host configures it again.
#[test]
fn software_reset_clears_eis_and_stops_implied_seek() {
    let mut f = running_fdc();
    configure(&mut f, CONFIG_RESET | CONFIG_EIS);
    let _ = transfer(
        &mut f,
        u32::from(FDC_CMD_MFM | FDC_CMD_READ_DATA),
        0x00,
        6,
        0,
        1,
        1,
    );
    assert_eq!(pcn(&mut f)[0], 6);

    f.port_write(FDC_DOR, 1, 0); // enter DOR software reset
    f.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
    assert_eq!(pcn(&mut f)[0], 0, "software reset zeroes the PCNs");

    let _ = transfer(
        &mut f,
        u32::from(FDC_CMD_MFM | FDC_CMD_READ_DATA),
        0x00,
        6,
        0,
        1,
        1,
    );
    assert_eq!(pcn(&mut f)[0], 0, "EIS returned to its reset default");
}
