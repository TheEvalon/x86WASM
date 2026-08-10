# Local APIC MMIO identity stub — Milestone 2 Round 6

## Why

Post-R5 SeaBIOS POST measured unmapped MMIO reads at `0xFEE00000` (Local
APIC default page). Claiming the 4 KiB window stops those probes from
appearing as unmapped open-bus.

## Spec

- Intel SDM Vol. 3A Chapter 10 "Advanced Programmable Interrupt Controller"
  - §10.4.4 — default base `FEE0_0000H`, 4 KiB
  - §10.4.6 — Local APIC ID Register offset `20H` (APIC ID bits 31:24)
  - §10.4.8 — Local APIC Version Register offset `30H` (version bits 7:0;
    Max LVT Entry bits 23:16)

## Model

`devices::LocalApicMmio` on `Machine` / `MachineBus`:

| Offset | Register | Behavior |
|---|---|---|
| `0x20` | ID | bits 31:24 store/readback; reset `0` |
| `0x30` | Version | RO `0x0003_0014` (version `0x14`, Max LVT Entry `3`) |
| other | — | read `0`; writes accepted, no side effects |

## CPUID honesty

Leaf 1 `EDX` bit 9 (`APIC`) remains clear. This stub is presence-only and
must not be taken as an advertised, usable local APIC.

## Unsupported (explicit)

- LVT timer / LINT / thermal / perfmon delivery
- ICR / IPI, EOI side effects, SVR, LDR/DFR, ESR
- x2APIC MSR interface
- APIC base MSR (`IA32_APIC_BASE`) relocation

## Tests

- `crates/devices/src/lapic.rs` unit tests
- `crates/machine-pc/tests/lapic_mmio.rs` bus + probe claim
