# HPET legacy IRQ0/IRQ8 honesty — Milestone 2 Round 14

## Spec

HPET 1.0a §2.3.1 — LEG_RT_CAP clear; LEG_RT_CNF writes dropped; no DualPic IRQ0/IRQ8 claim. PIT/CMOS remain owners.

## Tests

legacy_replacement_cap_clear_and_leg_rt_cnf_dropped; hpet_fire_does_not_drive_pic_irq0_or_irq8
