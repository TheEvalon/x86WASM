//! UHCI schedule + PORTSC stub (Intel UHCI / PIIX3 USB).
//!
//! Spec: Universal Host Controller Interface (UHCI) Design Guide, Revision 1.1;
//! Intel 82371SB (PIIX3) USB function. Frame list → (optional QH + bounded
//! horizontal chain) → TD transfer via host memory callbacks; PORTSC CCS/PED/PR
//! for firmware probe. No real USB device stack, no isochronous bandwidth
//! accounting, no full multi-QH reclaim walks.
//!
//! PCI config / BAR0 I/O decode remain in [`crate::pci::PciConfig`]; this module
//! owns schedule + PORTSC bit semantics. Round-14 wires host IRQ pending onto
//! PIIX3 PIRQD → classic ISA IRQ11 via machine-pc (see
//! `docs/uhci-r14-pic-irq-wire.md`, `docs/uhci-r14-ioc-usbsts.md`).
//! See `docs/uhci-r8-one-td.md`, `docs/uhci-r11-frame-list-walk.md`,
//! `docs/uhci-r11-portsc.md`, `docs/uhci-r12-qh-horizontal.md`,
//! `docs/uhci-r12-usbsts-usbintr.md`.

use crate::pci::{
    PCI_PIIX_USB_UHCI_FLBASEADD, PCI_PIIX_USB_UHCI_FRNUM, PCI_PIIX_USB_UHCI_IO_SIZE,
    PCI_PIIX_USB_UHCI_PORTSC1, PCI_PIIX_USB_UHCI_PORTSC2, PCI_PIIX_USB_UHCI_USBCMD,
    PCI_PIIX_USB_UHCI_USBINTR, PCI_PIIX_USB_UHCI_USBSTS,
};

/// USBCMD Run/Stop bit (UHCI 1.1 §2.1.1 bit 0).
pub const UHCI_USBCMD_RS: u16 = 1 << 0;

/// USBCMD Global Suspend (UHCI 1.1 §2.1.1 bit 3) — required for Resume Detect.
pub const UHCI_USBCMD_GSUSPEND: u16 = 1 << 3;

/// USBSTS USB Interrupt (UHCI 1.1 §2.1.2 bit 0) — set on IOC completion.
pub const UHCI_USBSTS_USBINT: u16 = 1 << 0;

/// USBSTS USB Error Interrupt (UHCI 1.1 §2.1.2 bit 1).
pub const UHCI_USBSTS_USBERRINT: u16 = 1 << 1;

/// USBSTS Resume Detect (UHCI 1.1 §2.1.2 bit 2).
pub const UHCI_USBSTS_RD: u16 = 1 << 2;

/// USBSTS Host System Error (UHCI 1.1 §2.1.2 bit 3).
pub const UHCI_USBSTS_HSE: u16 = 1 << 3;

/// USBSTS HC Process Error (UHCI 1.1 §2.1.2 bit 4) — fatal; not maskable.
pub const UHCI_USBSTS_HCPE: u16 = 1 << 4;

/// USBSTS HCHalted (UHCI 1.1 §2.1.2 bit 5) — sticky while RS clear (stub).
pub const UHCI_USBSTS_HCHALTED: u16 = 1 << 5;

/// USBSTS bits cleared by writing 1 (R/WC). Spec: UHCI 1.1 §2.1.2.
pub const UHCI_USBSTS_RWC_MASK: u16 = UHCI_USBSTS_USBINT
    | UHCI_USBSTS_USBERRINT
    | UHCI_USBSTS_RD
    | UHCI_USBSTS_HSE
    | UHCI_USBSTS_HCPE;

/// USBINTR Timeout/CRC Interrupt Enable (UHCI 1.1 §2.1.3 bit 0) — gates USBERRINT.
pub const UHCI_USBINTR_CRC: u16 = 1 << 0;

/// USBINTR Resume Interrupt Enable (bit 1).
pub const UHCI_USBINTR_RESUME: u16 = 1 << 1;

/// USBINTR Interrupt On Complete Enable (bit 2) — gates USBINT from IOC.
pub const UHCI_USBINTR_IOC: u16 = 1 << 2;

/// USBINTR Short Packet Interrupt Enable (bit 3).
pub const UHCI_USBINTR_SPI: u16 = 1 << 3;

/// Guest-writable USBINTR bits (15:4 reserved → hardwired 0).
pub const UHCI_USBINTR_WRITABLE: u16 =
    UHCI_USBINTR_CRC | UHCI_USBINTR_RESUME | UHCI_USBINTR_IOC | UHCI_USBINTR_SPI;

/// PIIX3 USB Host Controller interrupt pin index into PIRQA–D (0=A … 3=D).
///
/// Spec: Intel 82371SB — "For the PIIX3, the USB interrupt is output on
/// PIRQD#." Hosts drive [`crate::pci::PciConfig::set_pirq_line`] with this
/// index when [`uhci_interrupt_pending`] is true.
pub const UHCI_PIIX_PIRQD: u8 = 3;

/// Classic ISA IRQ used by this machine's UHCI→PIC tests / SeaBIOS-style route.
///
/// Spec: Intel 82371SB PIRQRC — bits 3:0 = `1011b` selects IRQ11. Firmware
/// programs `PIRQRC[D]` (config `0x63`) to this value; the HC itself only
/// asserts PIRQD#. Documented machine-model choice for POST USB probe honesty.
pub const UHCI_CLASSIC_ISA_IRQ: u8 = 11;

/// PIRQRC[D] byte that routes PIRQD → [`UHCI_CLASSIC_ISA_IRQ`] (IRQ11).
///
/// Spec: Intel 82371SB — bit7 clear enables route; bits3:0 = `0xB` → IRQ11.
pub const UHCI_CLASSIC_PIRQRC_D: u8 = UHCI_CLASSIC_ISA_IRQ;

/// Frame-list / QH / TD link Terminate bit.
pub const UHCI_LINK_TERMINATE: u32 = 1 << 0;

/// Link QH select bit (1 = QH, 0 = TD).
pub const UHCI_LINK_QH: u32 = 1 << 1;

/// TD Control/Status Active bit (UHCI 1.1 §3.2.2 bit 23).
pub const UHCI_TD_ACTIVE: u32 = 1 << 23;

/// TD Control/Status Interrupt On Complete (bit 24).
pub const UHCI_TD_IOC: u32 = 1 << 24;

/// TD token PID field mask (bits 7:0).
pub const UHCI_TD_PID_MASK: u32 = 0xFF;

/// UHCI OUT token PID.
pub const UHCI_PID_OUT: u8 = 0xE1;

/// UHCI IN token PID.
pub const UHCI_PID_IN: u8 = 0x69;

/// UHCI SETUP token PID.
pub const UHCI_PID_SETUP: u8 = 0x2D;

/// TD Maximum Length field (bits 31:21 of token) — encoded as `n − 1`.
pub const UHCI_TD_MAXLEN_SHIFT: u32 = 21;

/// TD Actual Length field (bits 10:0 of status) — encoded as `n − 1`.
pub const UHCI_TD_ACTLEN_MASK: u32 = 0x7FF;

/// Soft cap on bytes moved by one stub TD (safety; UHCI max packet is 1280).
pub const UHCI_TD_MAX_TRANSFER: usize = 1280;

/// Soft cap on frames walked by [`run_n_frames`] (1024-slot frame list).
pub const UHCI_MAX_FRAMES_WALK: u32 = 1024;

/// Soft cap on QH horizontal hops per frame (R12: deeper than R11's single hop).
///
/// Spec: UHCI 1.1 §3.3 — horizontal link may chain queue heads. This stub
/// follows the starting QH plus up to this many additional horizontal QH hops
/// (so at most `1 + UHCI_MAX_QH_HORIZONTAL` QHs / element TDs per frame).
/// Deeper chains return [`UhciTdError::QueueHeadHorizontalUnsupported`].
/// Isochronous TD schedules are not special-cased: a frame-list TD link still
/// executes as an ordinary Active TD; bandwidth reclamation / iso reclaim is
/// explicitly unsupported.
pub const UHCI_MAX_QH_HORIZONTAL: u32 = 4;

/// Max TD addresses collected per frame (`1` start + horizontal hops).
pub const UHCI_MAX_FRAME_TDS: usize = 1 + UHCI_MAX_QH_HORIZONTAL as usize;

/// PORTSC Current Connect Status (UHCI 1.1 §2.1.7 bit 0) — RO from guest.
pub const UHCI_PORTSC_CCS: u16 = 1 << 0;

/// PORTSC Connect Status Change (bit 1) — R/WC.
pub const UHCI_PORTSC_CSC: u16 = 1 << 1;

/// PORTSC Port Enabled/Disabled (bit 2).
pub const UHCI_PORTSC_PED: u16 = 1 << 2;

/// PORTSC Port Enable/Disable Change (bit 3) — R/WC.
pub const UHCI_PORTSC_PEDC: u16 = 1 << 3;

/// PORTSC Low Speed Device Attached (bit 8) — RO from guest.
pub const UHCI_PORTSC_LS: u16 = 1 << 8;

/// PORTSC reserved bit 10 — always reads as 1 (UHCI 1.1 §2.1.7).
pub const UHCI_PORTSC_RESERVED1: u16 = 1 << 10;

/// PORTSC Port Reset (bit 12).
pub const UHCI_PORTSC_PR: u16 = 1 << 12;

