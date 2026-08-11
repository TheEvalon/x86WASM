# HPET legacy IRQ0/IRQ8 honesty — Milestone 2 Round 14

## Why

HPET 1.0a Legacy Replacement (`LEG_RT_CAP` / `LEG_RT_CNF`) maps Timer 0 → IRQ0 and Timer 1 → IRQ8. Silently claiming that while still driving PIT/CMOS on the same PIC lines would dual-fire. This slice locks an **explicit stub**: capability clear, config bit dropped, no DualPic claim.

## Spec

- IA-PC HPET Specification 1.0a §2.3.1 — Legacy Replacement Route
  - `LEG_RT_CAP` (CAPS bit 15)
  - `LEG_RT_CNF` (General Configuration bit 1)
  - When active: Timer 0 → IRQ0, Timer 1 → IRQ8

## Model (explicit stub)

| Piece | Behavior |
|---|---|
| `HPET_LEG_RT_CAP` | **0** (not advertised) |
| `HPET_CFG_LEG_RT` | Writes **dropped** (only `ENABLE_CNF` sticks) |
| `legacy_replacement_active()` | Always `false` |
| `drives_pic_irq0()` / `drives_pic_irq8()` | Always `false` |
| Timer 0 IRQ | Remains I/O APIC GSI path (default IRQ2) |
| PIT IRQ0 / CMOS IRQ8 | Unchanged platform owners |

Rationale: `NUM_TIM_CAP=0` (one timer) cannot honestly map Timer1→IRQ8.

## Honesty

- No MSI/FSB (`Tn_FSB_INT_DEL_CAP` clear — R12)
- No `CPUID.APIC` lie
- Prefer clear tests over silent PIT+HPET dual-fire on IRQ0

## Not wired (explicit)

- Real legacy replacement delivery onto DualPic
- Second comparator (Timer 1) for IRQ8
- ACPI HPET table / `_CRS` legacy routing

## Tests

- `legacy_replacement_cap_clear_and_leg_rt_cnf_dropped`
- `hpet_fire_does_not_drive_pic_irq0_or_irq8`
