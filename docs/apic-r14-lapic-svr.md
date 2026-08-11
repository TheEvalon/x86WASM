# LAPIC SVR stub polish — Milestone 2 Round 14

## Why

Firmware probes the Spurious Interrupt Vector Register (`F0H`) during APIC bring-up. R14 documents sticky bit policy, drops unsupported EOI-Broadcast Suppression, and tests soft-enable gating honesty **without** advertising `CPUID.APIC`.

## Spec

- Intel SDM Vol. 3A §10.9 — Spurious Interrupt Vector Register
  - Bits 7:0 — spurious vector
  - Bit 8 — APIC Software Enable/Disable
  - Bit 9 — Focus Processor Checking (Pentium-era; presence/readback here)
  - Bit 12 — EOI-Broadcast Suppression (unsupported; writes dropped)
- CPUID leaf 1 EDX bit 9 (`APIC`) remains **clear**

## Model

| Piece | Behavior |
|---|---|
| Reset | `SVR = 0xFF` (vector `0xFF`, soft-enable clear) |
| Writable | Vector + soft-enable + Focus (`LAPIC_SVR_WRITABLE`) |
| EOI suppress | Dropped on write |
| Soft-enable clear | Timer fire and Fixed inject suppressed |
| `spurious_vector()` | Bits 7:0 helper for probes |

## Honesty

This stub does **not** set `CPUID.APIC`. MMIO presence ≠ advertised local APIC.

## Not wired (explicit)

- Focus Processor Checking delivery semantics
- EOI broadcast fabric / directed EOI
- CPU interpreter injection of the spurious vector
- x2APIC MSR SVR

## Tests

- `svr_store_readback_drops_eoi_suppress`
- `svr_soft_enable_gates_timer_and_inject`
