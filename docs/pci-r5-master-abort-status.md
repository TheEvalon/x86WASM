# PCI Received Master Abort on config absent targets

Milestone 2 round 5 closes the honesty gap recorded in
`docs/pci-r3-config-access.md`: a Mechanism #1 cycle that Master-Aborts still
returns all ones / drops the write, and now also sets **Received Master Abort**
on the host-bridge Status register.

## Spec

PCI Local Bus Specification Revision 3.0:

- §3.2.2.3.4 / footnote 15 — unclaimed config target completes with all ones on
  read and dropped data on write.
- §6.2.3 Status Register — "Received Master Abort: This bit must be set by a
  master device whenever its transaction (except for Special Cycle) is
  terminated with Master-Abort."

The host bridge is the initiator of CONFIG_DATA cycles, so the bit is latched
in `00:00.0` Status (RW1C). Enable-bit-clear accesses remain ordinary open-bus
I/O and do **not** set the bit.

## Not implemented

- Received Master Abort from non-config bus masters.
- Signaled/Received Target Abort, parity, SERR latching from real data paths.