/// Guest-writable PORTSC bits retained by this stub (excluding R/WC one-shots).
const UHCI_PORTSC_WRITABLE: u16 = UHCI_PORTSC_PED | UHCI_PORTSC_PR;

/// Result of a successful one-TD walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UhciTdTransfer {
    /// Physical address of the TD that was executed.
    pub td_addr: u32,
    /// Token PID processed.
    pub pid: u8,
    /// Bytes moved through the buffer pointer.
    pub bytes_copied: usize,
    /// Whether USBSTS.USBINT was latched (IOC was set).
    pub usbint: bool,
}

/// Summary of a multi-frame / QH-horizontal schedule walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UhciFrameWalkSummary {
    /// Frame-list slots examined (FRNUM advanced this many times).
    pub frames_scanned: u32,
    /// Transfer descriptors successfully completed.
    pub tds_completed: u32,
    /// Bytes copied across all completed TDs.
    pub bytes_copied: usize,
    /// Whether USBSTS.USBINT is set after the walk.
    pub usbint: bool,
}

/// Errors from [`run_one_td`] / [`run_n_frames`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UhciTdError {
    /// PCI Command.BusMaster clear — schedule must not DMA.
    BusMasterDisabled,
    /// USBCMD.RS clear — controller not running.
    NotRunning,
    /// Frame-list link Terminate, empty QH element, or inactive TD.
    NothingToDo,
    /// Frame list pointed at a QH whose element link is another QH (not walked).
    QueueHeadDepthUnsupported,
    /// Horizontal QH chain exceeded [`UHCI_MAX_QH_HORIZONTAL`].
    QueueHeadHorizontalUnsupported,
    /// Token PID is not IN/OUT/SETUP.
    UnsupportedPid(u8),
    /// Empty host/device buffer supplied for a data-bearing transfer.
    EmptyBuffer,
    /// Buffer pointer + length wraps the 32-bit physical address space.
    GuestAddressOverflow {
        phys_addr: u32,
        bytes_requested: usize,
    },
    /// `max_frames` was zero or exceeded [`UHCI_MAX_FRAMES_WALK`].
    InvalidFrameCount,
    /// PORTSC port index not 0 or 1.
    InvalidPortIndex,
}

/// Read a little-endian `u16` from the UHCI I/O register file.
fn reg_u16(regs: &[u8; PCI_PIIX_USB_UHCI_IO_SIZE as usize], off: u8) -> u16 {
    let i = off as usize;
    u16::from_le_bytes([regs[i], regs[i + 1]])
}

/// Write a little-endian `u16` into the UHCI I/O register file.
fn set_reg_u16(regs: &mut [u8; PCI_PIIX_USB_UHCI_IO_SIZE as usize], off: u8, value: u16) {
    let i = off as usize;
    let bytes = value.to_le_bytes();
    regs[i] = bytes[0];
    regs[i + 1] = bytes[1];
}

/// Read a little-endian `u32` from the UHCI I/O register file.
fn reg_u32(regs: &[u8; PCI_PIIX_USB_UHCI_IO_SIZE as usize], off: u8) -> u32 {
    let i = off as usize;
    u32::from_le_bytes([regs[i], regs[i + 1], regs[i + 2], regs[i + 3]])
}

fn read_phys_u32<R: FnMut(u32) -> u8>(mem_read: &mut R, addr: u32) -> u32 {
    u32::from_le_bytes([
        mem_read(addr),
        mem_read(addr.wrapping_add(1)),
        mem_read(addr.wrapping_add(2)),
        mem_read(addr.wrapping_add(3)),
    ])
}

fn write_phys_u32<W: FnMut(u32, u8)>(mem_write: &mut W, addr: u32, value: u32) {
    for (i, b) in value.to_le_bytes().into_iter().enumerate() {
        mem_write(addr.wrapping_add(i as u32), b);
    }
}

/// Decode Maximum Length / Actual Length (`n − 1` encoding; `0x7FF` = 0 bytes).
fn uhci_len_field(encoded: u32) -> usize {
    let n = (encoded & UHCI_TD_ACTLEN_MASK) as usize;
    if n == 0x7FF {
        0
    } else {
        n + 1
    }
}

fn encode_actlen(bytes: usize) -> u32 {
    if bytes == 0 {
        0x7FF
    } else {
        ((bytes - 1) as u32) & UHCI_TD_ACTLEN_MASK
    }
}

/// Resolve the first TD address from a frame-list / QH element link.
fn resolve_td_addr<R: FnMut(u32) -> u8>(mem_read: &mut R, link: u32) -> Result<u32, UhciTdError> {
    if link & UHCI_LINK_TERMINATE != 0 {
        return Err(UhciTdError::NothingToDo);
    }
    let ptr = link & !0xF;
    if link & UHCI_LINK_QH == 0 {
        return Ok(ptr);
    }
    // QH: follow Queue Element Link Pointer (dword 1) once to a TD.
    let element = read_phys_u32(mem_read, ptr.wrapping_add(4));
    if element & UHCI_LINK_TERMINATE != 0 {
        return Err(UhciTdError::NothingToDo);
    }
    if element & UHCI_LINK_QH != 0 {
        return Err(UhciTdError::QueueHeadDepthUnsupported);
    }
    Ok(element & !0xF)
}

/// Collect up to [`UHCI_MAX_FRAME_TDS`] TD addresses from a frame link.
///
/// Spec: UHCI 1.1 §3.3 — QH horizontal link may chain queue heads. This stub
/// follows the element TD of the first QH, then up to
/// [`UHCI_MAX_QH_HORIZONTAL`] additional horizontal QH hops (each element TD).
/// Deeper horizontal chains return
/// [`UhciTdError::QueueHeadHorizontalUnsupported`]. Isochronous / bandwidth
/// reclaim are not modeled (frame-list TDs still execute as ordinary TDs).
fn collect_frame_td_addrs<R: FnMut(u32) -> u8>(
    mem_read: &mut R,
    link: u32,
    out: &mut [u32; UHCI_MAX_FRAME_TDS],
) -> Result<usize, UhciTdError> {
    if link & UHCI_LINK_TERMINATE != 0 {
        return Err(UhciTdError::NothingToDo);
    }
    let mut count = 0usize;
    if link & UHCI_LINK_QH == 0 {
        out[0] = link & !0xF;
        return Ok(1);
    }

    let mut qh = link & !0xF;
    let mut horizontal_hops = 0u32;
    loop {
        let element = read_phys_u32(mem_read, qh.wrapping_add(4));
        if element & UHCI_LINK_TERMINATE == 0 {
            if element & UHCI_LINK_QH != 0 {
                return Err(UhciTdError::QueueHeadDepthUnsupported);
            }
            if count < out.len() {
                out[count] = element & !0xF;
                count += 1;
            }
        }
        let horizontal = read_phys_u32(mem_read, qh);
        if horizontal & UHCI_LINK_TERMINATE != 0 {
            break;
        }
        if horizontal & UHCI_LINK_QH == 0 {
            // Horizontal TD: treat as one more transfer target.
            if count < out.len() {
                out[count] = horizontal & !0xF;
                count += 1;
            }
            break;
        }
        if horizontal_hops >= UHCI_MAX_QH_HORIZONTAL {
            return Err(UhciTdError::QueueHeadHorizontalUnsupported);
        }
        horizontal_hops += 1;
        qh = horizontal & !0xF;
    }
    if count == 0 {
        Err(UhciTdError::NothingToDo)
    } else {
        Ok(count)
    }
}

fn execute_td_at<R, W>(
    regs: &mut [u8; PCI_PIIX_USB_UHCI_IO_SIZE as usize],
    device_buf: &mut [u8],
    mem_read: &mut R,
    mem_write: &mut W,
    td_addr: u32,
) -> Result<UhciTdTransfer, UhciTdError>
where
    R: FnMut(u32) -> u8,
    W: FnMut(u32, u8),
{
    let link_ptr = read_phys_u32(mem_read, td_addr);
    let mut status = read_phys_u32(mem_read, td_addr.wrapping_add(4));
    let token = read_phys_u32(mem_read, td_addr.wrapping_add(8));
    let buffer = read_phys_u32(mem_read, td_addr.wrapping_add(12));
    let _ = link_ptr; // TD vertical link not followed in this stub

    if status & UHCI_TD_ACTIVE == 0 {
        return Err(UhciTdError::NothingToDo);
    }

    let pid = (token & UHCI_TD_PID_MASK) as u8;
    let max_len = uhci_len_field(token >> UHCI_TD_MAXLEN_SHIFT).min(UHCI_TD_MAX_TRANSFER);
    if max_len == 0 {
        status = (status & !UHCI_TD_ACTIVE & !UHCI_TD_ACTLEN_MASK) | encode_actlen(0);
        write_phys_u32(mem_write, td_addr.wrapping_add(4), status);
        let usbint = status_was_ioc_and_latch(regs, status);
        return Ok(UhciTdTransfer {
            td_addr,
            pid,
            bytes_copied: 0,
            usbint,
        });
    }

    if device_buf.is_empty() {
        return Err(UhciTdError::EmptyBuffer);
    }
    let n = max_len.min(device_buf.len());
    if buffer.checked_add((n - 1) as u32).is_none() {
        return Err(UhciTdError::GuestAddressOverflow {
            phys_addr: buffer,
            bytes_requested: n,
        });
    }

    match pid {
        UHCI_PID_IN => {
            for (i, byte) in device_buf.iter().take(n).enumerate() {
                mem_write(buffer.wrapping_add(i as u32), *byte);
            }
        }
        UHCI_PID_OUT | UHCI_PID_SETUP => {
            for (i, slot) in device_buf.iter_mut().enumerate().take(n) {
                *slot = mem_read(buffer.wrapping_add(i as u32));
            }
        }
        other => return Err(UhciTdError::UnsupportedPid(other)),
    }

    let ioc = status & UHCI_TD_IOC != 0;
    status = (status & !UHCI_TD_ACTIVE & !UHCI_TD_ACTLEN_MASK) | encode_actlen(n);
    write_phys_u32(mem_write, td_addr.wrapping_add(4), status);
    let usbint = if ioc {
        let sts = reg_u16(regs, PCI_PIIX_USB_UHCI_USBSTS) | UHCI_USBSTS_USBINT;
        set_reg_u16(regs, PCI_PIIX_USB_UHCI_USBSTS, sts);
        true
    } else {
        false
    };

    Ok(UhciTdTransfer {
        td_addr,
        pid,
        bytes_copied: n,
        usbint,
    })
}

