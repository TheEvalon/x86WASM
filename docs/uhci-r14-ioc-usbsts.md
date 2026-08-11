# UHCI IOC / USBSTS deepen — Milestone 2 Round 14

## Spec

UHCI 1.1 §2.1.2 / §2.1.3 — USBINT sticky until W1C; USBINTR.IOC gates host IRQ without clearing status.

## Model

usbsts_write_w1c write-0 preserves; latch_usb_interrupt host helper; IOC TD always latches USBINT.

## Tests

usbsts_w1c_preserves_unwritten_bits; usbintr_ioc_disable_drops_pending_without_clearing_usbint; usbint_sticky_until_w1c_across_ioc_completions; ioc_completion_raises_host_irq_when_usbintr_ioc_enabled
