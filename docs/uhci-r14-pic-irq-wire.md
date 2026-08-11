# UHCI → PIC IRQ wire — Milestone 2 Round 14

## Why

R12 latched `USBSTS` / gated host IRQ via `uhci_interrupt_pending` but left DualPic / PIRQ unwired. Firmware expects the PIIX3 UHCI function to assert a legacy ISA IRQ through PIRQRC.

## Spec

- Intel 82371SB (PIIX3) — USB Host Controller interrupt hardwired to **PIRQD#**
- Intel 82371SB PIRQRC[D] (ISA config `0x63`) — bit7 clear enables; bits3:0 select ISA IRQ (`0xB` → IRQ11)
- UHCI 1.1 §2.1.2 / §2.1.3 — `uhci_interrupt_pending` (USBSTS ∩ USBINTR)

## Model

| Piece | Behavior |
|---|---|
| `UHCI_PIIX_PIRQD` | Constant `3` (PIRQD) |
| `UHCI_CLASSIC_ISA_IRQ` | Machine-model classic route **IRQ11** |
| `UHCI_CLASSIC_PIRQRC_D` | `0x0B` (enable + IRQ11) |
| `uhci_wire::sync_uhci_irq_to_pic` | `set_pirq_line(PIRQD, pending)` + `sync_pirq_to_pic` |
| `Machine::sync_uhci_irq_to_pic` | Thin wrapper |
| `MachineBus::poll_external_irq` | Syncs UHCI→PIRQD **after** fixed ISA sinks |

Does **not** rewrite `pci.rs` — uses existing `set_pirq_line` / `sync_pirq_to_pic`.

## Not wired (explicit)

- Automatic PCI Interrupt Pin config rewrite for the USB function
- Shared-IRQ storm with other PIRQD devices
- MSI for UHCI (legacy PIRQ only)

## Tests

- `uhci_pending_routes_pirqd_to_classic_irq11`
- `uhci_pic_irq11_via_poll_external_irq`
- `uhci_pending_pirqrc_disabled_does_not_raise_pic`
