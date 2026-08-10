# LAPIC ICR stub — Milestone 2 Round 12

## Why

Firmware and SMP bring-up code probe the Local APIC Interrupt Command
Register (`300H`/`310H`) even on single-CPU machines. R6–R11 claimed LAPIC
MMIO without an ICR. This slice adds presence/readback and a bounded Self
Fixed latch without advertising CPUID.APIC or multi-APIC delivery.

## Spec

- Intel SDM Vol. 3A §10.6 / §10.6.1 — Interrupt Command Register
  - ICR low @ `300H`: vector, delivery mode, dest mode, Delivery Status (RO),
    level/trigger, destination shorthand
  - ICR high @ `310H`: destination field bits 31:24
- CPUID leaf 1 EDX bit 9 (`APIC`) remains **clear** (MMIO presence ≠ feature)

## Model

| Piece | Behavior |
|---|---|
| ICR high | Store/readback destination bits 31:24 |
| ICR low | Store writable fields; Delivery Status always Idle (0) |
| Self / All-Including-Self + Fixed | Optional self-IPI: `inject_fixed(vector)` |
| No shorthand / All-Excluding-Self | Accept write; **no** delivery (single-CPU) |
| Non-Fixed modes (NMI/INIT/SIPI/…) | Presence/readback only; no latch |

## Not wired (explicit)

- Multi-APIC / logical destination delivery
- INIT / SIPI / NMI / SMI / ExtINT side effects
- x2APIC MSR ICR
- CPUID `APIC` advertisement (stays clear)
- CPU interpreter injection (hosts still `take_interrupt`)

## Tests

- `crates/devices/src/lapic.rs`
  - `icr_store_readback_drops_delivery_status`
  - `icr_self_fixed_latches_local_vector`
  - `icr_no_shorthand_and_all_excluding_self_no_delivery`
  - `icr_nmi_delivery_mode_no_latch`
