# UHCI frame-list walk deepen — Milestone 2 Round 11

## Why

R8 executed a single TD from the current `FRNUM` slot (`docs/uhci-r8-one-td.md`).
Firmware and early host stacks advance through multiple 1 ms frames and may
chain queue heads horizontally. This slice deepens the schedule walker without
claiming a full UHCI bandwidth engine.

## Spec

- Universal Host Controller Interface (UHCI) Design Guide, Revision 1.1
  - §3.1 Frame List pointer / `FRNUM`
  - §3.2 Transfer Descriptor
  - §3.3 Queue Head (element link + horizontal link)

## Model

`devices::uhci::run_n_frames`:

1. Require Bus Master + USBCMD.RS
2. For up to `max_frames` (cap [`UHCI_MAX_FRAMES_WALK`]):
   - Read `FLBASEADD[(FRNUM & 0x3FF)]`
   - TD link → execute Active TD
   - QH → execute element TD; follow **at most one** horizontal QH hop
     ([`UHCI_MAX_QH_HORIZONTAL`]) and execute that QH's element TD
   - Advance `FRNUM = (FRNUM + 1) & 0x3FF`
3. Empty / inactive frames are scanned without error

`run_one_td` remains the single-slot helper (does not advance `FRNUM`).

## Not wired (explicit)

- Isochronous TDs and full bandwidth reclamation / best-effort scheduling
- Vertical multi-TD chains (TD link pointer still ignored after first TD)
- Horizontal QH depth > 1 → `QueueHeadHorizontalUnsupported`
- Automatic schedule advance from the machine step clock
- USBINT → PIC/IOAPIC routing

## Tests

- `crates/devices/src/uhci.rs`
  - `n_frames_walks_two_slots_and_advances_frnum`
  - `qh_horizontal_hop_executes_second_td`
  - `qh_horizontal_depth_two_unsupported`
