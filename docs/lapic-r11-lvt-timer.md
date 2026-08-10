# LAPIC LVT Timer mode polish — Milestone 2 Round 11

## Why

R7 landed one-shot/periodic CCR countdown with mask and software-enable gates.
Firmware often rewrites LVT Timer mid-flight (vector, mask, mode). This slice
locks down readback and expiry edge cases without claiming TSC-deadline or
full LVT delivery-mode fields.

## Spec

- Intel SDM Vol. 3A §10.5.1 — LVT Timer (`320H`): vector (7:0), mask (16),
  timer mode (17); other bits not retained by this stub
- Intel SDM Vol. 3A §10.5.4 — ICR/CCR/DCR countdown
- CPUID leaf 1 EDX bit 9 (`APIC`) remains **clear** (honesty: MMIO presence ≠
  advertised local APIC feature)

## Model

| Edge case | Behavior |
|---|---|
| Write junk / TSC-deadline bit 18 | Dropped; readback = vector\|mask\|periodic only |
| Vector change before expiry | Fire uses vector at expiry time |
| Masked at expiry (one-shot) | CCR→0, no pending; later unmask does not fire |
| Masked at expiry (periodic) | CCR reloads from ICR, no pending |
| Mode one-shot→periodic mid-count | Next expiry reloads CCR (periodic) |

## Not wired (explicit)

- TSC-deadline mode, LINT0/1 / thermal / perfmon LVT entries
- Injection into the CPU interpreter / INTR pin
- CPUID `APIC` advertisement (stays clear by policy)

## Tests

- `crates/devices/src/lapic.rs`
  - `lvt_timer_readback_drops_reserved_bits`
  - `lvt_vector_sampled_at_fire_time`
  - `lvt_mask_at_expiry_and_unmask_no_retroactive_fire`
  - `lvt_mode_switch_oneshot_to_periodic_mid_count`
