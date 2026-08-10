# PCI Status error signaling honesty — Milestone 2 Round 8

## Why

Round 5 already returns all ones on Mechanism #1 Master-Abort and sets
**Received Master Abort** on the host-bridge Status. This slice closes the
remaining honesty gaps around **which** Status bits that path may set, and
makes **Signaled Target Abort** RW1C observable without inventing a data-path
target-abort engine.

## Spec

PCI Local Bus Specification Revision 3.0:

- §3.2.2.3.4 / footnote 15 — unclaimed config target → all ones / dropped write.
- §6.2.3 Status Register:
  - **Received Master Abort** (bit 13, RW1C) — set by the **master** when its
    transaction terminates with Master-Abort.
  - **Signaled Target Abort** (bit 11, RW1C) — set by the **target** that
    signaled Target-Abort.
  - **Received Target Abort** (bit 12, RW1C) — set by the master that observed
    Target-Abort.

A config Master-Abort therefore sets **RMA only** on the host bridge. It must
not set STA or RTA.

## Behaviour

| Event | Status effect |
|---|---|
| Config absent-target read/write (enable set) | Host bridge `RMA=1`; data all ones / write dropped |
| Config enable-clear open-bus I/O | No Status change |
| Guest write-1 to RW1C error bits | Clears those bits; hardwired CapList/FastB2B/DevSel remain |
| Host `PciConfig::latch_status_errors` | ORs a subset of `PCI_STATUS_RW1C_MASK` onto a **present** function for tests / future data-path hooks |

## Not implemented

- Real data-path parity / SERR / target-abort generation from IDE, UHCI, or
  memory masters.
- Received Master Abort from non-config bus masters.
- Reporting STA on a target that does not exist (absent still Master-Aborts).
