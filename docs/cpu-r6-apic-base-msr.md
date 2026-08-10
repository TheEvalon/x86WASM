# Round 6 — IA32_APIC_BASE MSR (`0x1B`)

## Scope

`RDMSR` / `WRMSR` for MSR `0x1B` only. CPU state store/readback; no Local APIC
MMIO enable side effect (device ownership is elsewhere).

## Reset value (documented choice)

| Field | Value | Rationale |
|---|---|---|
| Base `[35:12]` | `0xFEE0_0000` | Architectural default APIC physical base |
| BSP (bit 8) | `1` | Single-processor / BSP firmware expectation |
| Enable (bit 10) | `0` | Prefer clear until LAPIC MMIO is fully real |
| x2APIC (bit 11) | reserved `0` | Unsupported → `#GP(0)` if written `1` |

Reset constant: `IA32_APIC_BASE_RESET = 0xFEE0_0100` in `x86-core`.

## Writable mask

`BSP | EN | base[35:12]`. Any other bit (including bit 11 and `[63:36]`) on
`WRMSR` raises `#GP(0)` with state unchanged.

## Honesty

- Other MSR addresses still `#GP(0)`.
- `CPUID.01H:EDX[9]` (`APIC`) stays clear — this MSR does not advertise a Local APIC.
- No hook into device MMIO decode in this slice.

## Spec

Intel SDM Vol. 2 "RDMSR"/"WRMSR"; Vol. 3 Local APIC / IA32_APIC_BASE; Vol. 4 MSR `1Bh`.
