# UHCI IOC / USBSTS deepen — Milestone 2 Round 14

## Why

R12 modeled USBSTS R/WC and USBINTR gating (`docs/uhci-r12-usbsts-usbintr.md`). R14 deepens the IOC completion path: sticky `USBINT`, clear-on-write honesty, and USBINTR.IOC enable/disable without silently clearing status.

## Spec

- UHCI 1.1 §2.1.2 USBSTS — USBINT set on IOC completion; software clears by writing 1
- UHCI 1.1 §2.1.3 USBINTR — IOC enable gates host interrupt; disabled sources remain visible in USBSTS

## Model

| Piece | Behavior |
|---|---|
| IOC TD completion | Always latches `USBSTS.USBINT` (pollable) |
| `uhci_interrupt_pending` | `(USBINT ∧ USBINTR.IOC)` ∨ … |
| `usbsts_write_w1c` | Write-0 preserves; write-1 clears only written R/WC bits |
| USBINTR.IOC clear | Drops host pending; **USBINT stays set** until W1C |
| `latch_usb_interrupt` | Host helper to latch USBINT without a TD walk |

## Not wired (explicit)

- Short-packet SPI as a separate status bit
- Automatic HCPE from malformed schedules
- Real CRC/timeout engine

## Tests

- `usbsts_w1c_preserves_unwritten_bits`
- `usbintr_ioc_disable_drops_pending_without_clearing_usbint`
- `usbint_sticky_until_w1c_across_ioc_completions`
- `ioc_completion_raises_host_irq_when_usbintr_ioc_enabled`
