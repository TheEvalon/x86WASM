# UHCI PORTSC CCS/PED/PR stub — Milestone 2 Round 11

## Why

SeaBIOS / firmware USB probe reads PORTSC connect and reset bits before any
transfer schedule runs. R8 left PORTSC as a naive BAR0 store/readback in
`PciConfig`. This slice adds honest bit semantics in `uhci.rs` helpers without
rewriting `pci.rs`.

## Spec

- Universal Host Controller Interface (UHCI) Design Guide, Revision 1.1 §2.1.7
  Port Status and Control Register (PORTSC)
  - bit 0 CCS (RO) — Current Connect Status
  - bit 1 CSC (R/WC) — Connect Status Change
  - bit 2 PED — Port Enabled/Disabled
  - bit 3 PEDC (R/WC) — Port Enable/Disable Change
  - bit 8 LS — Low Speed Device Attached (RO)
  - bit 10 — reserved, always reads 1
  - bit 12 PR — Port Reset

## Model

| Helper | Behavior |
|---|---|
| `portsc_attach_device` | Host sets CCS (+ optional LS) and CSC |
| `portsc_detach_device` | Clears CCS/LS/PED/PR; latches CSC/PEDC |
| `portsc_write` | Guest write: R/WC CSC/PEDC; retain PED/PR; preserve RO CCS/LS |
| `portsc_read` | Returns value with bit 10 forced 1 |

Reset end (PR 1→0) while CCS is set auto-sets PED and PEDC (common firmware
connect-after-reset sequence).

BAR0 port I/O in `PciConfig` remains raw store/readback; hosts that need
accurate PORTSC RMW should call these helpers (or a future thin wire).

## Not wired (explicit)

- Overcurrent / suspend / resume-detect engines
- Automatic CSC on line-status chatter
- USBINT / port-change IRQ routing
- Rewriting `pci.rs` BAR decode to call `portsc_write` on every I/O store

## Tests

- `crates/devices/src/uhci.rs`
  - `portsc_attach_reset_enables_ped`
  - `portsc_detach_clears_ccs_and_ped`
