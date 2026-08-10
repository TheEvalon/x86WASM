# Round 6 — IA32_APIC_BASE MSR (`0x1B`)

## Scope

`RDMSR` / `WRMSR` for MSR `0x1B` only. CPU state store/readback; no Local APIC
MMIO enable side effect (device ownership is elsewhere).

## Reset value (documented choice)

| Field | Value | Rationale |
|---|---|---|
| Base `[35:12]` | `0xFEE0_0000` | Architectural default APIC physical base |
| BSP (bit 8) | `1` | Single-processor / BSP firmware expectation |
| EXTD (bit 10) | reserved `0` | x2APIC unsupported → `#GP(0)` if written `1` |
| EN (bit 11) | `0` | Prefer clear until LAPIC MMIO is fully real |

Reset constant: `IA32_APIC_BASE_RESET = 0xFEE0_0100` in `x86-core`.

Bit layout follows Intel SDM Vol. 3 §10.4.4 / x2APIC spec: **EN = bit 11**,
**EXTD = bit 10**.

## Writable mask

Software may write `EN | base[35:12]`. BSP is preserved from the prior value
(changing it → `#GP(0)`). EXTD and other reserved bits → `#GP(0)`.

## Honesty

- Other MSR addresses still `#GP(0)`.
- `CPUID.01H:EDX[9]` (`APIC`) stays clear — this MSR does not advertise a Local APIC.
- No hook into device MMIO decode in this slice.

## Spec

Intel SDM Vol. 2 "RDMSR"/"WRMSR"; Vol. 3 §10.4.4; Vol. 4 MSR `1Bh`.
