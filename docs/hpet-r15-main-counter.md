# HPET main-counter freerun polish — Milestone 2 Round 15

## Why

Firmware timing probes that enable the HPET main counter previously saw a
frozen value unless the host called `advance_hpet` explicitly. R15 ties freerun
to the existing PIT tick / step-clock path while keeping R14 legacy-IRQ honesty.

## Spec

- IA-PC HPET Specification 1.0a — Main Counter Register (`F0h`), `ENABLE_CNF`
- R14: `LEG_RT_CAP` / `LEG_RT_CNF` stay clear (`docs/timer-r14-hpet-legacy.md`)

## Model

| Piece | Behavior |
|---|---|
| `HPET_TICKS_PER_PIT_CLOCK` | **12** (14.31818 MHz / 1.193182 MHz) |
| `Machine::tick_pit` | When `ENABLE_CNF` set, advances main counter by `clocks × 12` and syncs Timer 0 → I/O APIC |
| `ENABLE_CNF` clear | Counter halted (no freerun) |
| `LEG_RT_CAP` / `LEG_RT_CNF` | Still clear / dropped — PIT IRQ0 and CMOS IRQ8 remain owners |
| Explicit `advance_hpet` | Still available for tests |

Ratio is a **model choice**, not wall-clock accuracy (same class as step-clock).

## Honesty

- No MSI/FSB
- No DualPic legacy replacement
- No `CPUID.APIC` lie

## Not wired (explicit)

- ACPI HPET table period programming
- Host monotonic / TSC coupling
- Second comparator (Timer 1)

## Tests

- `hpet_ticks_per_pit_clock_is_twelve` (devices)
- `machine_tick_pit_freeruns_hpet_main_counter` (machine-pc)
- Existing `legacy_replacement_cap_clear_and_leg_rt_cnf_dropped` remains green
