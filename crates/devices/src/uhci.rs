//! UHCI one-TD schedule stub (Intel UHCI / PIIX3 USB).
//!
//! Spec: Universal Host Controller Interface (UHCI) Design Guide, Revision 1.1;
//! Intel 82371SB (PIIX3) USB function. Frame list → (optional QH) → one TD
//! transfer via host memory callbacks. No real USB device stack, no multi-TD
//! queue walk, no isochronous bandwidth accounting.
//!
//! PCI config / BAR0 I/O decode remain in [`crate::pci::PciConfig`]; this module
//! owns schedule semantics only. See `docs/uhci-r8-one-td.md`.

use crate::pci::{
    PCI_PIIX_USB_UHCI_FLBASEADD, PCI_PIIX_USB_UHCI_FRNUM, PCI_PIIX_USB_UHCI_IO_SIZE,
    PCI_PIIX_USB_UHCI_USBCMD, PCI_PIIX_USB_UHCI_USBSTS,
};

/// USBCMD Run/Stop bit (UHCI 1.1 §2.1.1 bit 0).
pub const UHCI_USBCMD_RS: u16 = 1 << 0;

/// USBSTS USB Interrupt (UHCI 1.1 §2.1.2 bit 0) — set on IOC completion.
pub const UHCI_USBSTS_USBINT: u16 = 1 << 0;

/// USBSTS HCHalted (UHCI 1.1 §2.1.2 bit 5) — sticky while RS clear (stub).
pub const UHCI_USBSTS_HCHALTED: u16 = 1 << 5;

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

/// Errors from [`run_one_td`].
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
    /// Token PID is not IN/OUT/SETUP.
    UnsupportedPid(u8),
    /// Empty host/device buffer supplied for a data-bearing transfer.
    EmptyBuffer,
    /// Buffer pointer + length wraps the 32-bit physical address space.
    GuestAddressOverflow {
        phys_addr: u32,
        bytes_requested: usize,
    },
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

/// Resolve the first TD address from a frame-list link (one optional QH hop).
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

/// Walk **one** UHCI transfer descriptor from the current frame-list slot.
///
/// Spec: UHCI 1.1 §§2.1 / 3.1–3.2 — with USBCMD.RS set, read
/// `FLBASEADD + (FRNUM & 0x3FF) × 4`, follow one optional QH element link to a
/// TD, and if Active perform a single IN (device→guest) or OUT/SETUP
/// (guest→device) copy via callbacks. Clears Active, writes Actual Length,
/// and latches USBSTS.USBINT when IOC was set.
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

    let link_ptr = read_phys_u32(&mut mem_read, td_addr);
    let mut status = read_phys_u32(&mut mem_read, td_addr.wrapping_add(4));
    let token = read_phys_u32(&mut mem_read, td_addr.wrapping_add(8));
    let buffer = read_phys_u32(&mut mem_read, td_addr.wrapping_add(12));
    let _ = link_ptr; // one-TD stub does not follow TD link

    if status & UHCI_TD_ACTIVE == 0 {
        return Err(UhciTdError::NothingToDo);
    }

    let pid = (token & UHCI_TD_PID_MASK) as u8;
    let max_len = uhci_len_field(token >> UHCI_TD_MAXLEN_SHIFT).min(UHCI_TD_MAX_TRANSFER);
    if max_len == 0 {
        // Zero-length handshake still completes the TD.
        status = (status & !UHCI_TD_ACTIVE & !UHCI_TD_ACTLEN_MASK) | encode_actlen(0);
        write_phys_u32(&mut mem_write, td_addr.wrapping_add(4), status);
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
            for i in 0..n {
                device_buf[i] = mem_read(buffer.wrapping_add(i as u32));
            }
        }
        other => return Err(UhciTdError::UnsupportedPid(other)),
    }

    let ioc = status & UHCI_TD_IOC != 0;
    status = (status & !UHCI_TD_ACTIVE & !UHCI_TD_ACTLEN_MASK) | encode_actlen(n);
    write_phys_u32(&mut mem_write, td_addr.wrapping_add(4), status);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pci::{
        PciConfig, PCI_COMMAND_BUS_MASTER, PCI_COMMAND_IO, PCI_COMMAND_OFFSET,
        PCI_DEVICE_PIIX3_USB, PCI_PIIX_USB_BAR0_OFFSET, PCI_PIIX_USB_UHCI_USBCMD,
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
}
