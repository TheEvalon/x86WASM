# LAPIC LDR/DFR sticky stub — Milestone 2 Round 15

## Why

Firmware probes Logical Destination (`D0H`) and Destination Format (`E0H`)
during APIC bring-up RMW. Presence stubs avoid open-bus zeros without claiming
logical-destination delivery or advertising `CPUID.APIC`.

## Spec

- Intel SDM Vol. 3A §10.6.2.2 — LDR / DFR
  - LDR bits 31:24 — Logical APIC ID
  - DFR bits 31:28 — model (`0xF` Flat, `0x0` Cluster); other bits read as 1
- CPUID leaf 1 EDX bit 9 (`APIC`) remains **clear**

## Model

| Register | Reset | Writable | Delivery |
|---|---|---|---|
| LDR `D0H` | `0` | bits 31:24 sticky | none (no logical match) |
| DFR `E0H` | `0xFFFF_FFFF` | bits 31:28 sticky; low bits forced 1 | none |

## Honesty

MMIO presence ≠ advertised local APIC. This stub does **not** set `CPUID.APIC`.
ICR logical destination mode still does not deliver (R12 policy unchanged).

## Not wired (explicit)

- Logical destination matching / lowest-priority arbitration
- x2APIC LDR MSR
- Cluster model delivery fabric

## Tests

- `ldr_dfr_store_readback_sticky`