fn bump_frnum(regs: &mut [u8; PCI_PIIX_USB_UHCI_IO_SIZE as usize]) {
    let frnum = (reg_u16(regs, PCI_PIIX_USB_UHCI_FRNUM).wrapping_add(1)) & 0x3FF;
    set_reg_u16(regs, PCI_PIIX_USB_UHCI_FRNUM, frnum);
}

/// Read USBSTS with HCHalted overlay when USBCMD.RS is clear.
///
/// Spec: UHCI 1.1 §2.1.2 — HCHalted is set by the HC after it stops; this stub
/// reflects Run/Stop rather than a separate halt latch.
pub fn usbsts_read(regs: &[u8; PCI_PIIX_USB_UHCI_IO_SIZE as usize]) -> u16 {
    let mut sts =
        reg_u16(regs, PCI_PIIX_USB_UHCI_USBSTS) & (UHCI_USBSTS_RWC_MASK | UHCI_USBSTS_HCHALTED);
    if reg_u16(regs, PCI_PIIX_USB_UHCI_USBCMD) & UHCI_USBCMD_RS == 0 {
        sts |= UHCI_USBSTS_HCHALTED;
    } else {
        sts &= !UHCI_USBSTS_HCHALTED;
    }
    sts
}

/// Write USBSTS: R/WC clear for interrupt/error bits; HCHalted is not stored.
///
/// Spec: UHCI 1.1 §2.1.2 — software clears a bit by writing 1.
pub fn usbsts_write_w1c(regs: &mut [u8; PCI_PIIX_USB_UHCI_IO_SIZE as usize], value: u16) {
    let cur = reg_u16(regs, PCI_PIIX_USB_UHCI_USBSTS);
    let cleared = cur & !(value & UHCI_USBSTS_RWC_MASK);
    set_reg_u16(
        regs,
        PCI_PIIX_USB_UHCI_USBSTS,
        cleared & UHCI_USBSTS_RWC_MASK,
    );
}

/// Read USBINTR (bits 3:0 only).
pub fn usbintr_read(regs: &[u8; PCI_PIIX_USB_UHCI_IO_SIZE as usize]) -> u16 {
    reg_u16(regs, PCI_PIIX_USB_UHCI_USBINTR) & UHCI_USBINTR_WRITABLE
}

/// Write USBINTR; reserved bits 15:4 hardwired 0.
pub fn usbintr_write(regs: &mut [u8; PCI_PIIX_USB_UHCI_IO_SIZE as usize], value: u16) {
    set_reg_u16(
        regs,
        PCI_PIIX_USB_UHCI_USBINTR,
        value & UHCI_USBINTR_WRITABLE,
    );
}

/// Host/device IRQ line from USBSTS ∩ USBINTR (plus unmaskable HCPE).
///
/// Spec: UHCI 1.1 §2.1.3 — disabled sources still appear in USBSTS for polling;
/// this helper reports whether the HC would raise an interrupt to the host.
/// Machine hosts mirror this onto PIRQD ([`UHCI_PIIX_PIRQD`]) then DualPic via
/// PIRQRC (classic IRQ11 — see `docs/uhci-r14-pic-irq-wire.md`).
pub fn uhci_interrupt_pending(regs: &[u8; PCI_PIIX_USB_UHCI_IO_SIZE as usize]) -> bool {
    let sts = usbsts_read(regs);
    let en = usbintr_read(regs);
    if sts & UHCI_USBSTS_HCPE != 0 {
        return true;
    }
    if sts & UHCI_USBSTS_USBINT != 0 && en & UHCI_USBINTR_IOC != 0 {
        return true;
    }
    if sts & UHCI_USBSTS_USBERRINT != 0 && en & UHCI_USBINTR_CRC != 0 {
        return true;
    }
    if sts & UHCI_USBSTS_RD != 0 && en & UHCI_USBINTR_RESUME != 0 {
        return true;
    }
    // Short-packet completions latch USBINT; SPI enable alone is not modeled
    // as a separate status bit in this stub.
    let _ = UHCI_USBINTR_SPI;
    false
}

/// Host helper: latch USBERRINT (transaction-error stub).
///
/// Spec: UHCI 1.1 §2.1.2 bit 1. No real CRC/timeout engine — hosts call this
/// to exercise status / USBINTR gating.
pub fn latch_usb_error(regs: &mut [u8; PCI_PIIX_USB_UHCI_IO_SIZE as usize]) {
    let sts = reg_u16(regs, PCI_PIIX_USB_UHCI_USBSTS) | UHCI_USBSTS_USBERRINT;
    set_reg_u16(regs, PCI_PIIX_USB_UHCI_USBSTS, sts);
}

/// Host helper: latch USBSTS.USBINT (IOC-completion stub without a TD walk).
///
/// Spec: UHCI 1.1 §2.1.2 bit 0. Real completions also set this via
/// [`run_one_td`] when TD.IOC is set; this helper exercises PIC / USBINTR paths.
pub fn latch_usb_interrupt(regs: &mut [u8; PCI_PIIX_USB_UHCI_IO_SIZE as usize]) {
    let sts = reg_u16(regs, PCI_PIIX_USB_UHCI_USBSTS) | UHCI_USBSTS_USBINT;
    set_reg_u16(regs, PCI_PIIX_USB_UHCI_USBSTS, sts);
}

/// Host helper: latch Resume Detect when USBCMD.GSuspend is set.
///
/// Spec: UHCI 1.1 §2.1.2 bit 2 — only valid in global suspend. Returns `false`
/// if GSuspend is clear (no latch).
pub fn latch_resume_detect(regs: &mut [u8; PCI_PIIX_USB_UHCI_IO_SIZE as usize]) -> bool {
    if reg_u16(regs, PCI_PIIX_USB_UHCI_USBCMD) & UHCI_USBCMD_GSUSPEND == 0 {
        return false;
    }
    let sts = reg_u16(regs, PCI_PIIX_USB_UHCI_USBSTS) | UHCI_USBSTS_RD;
    set_reg_u16(regs, PCI_PIIX_USB_UHCI_USBSTS, sts);
    true
}

/// Walk **one** UHCI transfer descriptor from the current frame-list slot.
///
/// Spec: UHCI 1.1 §§2.1 / 3.1–3.2 — with USBCMD.RS set, read
/// `FLBASEADD + (FRNUM & 0x3FF) × 4`, follow one optional QH element link to a
/// TD, and if Active perform a single IN (device→guest) or OUT/SETUP
/// (guest→device) copy via callbacks. Clears Active, writes Actual Length,
/// and latches USBSTS.USBINT when IOC was set. Does **not** advance FRNUM.
///
/// `regs` is the 32-byte BAR0 I/O file owned by [`crate::pci::PciConfig`].
/// Requires PCI Bus Master Enable (checked by the caller / wrapper).
pub fn run_one_td<R, W>(
    regs: &mut [u8; PCI_PIIX_USB_UHCI_IO_SIZE as usize],
    bus_master: bool,
    device_buf: &mut [u8],
    mut mem_read: R,
    mut mem_write: W,
) -> Result<UhciTdTransfer, UhciTdError>
where
    R: FnMut(u32) -> u8,
    W: FnMut(u32, u8),
{
    if !bus_master {
        return Err(UhciTdError::BusMasterDisabled);
    }
    let usbcmd = reg_u16(regs, PCI_PIIX_USB_UHCI_USBCMD);
    if usbcmd & UHCI_USBCMD_RS == 0 {
        return Err(UhciTdError::NotRunning);
    }

    let flbase = reg_u32(regs, PCI_PIIX_USB_UHCI_FLBASEADD) & !0xFFF;
    let frnum = (reg_u16(regs, PCI_PIIX_USB_UHCI_FRNUM) & 0x3FF) as u32;
    let frame_entry_addr = flbase.wrapping_add(frnum.wrapping_mul(4));
    let link = read_phys_u32(&mut mem_read, frame_entry_addr);
    let td_addr = resolve_td_addr(&mut mem_read, link)?;
    execute_td_at(regs, device_buf, &mut mem_read, &mut mem_write, td_addr)
}

