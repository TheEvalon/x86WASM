# UHCI one-TD schedule stub — Milestone 2 Round 8

## Why

R7 deferred the optional UHCI TD walker (`docs/uhci-r7-one-td-skipped.md`) to
avoid contested `pci.rs` edits. This slice lands a **bounded** schedule helper
in a dedicated module while leaving BAR/port decode in `PciConfig`.

## Spec

- Universal Host Controller Interface (UHCI) Design Guide, Revision 1.1
  - §2.1.1 USBCMD (Run/Stop bit 0)
  - §2.1.2 USBSTS (USBINT bit 0)
  - §3.1 Frame List pointer
  - §3.2 Transfer Descriptor (Link / Status / Token / Buffer)
  - Queue Head: one element-link hop to a TD only
- Intel 82371SB (PIIX3) USB function — BAR0 32-byte I/O + Bus Master Enable

## Model

| Piece | Owner |
|---|---|
| PCI config + BAR0 decode + `uhci_io[32]` | `crates/devices/src/pci.rs` |
| One-TD schedule | `crates/devices/src/uhci.rs` (`run_one_td`) |
| Thin host entry | `PciConfig::run_one_uhci_td` |

`run_one_td` / `run_one_uhci_td`:

1. Require PCI Command.BusMaster and USBCMD.RS
2. Read frame-list dword at `FLBASEADD + (FRNUM & 0x3FF)×4`
3. Follow Terminate / TD / one QH→TD hop
4. If TD Active: IN copies `device_buf`→guest; OUT/SETUP copies guest→`device_buf`
5. Clear Active, write Actual Length (`n−1` encoding), latch USBSTS.USBINT when IOC

## Not wired (explicit)

- Multi-TD / multi-QH horizontal walks, bandwidth reclamation, isochronous
- Real USB device models, port connect/reset engines, IRQ routing from USBINT
- Automatic schedule advance from the machine step clock
- HCRESET side effects beyond existing register-file store/readback

## Tests

- `crates/devices/src/uhci.rs` (IN, OUT via QH, RS/BM gates, reset)
