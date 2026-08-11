# UHCI PORTSC reset / connect honesty — Milestone 2 Round 15

## Why

Firmware USB probes pulse Port Reset and expect connect/enable bits to track
honestly. R11 added CCS/PED/PR basics; R15 deepens the reset handshake so PED
cannot fake-enable while disconnected or while reset is asserted.

## Spec

- Universal Host Controller Interface (UHCI) Design Guide, Revision 1.1 §2.1.7
  Port Status and Control Register (PORTSC)
  - bit 0 CCS (RO)
  - bit 1 CSC (R/WC)
  - bit 2 PED
  - bit 3 PEDC (R/WC)
  - bit 12 PR — Port Reset

## Model (R15 deepen)

| Event | Behavior |
|---|---|
| PR 0→1 | Clears PED; latches PEDC if PED was set |
| While PR=1 | PED writes do not stick |
| CCS=0 | PED writes do not stick (connect-status honesty) |
| PR 1→0 + CCS=1 | Auto-sets PED + PEDC (unchanged from R11) |
| Host helper | `portsc_attach_and_reset` / `Machine::uhci_portsc_attach_and_reset` |

BAR0 raw store in `pci.rs` is unchanged (lane ownership); helpers own semantics.

## Not wired (explicit)

- Overcurrent / suspend / resume engines
- Port-change IRQ from CSC/PEDC alone
- Rewriting `pci.rs` BAR0 decode to call `portsc_write` on every I/O store

## Tests

- `portsc_pr_assert_clears_ped`
- `portsc_ped_ignored_while_disconnected`
- `portsc_ped_ignored_while_reset_active`
- `uhci_portsc_attach_and_reset_enables_ped` (machine-pc)
