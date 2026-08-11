//! UHCI → DualPic interrupt wire (PIIX3 PIRQD path) + PORTSC host helpers.
//!
//! Spec: Intel 82371SB — USB Host Controller interrupt is hardwired to
//! PIRQD#; PIRQRC[D] selects the ISA IRQ. This tree's classic test/route
//! target is IRQ11 (`UHCI_CLASSIC_ISA_IRQ`). See `docs/usb-r14-uhci-pic.md`.
//! Round-15: thin PORTSC attach/reset handshake helpers for firmware probes
//! without rewriting `pci.rs` (`docs/uhci-r15-portsc-reset.md`).
//!
//! Ownership (R14/R15 usb-timer): this module + thin `Machine::sync_uhci_irq_to_pic`
//! / `MachineBus::poll_external_irq` UHCI sync. Does **not** edit `pci.rs`.

use devices::{
    portsc_attach_device, portsc_read, portsc_write, uhci_interrupt_pending, DualPic, PciConfig,
    UhciTdError, UHCI_PIIX_PIRQD, UHCI_PORTSC_CSC, UHCI_PORTSC_PED, UHCI_PORTSC_PEDC,
    UHCI_PORTSC_PR,
};

/// Mirror UHCI host-IRQ pending onto PIRQD and sync through PIRQRC → DualPic.
///
/// Spec: UHCI 1.1 §2.1.2/§2.1.3 (`uhci_interrupt_pending`) + Intel 82371SB
/// (USB → PIRQD# → PIRQRC → ISA IRQ). Returns whether the HC interrupt line
/// is asserted after the sync.
pub fn sync_uhci_irq_to_pic(pci: &mut PciConfig, pic: &mut DualPic) -> bool {
    let pending = uhci_interrupt_pending(&pci.uhci_io);
    pci.set_pirq_line(UHCI_PIIX_PIRQD, pending);
    pci.sync_pirq_to_pic(pic);
    pending
}

/// Host attach + Port Reset pulse ending with PED set (firmware probe path).
///
/// Spec: UHCI 1.1 §2.1.7 — CCS via attach, PR assert/clear, PED after reset end.
/// Operates on `pci.uhci_io` without rewriting PCI config space.
pub fn portsc_attach_and_reset(
    pci: &mut PciConfig,
    port_index: u8,
    low_speed: bool,
) -> Result<u16, UhciTdError> {
    portsc_attach_device(&mut pci.uhci_io, port_index, low_speed)?;
    // Clear CSC so the reset pulse is unambiguous for probes.
    let _ = portsc_write(&mut pci.uhci_io, port_index, UHCI_PORTSC_CSC)?;
    let _ = portsc_write(&mut pci.uhci_io, port_index, UHCI_PORTSC_PR)?;
    let end = portsc_write(&mut pci.uhci_io, port_index, 0)?;
    debug_assert_ne!(end & UHCI_PORTSC_PED, 0);
    debug_assert_ne!(end & UHCI_PORTSC_PEDC, 0);
    portsc_read(&pci.uhci_io, port_index)
}