/// Walk up to `max_frames` frame-list slots starting at FRNUM.
///
/// Spec: UHCI 1.1 §§3.1 / 3.3 — each 1 ms frame selects
/// `FLBASEADD[(FRNUM & 0x3FF)]`; this stub processes that link (TD or QH with
/// up to [`UHCI_MAX_QH_HORIZONTAL`] horizontal hops), executes active TDs, then
/// advances FRNUM. Empty / inactive frames count as scanned but are not errors.
///
/// Unsupported (explicit): isochronous bandwidth reclamation, full QH
/// breadth-first reclaim, multi-packet TD vertical chains, depth >
/// [`UHCI_MAX_QH_HORIZONTAL`].
pub fn run_n_frames<R, W>(
    regs: &mut [u8; PCI_PIIX_USB_UHCI_IO_SIZE as usize],
    bus_master: bool,
    max_frames: u32,
    device_buf: &mut [u8],
    mut mem_read: R,
    mut mem_write: W,
) -> Result<UhciFrameWalkSummary, UhciTdError>
where
    R: FnMut(u32) -> u8,
    W: FnMut(u32, u8),
{
    if max_frames == 0 || max_frames > UHCI_MAX_FRAMES_WALK {
        return Err(UhciTdError::InvalidFrameCount);
    }
    if !bus_master {
        return Err(UhciTdError::BusMasterDisabled);
    }
    let usbcmd = reg_u16(regs, PCI_PIIX_USB_UHCI_USBCMD);
    if usbcmd & UHCI_USBCMD_RS == 0 {
        return Err(UhciTdError::NotRunning);
    }

    let flbase = reg_u32(regs, PCI_PIIX_USB_UHCI_FLBASEADD) & !0xFFF;
    let mut tds_completed = 0u32;
    let mut bytes_copied = 0usize;
    let mut buf_offset = 0usize;

    for _ in 0..max_frames {
        let frnum = (reg_u16(regs, PCI_PIIX_USB_UHCI_FRNUM) & 0x3FF) as u32;
        let frame_entry_addr = flbase.wrapping_add(frnum.wrapping_mul(4));
        let link = read_phys_u32(&mut mem_read, frame_entry_addr);

        let mut td_addrs = [0u32; UHCI_MAX_FRAME_TDS];
        match collect_frame_td_addrs(&mut mem_read, link, &mut td_addrs) {
            Ok(n) => {
                for &td_addr in td_addrs.iter().take(n) {
                    let slice = if buf_offset < device_buf.len() {
                        &mut device_buf[buf_offset..]
                    } else {
                        &mut []
                    };
                    match execute_td_at(regs, slice, &mut mem_read, &mut mem_write, td_addr) {
                        Ok(xfer) => {
                            tds_completed += 1;
                            bytes_copied += xfer.bytes_copied;
                            buf_offset = buf_offset.saturating_add(xfer.bytes_copied);
                        }
                        Err(UhciTdError::NothingToDo) | Err(UhciTdError::EmptyBuffer) => {}
                        Err(e) => return Err(e),
                    }
                }
            }
            Err(UhciTdError::NothingToDo) => {}
            Err(e) => return Err(e),
        }
        bump_frnum(regs);
    }

    let usbint = reg_u16(regs, PCI_PIIX_USB_UHCI_USBSTS) & UHCI_USBSTS_USBINT != 0;
    Ok(UhciFrameWalkSummary {
        frames_scanned: max_frames,
        tds_completed,
        bytes_copied,
        usbint,
    })
}

fn status_was_ioc_and_latch(
    regs: &mut [u8; PCI_PIIX_USB_UHCI_IO_SIZE as usize],
    status_before_clear: u32,
) -> bool {
    // Caller already cleared Active; check IOC from the value written path used IOC bit.
    // For zero-length we pass post-update status which still retains IOC.
    if status_before_clear & UHCI_TD_IOC != 0 {
        let sts = reg_u16(regs, PCI_PIIX_USB_UHCI_USBSTS) | UHCI_USBSTS_USBINT;
        set_reg_u16(regs, PCI_PIIX_USB_UHCI_USBSTS, sts);
        true
    } else {
        false
    }
}

fn portsc_offset(port_index: u8) -> Result<u8, UhciTdError> {
    match port_index {
        0 => Ok(PCI_PIIX_USB_UHCI_PORTSC1),
        1 => Ok(PCI_PIIX_USB_UHCI_PORTSC2),
        _ => Err(UhciTdError::InvalidPortIndex),
    }
}

/// Read PORTSCn with RO overlays (UHCI 1.1 §2.1.7 — bit 10 always 1).
pub fn portsc_read(
    regs: &[u8; PCI_PIIX_USB_UHCI_IO_SIZE as usize],
    port_index: u8,
) -> Result<u16, UhciTdError> {
    let off = portsc_offset(port_index)?;
    Ok(reg_u16(regs, off) | UHCI_PORTSC_RESERVED1)
}

/// Guest PORTSC write: R/WC CSC/PEDC, retain PED/PR, preserve RO CCS/LS.
///
/// Spec: UHCI 1.1 §2.1.7 — firmware probe typically pulses PR then enables PED
/// when CCS is set. Ending reset (PR 1→0) while CCS is set auto-sets PED and
/// PEDC so a connect is enabled after reset.
pub fn portsc_write(
    regs: &mut [u8; PCI_PIIX_USB_UHCI_IO_SIZE as usize],
    port_index: u8,
    value: u16,
) -> Result<u16, UhciTdError> {
    let off = portsc_offset(port_index)?;
    let old = reg_u16(regs, off);
    let ro = old & (UHCI_PORTSC_CCS | UHCI_PORTSC_LS);
    let mut next = ro;
    next |= value & UHCI_PORTSC_WRITABLE;
    // R/WC: write-1 clears CSC / PEDC.
    let mut csc = old & UHCI_PORTSC_CSC;
    let mut pedc = old & UHCI_PORTSC_PEDC;
    if value & UHCI_PORTSC_CSC != 0 {
        csc = 0;
    }
    if value & UHCI_PORTSC_PEDC != 0 {
        pedc = 0;
    }
    // Reset end (PR 1→0) with device present → enable port.
    let pr_was = old & UHCI_PORTSC_PR != 0;
    let pr_now = next & UHCI_PORTSC_PR != 0;
    if pr_was && !pr_now && ro & UHCI_PORTSC_CCS != 0 && next & UHCI_PORTSC_PED == 0 {
        next |= UHCI_PORTSC_PED;
        pedc = UHCI_PORTSC_PEDC;
    }
    // Software PED clear while connected latches PEDC.
    if old & UHCI_PORTSC_PED != 0 && next & UHCI_PORTSC_PED == 0 {
        pedc = UHCI_PORTSC_PEDC;
    }
    next |= csc | pedc | UHCI_PORTSC_RESERVED1;
    set_reg_u16(regs, off, next);
    Ok(next)
}

/// Host: attach a device on PORTSCn — sets CCS (+ optional LS) and CSC.
///
/// Spec: UHCI 1.1 §2.1.7 — CCS is RO to software; the HC updates it on connect.
pub fn portsc_attach_device(
    regs: &mut [u8; PCI_PIIX_USB_UHCI_IO_SIZE as usize],
    port_index: u8,
    low_speed: bool,
) -> Result<u16, UhciTdError> {
    let off = portsc_offset(port_index)?;
    let mut v = reg_u16(regs, off);
    let was = v & UHCI_PORTSC_CCS != 0;
    v |= UHCI_PORTSC_CCS | UHCI_PORTSC_RESERVED1;
    if low_speed {
        v |= UHCI_PORTSC_LS;
    } else {
        v &= !UHCI_PORTSC_LS;
    }
    if !was {
        v |= UHCI_PORTSC_CSC;
    }
    set_reg_u16(regs, off, v);
    Ok(v | UHCI_PORTSC_RESERVED1)
}

