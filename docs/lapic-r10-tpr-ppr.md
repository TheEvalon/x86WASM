# Local APIC TPR / PPR stub — Milestone 2 Round 10

## Why

Firmware probes Task Priority (`80H`) and Processor Priority (`A0H`) during
APIC bring-up. R8 left those offsets as open-bus zeros. A minimal writable TPR
plus computed PPR lets probes succeed and documents how this stub inhibits
lower-priority pending Fixed interrupts without CPU IDT injection.

## Spec

- Intel SDM Vol. 3A Chapter 10
  - §10.8.3.1 — Task Priority Register (TPR) at `80H` (bits 7:0)
  - §10.8.3.2 — Processor Priority Register (PPR) at `A0H` (RO)
  - Interrupts are accepted only when their priority class is **strictly
    greater** than the current PPR class

## Model

`devices::LocalApicMmio`:

| Register | Behavior |
|---|---|
| TPR | Store/readback bits 7:0 |
| PPR | RO: equals TPR when no ISR or TPR class ≥ ISRV class; else `(ISRV>>4)<<4` |
| `interrupt_pending` / `take_interrupt` | Require pending vector class > PPR class; inhibited vectors stay latched in IRR/`pending_vector` |
| PPR MMIO write | Claimed, ignored |

## Honesty limits

- Single outstanding `pending_vector` (no multi-IRR arbitration beyond that)
- Sub-class (bits 3:0) stored in TPR and mirrored into PPR when PPR follows TPR,
  but acceptance uses **class** comparison only
- No CPU IDT injection; CPUID `APIC` bit remains clear

## Tests

- `crates/devices/src/lapic.rs` (`tpr_ppr_store_readback_and_masks_pending`)
