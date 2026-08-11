//! UHCI → DualPic interrupt wire (PIIX3 PIRQD path).
//!
//! Spec: Intel 82371SB — USB Host Controller interrupt is hardwired to
//! PIRQD#; PIRQRC[D] selects the ISA IRQ. This tree's classic test/route
//! target is IRQ11 (`UHCI_CLASSIC_ISA_IRQ`). See `docs/uhci-r14-pic-irq-wire.md`.
//!
//! Ownership (R14 usb-timer): this module + thin `Machine::sync_uhci_irq_to_pic`
//! / `MachineBus::poll_external_irq` UHCI sync. Does **not** edit `pci.rs`.

use devices::{uhci_interrupt_pending, DualPic, PciConfig, UHCI_PIIX_PIRQD};

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
