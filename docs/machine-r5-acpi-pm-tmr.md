# ACPI PM_TMR freerun — Milestone 2 Round 5

## Why

After APM `0xB2`/`0xB3` unblocked SeaBIOS SMM bring-up, POST exhausted the
step budget spinning on:

```text
IN  port=0xB008 size=4 value=0x00000000   (repeat)
```

`0xB000` is the programmed PIIX ACPI PMBASE; `+08h` is `PM_TMR`. The PCI stub
only store/readbacks that dword, so the timer never moved and firmware delays
never completed.

## Spec

- Intel 82371AB (PIIX4) — Power Management Timer at PMBASE+`08h`: 24-bit
  free-running counter, frequency **3.579545 MHz** (14.31818 MHz / 4).
- That rate is exactly **three** IBM PC/AT 8254 input clocks (1.193182 MHz).

## Model

When [`StepClock`] is armed (including `--post-probe`), each retired
instruction that charges `pit_clocks` advances PIT time through
[`Machine::tick_pit`], which freeruns `PciConfig::acpi_pm_io[PM_TMR]` via
[`PciConfig::tick_acpi_pm`] at `pit_clocks × 3` under a 24-bit mask (and sets
`TMR_STS` on MSB toggle). Round 9 consolidates that PM advance **inside**
`tick_pit` so host PIT quanta freerun the counter too — see
`docs/platform-r9-pm-tmr.md`. There is one register authority — no second
freerun field.

## Unsupported

- No PM1 event/SCI, no TMR_STS wrap interrupt.
- No wall-clock accuracy; same instruction-count model choice as the PIT.
- Local APIC (`0xFEE00000`) and HPET (`0xFED00000`) remain unmapped MMIO.
- `RDTSC` is not advanced here (CPU ownership).
