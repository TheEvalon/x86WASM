# UHCI deeper QH horizontal walk — Milestone 2 Round 12

## Why

R11 walked one horizontal QH hop per frame (`docs/uhci-r11-frame-list-walk.md`).
Host stacks often chain several queue heads on the same 1 ms frame. This slice
raises the soft depth without claiming a full UHCI bandwidth / reclaim engine.

## Spec

- Universal Host Controller Interface (UHCI) Design Guide, Revision 1.1
  - §3.1 Frame List pointer / `FRNUM`
  - §3.2 Transfer Descriptor
  - §3.3 Queue Head (element link + horizontal link)

## Model

`UHCI_MAX_QH_HORIZONTAL = 4` (documented soft cap):

1. Frame-list link → TD: execute that Active TD (unchanged)
2. Frame-list link → QH: execute each QH's element TD while following horizontal
   QH links, up to **4 hops** after the starting QH
   (`UHCI_MAX_FRAME_TDS = 1 + UHCI_MAX_QH_HORIZONTAL`)
3. A 5th horizontal QH hop → `QueueHeadHorizontalUnsupported`

Isochronous honesty: a frame-list **TD** link still runs as an ordinary Active
TD (no separate iso engine). Bandwidth reclamation / iso schedule accounting
remain unsupported.

## Not wired (explicit)

- Horizontal depth > 4
- Vertical multi-TD chains (TD link ignored after first TD)
- Isochronous bandwidth reclamation / best-effort scheduling
- Automatic schedule advance from the machine step clock
- USBINT → PIC/IOAPIC routing (see `docs/uhci-r12-usbsts-usbintr.md`)

## Tests

- `crates/devices/src/uhci.rs`
  - `qh_horizontal_depth_four_executes_all_tds`
  - `qh_horizontal_depth_five_unsupported`
  - existing `qh_horizontal_hop_executes_second_td` (depth 1 still valid)