/// Host: detach device — clears CCS/LS/PED, sets CSC (+ PEDC if was enabled).
pub fn portsc_detach_device(
    regs: &mut [u8; PCI_PIIX_USB_UHCI_IO_SIZE as usize],
    port_index: u8,
) -> Result<u16, UhciTdError> {
    let off = portsc_offset(port_index)?;
    let mut v = reg_u16(regs, off);
    let was_ccs = v & UHCI_PORTSC_CCS != 0;
    let was_ped = v & UHCI_PORTSC_PED != 0;
    v &= !(UHCI_PORTSC_CCS | UHCI_PORTSC_LS | UHCI_PORTSC_PED | UHCI_PORTSC_PR);
    if was_ccs {
        v |= UHCI_PORTSC_CSC;
    }
    if was_ped {
        v |= UHCI_PORTSC_PEDC;
    }
    v |= UHCI_PORTSC_RESERVED1;
    set_reg_u16(regs, off, v);
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pci::{
        PciConfig, PCI_COMMAND_BUS_MASTER, PCI_COMMAND_IO, PCI_COMMAND_OFFSET,
        PCI_DEVICE_PIIX3_USB, PCI_PIIX_USB_BAR0_OFFSET, PCI_PIIX_USB_UHCI_FLBASEADD,
        PCI_PIIX_USB_UHCI_FRNUM, PCI_PIIX_USB_UHCI_IO_SIZE, PCI_PIIX_USB_UHCI_USBCMD,
        PCI_PIIX_USB_UHCI_USBSTS,
    };
    use crate::PortDevice;
    use std::cell::RefCell;

    /// Guest RAM for schedule + TD + buffer (sparse HashMap behind RefCell).
    struct FakeMem {
        bytes: RefCell<std::collections::HashMap<u32, u8>>,
    }

    impl FakeMem {
        fn new() -> Self {
            Self {
                bytes: RefCell::new(std::collections::HashMap::new()),
            }
        }
        fn write_u32(&self, addr: u32, value: u32) {
            let mut bytes = self.bytes.borrow_mut();
            for (i, b) in value.to_le_bytes().into_iter().enumerate() {
                bytes.insert(addr.wrapping_add(i as u32), b);
            }
        }
        fn read_u32(&self, addr: u32) -> u32 {
            let bytes = self.bytes.borrow();
            u32::from_le_bytes([
                bytes.get(&addr).copied().unwrap_or(0),
                bytes.get(&addr.wrapping_add(1)).copied().unwrap_or(0),
                bytes.get(&addr.wrapping_add(2)).copied().unwrap_or(0),
                bytes.get(&addr.wrapping_add(3)).copied().unwrap_or(0),
            ])
        }
        fn write_bytes(&self, addr: u32, data: &[u8]) {
            let mut bytes = self.bytes.borrow_mut();
            for (i, b) in data.iter().enumerate() {
                bytes.insert(addr.wrapping_add(i as u32), *b);
            }
        }
        fn read_bytes(&self, addr: u32, len: usize) -> Vec<u8> {
            let bytes = self.bytes.borrow();
            (0..len)
                .map(|i| {
                    bytes
                        .get(&addr.wrapping_add(i as u32))
                        .copied()
                        .unwrap_or(0)
                })
                .collect()
        }
        fn get(&self, addr: u32) -> u8 {
            self.bytes.borrow().get(&addr).copied().unwrap_or(0)
        }
        fn set(&self, addr: u32, val: u8) {
            self.bytes.borrow_mut().insert(addr, val);
        }
    }

    fn enable_uhci_io(pci: &mut PciConfig, bar: u16) {
        // Select PIIX USB function and program BAR0 + Command IO|BM.
        pci.port_write(
            0xCF8,
            4,
            PciConfig::make_address(0, 1, 2, PCI_PIIX_USB_BAR0_OFFSET, true),
        );
        pci.port_write(0xCFC, 4, u32::from(bar) | 1);
        pci.port_write(
            0xCF8,
            4,
            PciConfig::make_address(0, 1, 2, PCI_COMMAND_OFFSET, true),
        );
        pci.port_write(0xCFC, 2, u32::from(PCI_COMMAND_IO | PCI_COMMAND_BUS_MASTER));
        let _ = PCI_DEVICE_PIIX3_USB;
    }

    fn write_uhci_reg(pci: &mut PciConfig, bar: u16, off: u8, size: u8, value: u32) {
        pci.port_write(bar + u16::from(off), size, value);
    }

    fn read_uhci_reg(pci: &mut PciConfig, bar: u16, off: u8, size: u8) -> u32 {
        pci.port_read(bar + u16::from(off), size)
    }

    /// Spec: UHCI 1.1 §3.1–3.2 — RS + frame-list TD IN copies device→guest, clears Active.
    #[test]
    fn one_td_in_copies_and_clears_active() {
        let mut pci = PciConfig::new();
        let bar = 0xD000u16;
        enable_uhci_io(&mut pci, bar);

        let flbase = 0x0001_0000u32;
        let td_addr = 0x0001_1000u32;
        let buf_addr = 0x0001_2000u32;
        let mem = FakeMem::new();
        // Frame 0 → TD.
        mem.write_u32(flbase, td_addr);
        // TD: link T=1, status Active|IOC, token IN maxlen=3, buffer.
        mem.write_u32(td_addr, UHCI_LINK_TERMINATE);
        mem.write_u32(td_addr + 4, UHCI_TD_ACTIVE | UHCI_TD_IOC);
        let token = u32::from(UHCI_PID_IN) | ((3 - 1) << UHCI_TD_MAXLEN_SHIFT);
        mem.write_u32(td_addr + 8, token);
        mem.write_u32(td_addr + 12, buf_addr);

        write_uhci_reg(&mut pci, bar, PCI_PIIX_USB_UHCI_FLBASEADD, 4, flbase);
        write_uhci_reg(&mut pci, bar, PCI_PIIX_USB_UHCI_FRNUM, 2, 0);
        write_uhci_reg(
            &mut pci,
            bar,
            PCI_PIIX_USB_UHCI_USBCMD,
            2,
            u32::from(UHCI_USBCMD_RS),
        );

        let mut device = [0xAAu8, 0xBB, 0xCC, 0xDD];
        let result = pci
            .run_one_uhci_td(&mut device, |a| mem.get(a), |a, v| mem.set(a, v))
            .expect("IN TD");
        assert_eq!(result.td_addr, td_addr);
        assert_eq!(result.pid, UHCI_PID_IN);
        assert_eq!(result.bytes_copied, 3);
        assert!(result.usbint);
        assert_eq!(mem.read_bytes(buf_addr, 3), vec![0xAA, 0xBB, 0xCC]);
        let status = mem.read_u32(td_addr + 4);
        assert_eq!(status & UHCI_TD_ACTIVE, 0);
        assert_eq!(status & UHCI_TD_ACTLEN_MASK, encode_actlen(3));
        assert_ne!(
            read_uhci_reg(&mut pci, bar, PCI_PIIX_USB_UHCI_USBSTS, 2) as u16 & UHCI_USBSTS_USBINT,
            0
        );
    }

    /// Spec: UHCI 1.1 — OUT moves guest→device buffer; QH element link allowed once.
    #[test]
    fn one_td_out_via_qh_element() {
        let mut pci = PciConfig::new();
        let bar = 0xD000u16;
        enable_uhci_io(&mut pci, bar);

        let flbase = 0x0002_0000u32;
        let qh_addr = 0x0002_0100u32;
        let td_addr = 0x0002_0200u32;
        let buf_addr = 0x0002_0300u32;
        let mem = FakeMem::new();
        mem.write_u32(flbase, qh_addr | UHCI_LINK_QH);
        mem.write_u32(qh_addr, UHCI_LINK_TERMINATE); // horizontal
        mem.write_u32(qh_addr + 4, td_addr); // element → TD
        mem.write_u32(td_addr, UHCI_LINK_TERMINATE);
        mem.write_u32(td_addr + 4, UHCI_TD_ACTIVE); // no IOC
        let token = u32::from(UHCI_PID_OUT) | ((2 - 1) << UHCI_TD_MAXLEN_SHIFT);
        mem.write_u32(td_addr + 8, token);
        mem.write_u32(td_addr + 12, buf_addr);
        mem.write_bytes(buf_addr, &[0x11, 0x22]);

        write_uhci_reg(&mut pci, bar, PCI_PIIX_USB_UHCI_FLBASEADD, 4, flbase);
        write_uhci_reg(
            &mut pci,
            bar,
            PCI_PIIX_USB_UHCI_USBCMD,
            2,
            u32::from(UHCI_USBCMD_RS),
        );

        let mut device = [0u8; 4];
        let result = pci
            .run_one_uhci_td(&mut device, |a| mem.get(a), |a, v| mem.set(a, v))
            .expect("OUT TD");
        assert_eq!(result.bytes_copied, 2);
        assert!(!result.usbint);
        assert_eq!(&device[..2], &[0x11, 0x22]);
        assert_eq!(
            read_uhci_reg(&mut pci, bar, PCI_PIIX_USB_UHCI_USBSTS, 2) as u16 & UHCI_USBSTS_USBINT,
            0
        );
    }

    #[test]
    fn run_bit_and_bus_master_gate() {
        let mut pci = PciConfig::new();
        let bar = 0xD000u16;
        enable_uhci_io(&mut pci, bar);
        let mut device = [0u8; 1];
        let mem = FakeMem::new();
        // RS clear.
        assert_eq!(
            pci.run_one_uhci_td(&mut device, |_| 0, |_, _| {}),
            Err(UhciTdError::NotRunning)
        );
        write_uhci_reg(
            &mut pci,
            bar,
            PCI_PIIX_USB_UHCI_USBCMD,
            2,
            u32::from(UHCI_USBCMD_RS),
        );
        // Empty frame list (T=1).
        write_uhci_reg(&mut pci, bar, PCI_PIIX_USB_UHCI_FLBASEADD, 4, 0x3000);
        mem.write_u32(0x3000, UHCI_LINK_TERMINATE);
        assert_eq!(
            pci.run_one_uhci_td(&mut device, |a| mem.get(a), |a, v| mem.set(a, v)),
            Err(UhciTdError::NothingToDo)
        );

        // Bus master off.
        pci.port_write(
            0xCF8,
            4,
            PciConfig::make_address(0, 1, 2, PCI_COMMAND_OFFSET, true),
        );
        pci.port_write(0xCFC, 2, u32::from(PCI_COMMAND_IO));
        assert_eq!(
            pci.run_one_uhci_td(&mut device, |_| 0, |_, _| {}),
            Err(UhciTdError::BusMasterDisabled)
        );
    }

    #[test]
    fn reset_clears_uhci_io_used_by_stub() {
        let mut pci = PciConfig::new();
        let bar = 0xD000u16;
        enable_uhci_io(&mut pci, bar);
        write_uhci_reg(
            &mut pci,
            bar,
            PCI_PIIX_USB_UHCI_USBCMD,
            2,
            u32::from(UHCI_USBCMD_RS),
        );
        write_uhci_reg(&mut pci, bar, PCI_PIIX_USB_UHCI_FLBASEADD, 4, 0xABCD_1000);
        pci.reset();
        // After reset BAR decode is off and register file is zero.
        assert_eq!(pci.uhci_io_base(), None);
        assert!(pci.uhci_io.iter().all(|&b| b == 0));
    }

    /// Spec: UHCI 1.1 §3.1 — walk N frame-list slots and advance FRNUM.
    #[test]
    fn n_frames_walks_two_slots_and_advances_frnum() {
        let mut pci = PciConfig::new();
        let bar = 0xD000u16;
        enable_uhci_io(&mut pci, bar);

        let flbase = 0x0003_0000u32;
        let td0 = 0x0003_1000u32;
        let td1 = 0x0003_1100u32;
        let buf0 = 0x0003_2000u32;
        let buf1 = 0x0003_2100u32;
        let mem = FakeMem::new();
        mem.write_u32(flbase, td0);
        mem.write_u32(flbase + 4, td1);
        for (td, buf, pid_byte) in [(td0, buf0, 0xAAu8), (td1, buf1, 0xBBu8)] {
            mem.write_u32(td, UHCI_LINK_TERMINATE);
            mem.write_u32(td + 4, UHCI_TD_ACTIVE);
            let token = u32::from(UHCI_PID_IN) | ((1 - 1) << UHCI_TD_MAXLEN_SHIFT);
            mem.write_u32(td + 8, token);
            mem.write_u32(td + 12, buf);
            let _ = pid_byte;
        }
        // maxlen=1 for each
        mem.write_u32(
            td0 + 8,
            u32::from(UHCI_PID_IN) | ((1 - 1) << UHCI_TD_MAXLEN_SHIFT),
        );
        mem.write_u32(
            td1 + 8,
            u32::from(UHCI_PID_IN) | ((1 - 1) << UHCI_TD_MAXLEN_SHIFT),
        );

        write_uhci_reg(&mut pci, bar, PCI_PIIX_USB_UHCI_FLBASEADD, 4, flbase);
        write_uhci_reg(&mut pci, bar, PCI_PIIX_USB_UHCI_FRNUM, 2, 0);
        write_uhci_reg(
            &mut pci,
            bar,
            PCI_PIIX_USB_UHCI_USBCMD,
            2,
            u32::from(UHCI_USBCMD_RS),
        );

        let mut device = [0xAA, 0xBB];
        let bus_master = true;
        let summary = run_n_frames(
            &mut pci.uhci_io,
            bus_master,
            2,
            &mut device,
            |a| mem.get(a),
            |a, v| mem.set(a, v),
        )
        .expect("2-frame walk");
        assert_eq!(summary.frames_scanned, 2);
        assert_eq!(summary.tds_completed, 2);
        assert_eq!(summary.bytes_copied, 2);
        assert_eq!(mem.get(buf0), 0xAA);
        assert_eq!(mem.get(buf1), 0xBB);
        assert_eq!(
            read_uhci_reg(&mut pci, bar, PCI_PIIX_USB_UHCI_FRNUM, 2) as u16 & 0x3FF,
            2
        );
    }

    /// Spec: UHCI 1.1 §3.3 — one QH horizontal hop to a second element TD.
    #[test]
    fn qh_horizontal_hop_executes_second_td() {
        let mut pci = PciConfig::new();
        let bar = 0xD000u16;
        enable_uhci_io(&mut pci, bar);

        let flbase = 0x0004_0000u32;
        let qh0 = 0x0004_0100u32;
        let qh1 = 0x0004_0120u32;
        let td0 = 0x0004_0200u32;
        let td1 = 0x0004_0220u32;
        let buf0 = 0x0004_0300u32;
        let buf1 = 0x0004_0310u32;
        let mem = FakeMem::new();
        mem.write_u32(flbase, qh0 | UHCI_LINK_QH);
        mem.write_u32(qh0, qh1 | UHCI_LINK_QH); // horizontal → QH1
        mem.write_u32(qh0 + 4, td0);
        mem.write_u32(qh1, UHCI_LINK_TERMINATE);
        mem.write_u32(qh1 + 4, td1);
        for (td, buf) in [(td0, buf0), (td1, buf1)] {
            mem.write_u32(td, UHCI_LINK_TERMINATE);
            mem.write_u32(td + 4, UHCI_TD_ACTIVE);
            mem.write_u32(
                td + 8,
                u32::from(UHCI_PID_IN) | ((1 - 1) << UHCI_TD_MAXLEN_SHIFT),
            );
            mem.write_u32(td + 12, buf);
        }

        write_uhci_reg(&mut pci, bar, PCI_PIIX_USB_UHCI_FLBASEADD, 4, flbase);
        write_uhci_reg(
            &mut pci,
            bar,
            PCI_PIIX_USB_UHCI_USBCMD,
            2,
            u32::from(UHCI_USBCMD_RS),
        );

        let mut device = [0x11, 0x22];
        let summary = run_n_frames(
            &mut pci.uhci_io,
            true,
            1,
            &mut device,
            |a| mem.get(a),
            |a, v| mem.set(a, v),
        )
        .expect("QH horizontal");
        assert_eq!(summary.tds_completed, 2);
        assert_eq!(mem.get(buf0), 0x11);
        assert_eq!(mem.get(buf1), 0x22);
    }

    /// Spec: UHCI 1.1 §3.3 — R12 allows up to [`UHCI_MAX_QH_HORIZONTAL`] hops.
    #[test]
    fn qh_horizontal_depth_four_executes_all_tds() {
        let mut pci = PciConfig::new();
        let bar = 0xD000u16;
        enable_uhci_io(&mut pci, bar);

        let flbase = 0x0006_0000u32;
        // Five QHs: start + 4 horizontal hops (MAX=4).
        let qhs: [u32; 5] = [
            0x0006_0100,
            0x0006_0120,
            0x0006_0140,
            0x0006_0160,
            0x0006_0180,
        ];
        let tds: [u32; 5] = [
            0x0006_0200,
            0x0006_0220,
            0x0006_0240,
            0x0006_0260,
            0x0006_0280,
        ];
        let bufs: [u32; 5] = [
            0x0006_0300,
            0x0006_0310,
            0x0006_0320,
            0x0006_0330,
            0x0006_0340,
        ];
        let mem = FakeMem::new();
        mem.write_u32(flbase, qhs[0] | UHCI_LINK_QH);
        for i in 0..5 {
            if i + 1 < 5 {
                mem.write_u32(qhs[i], qhs[i + 1] | UHCI_LINK_QH);
            } else {
                mem.write_u32(qhs[i], UHCI_LINK_TERMINATE);
            }
            mem.write_u32(qhs[i] + 4, tds[i]);
            mem.write_u32(tds[i], UHCI_LINK_TERMINATE);
            mem.write_u32(tds[i] + 4, UHCI_TD_ACTIVE);
            mem.write_u32(
                tds[i] + 8,
                u32::from(UHCI_PID_IN) | ((1 - 1) << UHCI_TD_MAXLEN_SHIFT),
            );
            mem.write_u32(tds[i] + 12, bufs[i]);
        }

        write_uhci_reg(&mut pci, bar, PCI_PIIX_USB_UHCI_FLBASEADD, 4, flbase);
        write_uhci_reg(
            &mut pci,
            bar,
            PCI_PIIX_USB_UHCI_USBCMD,
            2,
            u32::from(UHCI_USBCMD_RS),
        );

        let mut device = [0xA0, 0xA1, 0xA2, 0xA3, 0xA4];
        let summary = run_n_frames(
            &mut pci.uhci_io,
            true,
            1,
            &mut device,
            |a| mem.get(a),
            |a, v| mem.set(a, v),
        )
        .expect("QH horizontal depth 4");
        assert_eq!(summary.tds_completed, 5);
        assert_eq!(UHCI_MAX_QH_HORIZONTAL, 4);
        for (i, &buf) in bufs.iter().enumerate() {
            assert_eq!(mem.get(buf), 0xA0 + i as u8);
        }
    }

    /// Spec: UHCI 1.1 §3.3 — hop beyond [`UHCI_MAX_QH_HORIZONTAL`] is unsupported.
    #[test]
    fn qh_horizontal_depth_five_unsupported() {
        let mut pci = PciConfig::new();
        let bar = 0xD000u16;
        enable_uhci_io(&mut pci, bar);
        let flbase = 0x0005_0000u32;
        // Six QHs → five hops > MAX=4.
        let qhs: [u32; 6] = [
            0x0005_0100,
            0x0005_0120,
            0x0005_0140,
            0x0005_0160,
            0x0005_0180,
            0x0005_01A0,
        ];
        let td = 0x0005_0200u32;
        let mem = FakeMem::new();
        mem.write_u32(flbase, qhs[0] | UHCI_LINK_QH);
        for i in 0..6 {
            if i + 1 < 6 {
                mem.write_u32(qhs[i], qhs[i + 1] | UHCI_LINK_QH);
            } else {
                mem.write_u32(qhs[i], UHCI_LINK_TERMINATE);
            }
            // Only the second QH has an active TD so walk does not early-exit
            // before hitting the depth cap on the fifth hop.
            if i == 1 {
                mem.write_u32(qhs[i] + 4, td);
            } else {
                mem.write_u32(qhs[i] + 4, UHCI_LINK_TERMINATE);
            }
        }
        mem.write_u32(td, UHCI_LINK_TERMINATE);
        mem.write_u32(td + 4, UHCI_TD_ACTIVE);
        mem.write_u32(
            td + 8,
            u32::from(UHCI_PID_IN) | ((1 - 1) << UHCI_TD_MAXLEN_SHIFT),
        );
        mem.write_u32(td + 12, 0x0005_0300);

        write_uhci_reg(&mut pci, bar, PCI_PIIX_USB_UHCI_FLBASEADD, 4, flbase);
        write_uhci_reg(
            &mut pci,
            bar,
            PCI_PIIX_USB_UHCI_USBCMD,
            2,
            u32::from(UHCI_USBCMD_RS),
        );
        let mut device = [0u8; 1];
        assert_eq!(
            run_n_frames(
                &mut pci.uhci_io,
                true,
                1,
                &mut device,
                |a| mem.get(a),
                |a, v| mem.set(a, v),
            ),
            Err(UhciTdError::QueueHeadHorizontalUnsupported)
        );
    }

    /// Spec: UHCI 1.1 §2.1.7 — CCS/PED/PR enough for firmware connect+reset probe.
    #[test]
    fn portsc_attach_reset_enables_ped() {
        let mut regs = [0u8; PCI_PIIX_USB_UHCI_IO_SIZE as usize];
        let v = portsc_attach_device(&mut regs, 0, false).unwrap();
        assert_ne!(v & UHCI_PORTSC_CCS, 0);
        assert_ne!(v & UHCI_PORTSC_CSC, 0);
        assert_ne!(v & UHCI_PORTSC_RESERVED1, 0);
        assert_eq!(v & UHCI_PORTSC_PED, 0);

        // Pulse PR.
        let _ = portsc_write(&mut regs, 0, UHCI_PORTSC_PR | UHCI_PORTSC_CSC).unwrap();
        let mid = portsc_read(&regs, 0).unwrap();
        assert_ne!(mid & UHCI_PORTSC_PR, 0);
        assert_eq!(mid & UHCI_PORTSC_CSC, 0); // W1C

        // End reset → PED + PEDC.
        let end = portsc_write(&mut regs, 0, 0).unwrap();
        assert_eq!(end & UHCI_PORTSC_PR, 0);
        assert_ne!(end & UHCI_PORTSC_PED, 0);
        assert_ne!(end & UHCI_PORTSC_PEDC, 0);
        assert_ne!(end & UHCI_PORTSC_CCS, 0);
    }

    #[test]
    fn portsc_detach_clears_ccs_and_ped() {
        let mut regs = [0u8; PCI_PIIX_USB_UHCI_IO_SIZE as usize];
        portsc_attach_device(&mut regs, 1, true).unwrap();
        portsc_write(&mut regs, 1, UHCI_PORTSC_PR).unwrap();
        portsc_write(&mut regs, 1, 0).unwrap(); // end reset → PED
        let v = portsc_detach_device(&mut regs, 1).unwrap();
        assert_eq!(v & UHCI_PORTSC_CCS, 0);
        assert_eq!(v & UHCI_PORTSC_LS, 0);
        assert_eq!(v & UHCI_PORTSC_PED, 0);
        assert_ne!(v & UHCI_PORTSC_CSC, 0);
        assert_ne!(v & UHCI_PORTSC_PEDC, 0);
    }

    /// Spec: UHCI 1.1 §2.1.2/§2.1.3 — IOC latches USBINT; USBINTR.IOC gates host IRQ.
    #[test]
    fn usbsts_usbint_gated_by_usbintr_ioc() {
        let mut pci = PciConfig::new();
        let bar = 0xD000u16;
        enable_uhci_io(&mut pci, bar);

        let flbase = 0x0007_0000u32;
        let td = 0x0007_1000u32;
        let buf = 0x0007_2000u32;
        let mem = FakeMem::new();
        mem.write_u32(flbase, td);
        mem.write_u32(td, UHCI_LINK_TERMINATE);
        mem.write_u32(td + 4, UHCI_TD_ACTIVE | UHCI_TD_IOC);
        mem.write_u32(
            td + 8,
            u32::from(UHCI_PID_IN) | ((1 - 1) << UHCI_TD_MAXLEN_SHIFT),
        );
        mem.write_u32(td + 12, buf);

        write_uhci_reg(&mut pci, bar, PCI_PIIX_USB_UHCI_FLBASEADD, 4, flbase);
        write_uhci_reg(
            &mut pci,
            bar,
            PCI_PIIX_USB_UHCI_USBCMD,
            2,
            u32::from(UHCI_USBCMD_RS),
        );
        // USBINTR.IOC clear — status still latches, host IRQ not pending.
        usbintr_write(&mut pci.uhci_io, 0);

        let mut device = [0x55];
        let xfer = run_one_td(
            &mut pci.uhci_io,
            true,
            &mut device,
            |a| mem.get(a),
            |a, v| mem.set(a, v),
        )
        .expect("IOC TD");
        assert!(xfer.usbint);
        assert_ne!(usbsts_read(&pci.uhci_io) & UHCI_USBSTS_USBINT, 0);
        assert!(!uhci_interrupt_pending(&pci.uhci_io));

        usbintr_write(&mut pci.uhci_io, UHCI_USBINTR_IOC);
        assert!(uhci_interrupt_pending(&pci.uhci_io));

        usbsts_write_w1c(&mut pci.uhci_io, UHCI_USBSTS_USBINT);
        assert_eq!(usbsts_read(&pci.uhci_io) & UHCI_USBSTS_USBINT, 0);
        assert!(!uhci_interrupt_pending(&pci.uhci_io));
    }

    /// Spec: UHCI 1.1 §2.1.2/§2.1.3 — USBERRINT gated by Timeout/CRC enable.
    #[test]
    fn usbsts_usberrint_gated_by_crc_enable() {
        let mut regs = [0u8; PCI_PIIX_USB_UHCI_IO_SIZE as usize];
        latch_usb_error(&mut regs);
        assert_ne!(usbsts_read(&regs) & UHCI_USBSTS_USBERRINT, 0);
        assert!(!uhci_interrupt_pending(&regs));
        usbintr_write(&mut regs, UHCI_USBINTR_CRC);
        assert!(uhci_interrupt_pending(&regs));
        usbsts_write_w1c(&mut regs, UHCI_USBSTS_USBERRINT);
        assert_eq!(usbsts_read(&regs) & UHCI_USBSTS_USBERRINT, 0);
        assert!(!uhci_interrupt_pending(&regs));
    }

    /// Spec: UHCI 1.1 §2.1.2 — Resume Detect only while Global Suspend.
    #[test]
    fn usbsts_resume_detect_requires_gsuspend() {
        let mut regs = [0u8; PCI_PIIX_USB_UHCI_IO_SIZE as usize];
        assert!(!latch_resume_detect(&mut regs));
        assert_eq!(usbsts_read(&regs) & UHCI_USBSTS_RD, 0);

        set_reg_u16(&mut regs, PCI_PIIX_USB_UHCI_USBCMD, UHCI_USBCMD_GSUSPEND);
        assert!(latch_resume_detect(&mut regs));
        usbintr_write(&mut regs, UHCI_USBINTR_RESUME);
        assert!(uhci_interrupt_pending(&regs));
        usbsts_write_w1c(&mut regs, UHCI_USBSTS_RD);
        assert!(!uhci_interrupt_pending(&regs));
    }

    /// Spec: UHCI 1.1 §2.1.2 — HCPE is unmaskable.
    #[test]
    fn usbsts_hcpe_unmaskable() {
        let mut regs = [0u8; PCI_PIIX_USB_UHCI_IO_SIZE as usize];
        let sts = reg_u16(&regs, PCI_PIIX_USB_UHCI_USBSTS) | UHCI_USBSTS_HCPE;
        set_reg_u16(&mut regs, PCI_PIIX_USB_UHCI_USBSTS, sts);
        usbintr_write(&mut regs, 0);
        assert!(uhci_interrupt_pending(&regs));
    }

    /// Spec: UHCI 1.1 §2.1.2 — HCHalted overlays when RS clear.
    #[test]
    fn usbsts_hchalted_when_rs_clear() {
        let mut regs = [0u8; PCI_PIIX_USB_UHCI_IO_SIZE as usize];
        assert_ne!(usbsts_read(&regs) & UHCI_USBSTS_HCHALTED, 0);
        set_reg_u16(&mut regs, PCI_PIIX_USB_UHCI_USBCMD, UHCI_USBCMD_RS);
        assert_eq!(usbsts_read(&regs) & UHCI_USBSTS_HCHALTED, 0);
    }

    /// Spec: Intel 82371SB — USB → PIRQD; PIRQRC[D]=IRQ11 → DualPic vector 0x73.
    #[test]
    fn uhci_pending_routes_pirqd_to_classic_irq11() {
        use crate::{
            DualPic, PciConfig, PCI_CONFIG_ADDRESS, PCI_PIIX_ISA_PIRQRC_OFFSET, PIC_MASTER_CMD,
            PIC_MASTER_DATA, PIC_SLAVE_CMD, PIC_SLAVE_DATA,
        };

        assert_eq!(UHCI_PIIX_PIRQD, 3);
        assert_eq!(UHCI_CLASSIC_ISA_IRQ, 11);
        assert_eq!(UHCI_CLASSIC_PIRQRC_D, 0x0B);

        let mut pci = PciConfig::new();
        let mut pic = DualPic::new();
        // Classic AT cascade; unmask slave IR3 (IRQ11) + master cascade IR2.
        pic.port_write(PIC_MASTER_CMD, 1, 0x11);
        pic.port_write(PIC_MASTER_DATA, 1, 0x08);
        pic.port_write(PIC_MASTER_DATA, 1, 0x04);
        pic.port_write(PIC_MASTER_DATA, 1, 0x01);
        pic.port_write(PIC_SLAVE_CMD, 1, 0x11);
        pic.port_write(PIC_SLAVE_DATA, 1, 0x70);
        pic.port_write(PIC_SLAVE_DATA, 1, 0x02);
        pic.port_write(PIC_SLAVE_DATA, 1, 0x01);
        pic.port_write(PIC_MASTER_DATA, 1, 0xFB); // unmask IR2
        pic.port_write(PIC_SLAVE_DATA, 1, 0xF7); // unmask IR3 (IRQ11)

        // PIRQRC[D] at ISA config 0x63 → IRQ11 (CF8 dword-aligned; lane 0xCFF).
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 0, PCI_PIIX_ISA_PIRQRC_OFFSET, true),
        );
        pci.port_write(0xCFF, 1, u32::from(UHCI_CLASSIC_PIRQRC_D));
        assert_eq!(
            crate::pirqrc_routed_irq(pci.pirqrc_byte(UHCI_PIIX_PIRQD)),
            Some(UHCI_CLASSIC_ISA_IRQ)
        );

        usbintr_write(&mut pci.uhci_io, UHCI_USBINTR_IOC);
        latch_usb_interrupt(&mut pci.uhci_io);
        assert!(uhci_interrupt_pending(&pci.uhci_io));

        pci.set_pirq_line(UHCI_PIIX_PIRQD, uhci_interrupt_pending(&pci.uhci_io));
        pci.sync_pirq_to_pic(&mut pic);
        assert_eq!(pic.poll_irq(), Some(0x73)); // 0x70 + IR3
        pic.port_write(PIC_SLAVE_CMD, 1, 0x20);
        pic.port_write(PIC_MASTER_CMD, 1, 0x20);

        // W1C clears USBINT → deassert PIRQD → PIC idle.
        usbsts_write_w1c(&mut pci.uhci_io, UHCI_USBSTS_USBINT);
        assert!(!uhci_interrupt_pending(&pci.uhci_io));
        pci.set_pirq_line(UHCI_PIIX_PIRQD, false);
        pci.sync_pirq_to_pic(&mut pic);
        assert_eq!(pic.poll_irq(), None);
    }

    /// Spec: UHCI 1.1 §2.1.2/§2.1.3 — IOC completion + USBINTR.IOC raises host IRQ.
    #[test]
    fn ioc_completion_raises_host_irq_when_usbintr_ioc_enabled() {
        let mut pci = PciConfig::new();
        let bar = 0xD000u16;
        enable_uhci_io(&mut pci, bar);

        let flbase = 0x0008_0000u32;
        let td = 0x0008_1000u32;
        let buf = 0x0008_2000u32;
        let mem = FakeMem::new();
        mem.write_u32(flbase, td);
        mem.write_u32(td, UHCI_LINK_TERMINATE);
        mem.write_u32(td + 4, UHCI_TD_ACTIVE | UHCI_TD_IOC);
        mem.write_u32(
            td + 8,
            u32::from(UHCI_PID_IN) | ((1 - 1) << UHCI_TD_MAXLEN_SHIFT),
        );
        mem.write_u32(td + 12, buf);

        write_uhci_reg(&mut pci, bar, PCI_PIIX_USB_UHCI_FLBASEADD, 4, flbase);
        write_uhci_reg(
            &mut pci,
            bar,
            PCI_PIIX_USB_UHCI_USBCMD,
            2,
            u32::from(UHCI_USBCMD_RS),
        );
        usbintr_write(&mut pci.uhci_io, UHCI_USBINTR_IOC);

        let mut device = [0xAA];
        let xfer = run_one_td(
            &mut pci.uhci_io,
            true,
            &mut device,
            |a| mem.get(a),
            |a, v| mem.set(a, v),
        )
        .expect("IOC TD");
        assert!(xfer.usbint);
        assert_ne!(usbsts_read(&pci.uhci_io) & UHCI_USBSTS_USBINT, 0);
        assert!(
            uhci_interrupt_pending(&pci.uhci_io),
            "IOC + USBSTS.USBINT + USBINTR.IOC must raise host IRQ"
        );
    }

    /// Spec: UHCI 1.1 §2.1.2 — USBSTS R/WC: write-0 preserves; write-1 clears
    /// only the bits written as 1; HCHalted is not a stored R/WC bit.
    #[test]
    fn usbsts_w1c_preserves_unwritten_bits() {
        let mut regs = [0u8; PCI_PIIX_USB_UHCI_IO_SIZE as usize];
        set_reg_u16(
            &mut regs,
            PCI_PIIX_USB_UHCI_USBSTS,
            UHCI_USBSTS_USBINT | UHCI_USBSTS_USBERRINT | UHCI_USBSTS_HSE,
        );
        // Write-0 must not clear.
        usbsts_write_w1c(&mut regs, 0);
        assert_eq!(
            usbsts_read(&regs) & UHCI_USBSTS_RWC_MASK,
            UHCI_USBSTS_USBINT | UHCI_USBSTS_USBERRINT | UHCI_USBSTS_HSE
        );
        // Clear only USBINT; leave USBERRINT + HSE.
        usbsts_write_w1c(&mut regs, UHCI_USBSTS_USBINT);
        assert_eq!(usbsts_read(&regs) & UHCI_USBSTS_USBINT, 0);
        assert_ne!(usbsts_read(&regs) & UHCI_USBSTS_USBERRINT, 0);
        assert_ne!(usbsts_read(&regs) & UHCI_USBSTS_HSE, 0);
        usbsts_write_w1c(&mut regs, UHCI_USBSTS_USBERRINT | UHCI_USBSTS_HSE);
        assert_eq!(usbsts_read(&regs) & UHCI_USBSTS_RWC_MASK, 0);
    }

    /// Spec: UHCI 1.1 §2.1.2/§2.1.3 — clearing USBINTR.IOC drops host IRQ while
    /// USBSTS.USBINT remains set (pollable); re-enabling IOC reasserts.
    #[test]
    fn usbintr_ioc_disable_drops_pending_without_clearing_usbint() {
        let mut regs = [0u8; PCI_PIIX_USB_UHCI_IO_SIZE as usize];
        set_reg_u16(&mut regs, PCI_PIIX_USB_UHCI_USBSTS, UHCI_USBSTS_USBINT);
        usbintr_write(&mut regs, UHCI_USBINTR_IOC);
        assert!(uhci_interrupt_pending(&regs));
        usbintr_write(&mut regs, 0);
        assert!(!uhci_interrupt_pending(&regs));
        assert_ne!(usbsts_read(&regs) & UHCI_USBSTS_USBINT, 0);
        usbintr_write(&mut regs, UHCI_USBINTR_IOC);
        assert!(uhci_interrupt_pending(&regs));
    }

    /// Spec: UHCI 1.1 §2.1.2 — USBINT remains latched across a second IOC TD
    /// until software W1C-clears it.
    #[test]
    fn usbint_sticky_until_w1c_across_ioc_completions() {
        let mut pci = PciConfig::new();
        let bar = 0xD000u16;
        enable_uhci_io(&mut pci, bar);

        let flbase = 0x0009_0000u32;
        let td = 0x0009_1000u32;
        let buf = 0x0009_2000u32;
        let mem = FakeMem::new();
        mem.write_u32(flbase, td);
        mem.write_u32(td, UHCI_LINK_TERMINATE);
        mem.write_u32(td + 4, UHCI_TD_ACTIVE | UHCI_TD_IOC);
        mem.write_u32(
            td + 8,
            u32::from(UHCI_PID_IN) | ((1 - 1) << UHCI_TD_MAXLEN_SHIFT),
        );
        mem.write_u32(td + 12, buf);

        write_uhci_reg(&mut pci, bar, PCI_PIIX_USB_UHCI_FLBASEADD, 4, flbase);
        write_uhci_reg(
            &mut pci,
            bar,
            PCI_PIIX_USB_UHCI_USBCMD,
            2,
            u32::from(UHCI_USBCMD_RS),
        );
        usbintr_write(&mut pci.uhci_io, UHCI_USBINTR_IOC);

        let mut device = [0x11];
        run_one_td(
            &mut pci.uhci_io,
            true,
            &mut device,
            |a| mem.get(a),
            |a, v| mem.set(a, v),
        )
        .expect("first IOC");
        assert_ne!(usbsts_read(&pci.uhci_io) & UHCI_USBSTS_USBINT, 0);

        // Re-arm Active+IOC; USBINT still set from first completion.
        mem.write_u32(td + 4, UHCI_TD_ACTIVE | UHCI_TD_IOC);
        device[0] = 0x22;
        run_one_td(
            &mut pci.uhci_io,
            true,
            &mut device,
            |a| mem.get(a),
            |a, v| mem.set(a, v),
        )
        .expect("second IOC");
        assert_ne!(usbsts_read(&pci.uhci_io) & UHCI_USBSTS_USBINT, 0);
        assert!(uhci_interrupt_pending(&pci.uhci_io));
        usbsts_write_w1c(&mut pci.uhci_io, UHCI_USBSTS_USBINT);
        assert_eq!(usbsts_read(&pci.uhci_io) & UHCI_USBSTS_USBINT, 0);
        assert!(!uhci_interrupt_pending(&pci.uhci_io));
    }
}
