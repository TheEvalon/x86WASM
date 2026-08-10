# UHCI USBSTS / USBINTR polish — Milestone 2 Round 12

## Why

R8/R11 latch `USBSTS.USBINT` on IOC completion but do not model error/resume
status bits, R/WC clear, or USBINTR enable gating. Firmware polls and clears
these registers during UHCI bring-up.

## Spec

- Universal Host Controller Interface (UHCI) Design Guide, Revision 1.1
  - §2.1.2 USBSTS — USBINT / USBERRINT / Resume Detect / HSE / HCPE / HCHalted;
    software clears interrupt/error bits by writing 1
  - §2.1.3 USBINTR — Timeout/CRC, Resume, IOC, Short Packet enables; disabled
    sources still appear in USBSTS for polling; HCPE is not maskable

## Model

| Piece | Behavior |
|---|---|
| `usbsts_read` | Stored R/WC bits + HCHalted overlay when USBCMD.RS clear |
| `usbsts_write_w1c` | Clears bits 0–4 written as 1; does not store HCHalted |
| `usbintr_read` / `usbintr_write` | Bits 3:0 retained; 15:4 hardwired 0 |
| `uhci_interrupt_pending` | `(USBINT∧IOC)` ∨ `(USBERRINT∧CRC)` ∨ `(RD∧Resume)` ∨ `HCPE` |
| `latch_usb_error` / `latch_resume_detect` | Host stubs (no CRC/resume wire) |
| BAR0 path | Minimal `PciConfig` decode uses the helpers above |

IOC completion still always latches `USBSTS.USBINT` (pollable); host IRQ is
gated by `USBINTR.IOC`.

## Not wired (explicit)

- DualPic / PIRQ routing of UHCI interrupts
- Real CRC/timeout / short-packet detection engine (SPI enable documented only)
- Host System Error from PCI parity
- Automatic HCPE generation from malformed schedules

## Tests

- `crates/devices/src/uhci.rs`
  - `usbsts_usbint_gated_by_usbintr_ioc`
  - `usbsts_usberrint_gated_by_crc_enable`
  - `usbsts_resume_detect_requires_gsuspend`
  - `usbsts_hcpe_unmaskable`
  - `usbsts_hchalted_when_rs_clear`
