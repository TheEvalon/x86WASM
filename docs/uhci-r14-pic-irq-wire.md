# UHCI → PIC IRQ wire — Milestone 2 Round 14

## Why

R12 latched USBSTS / gated host IRQ via uhci_interrupt_pending but left DualPic / PIRQ unwired.

## Spec

- Intel 82371SB — USB interrupt hardwired to PIRQD#
- PIRQRC[D] classic route IRQ11 (0x0B)
- UHCI 1.1 §2.1.2 / §2.1.3

## Model

UHCI_PIIX_PIRQD=3; UHCI_CLASSIC_ISA_IRQ=11; uhci_wire::sync_uhci_irq_to_pic; MachineBus::poll_external_irq syncs after ISA sinks. Does not rewrite pci.rs.

## Tests

uhci_pending_routes_pirqd_to_classic_irq11; uhci_pic_irq11_via_poll_external_irq; uhci_pending_pirqrc_disabled_does_not_raise_pic
