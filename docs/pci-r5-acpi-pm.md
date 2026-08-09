# PIIX4 ACPI PM1a / PM_TMR (bounded)

Milestone 2 round 5. Moves the PIIX ACPI PM I/O block at PMBASE past pure
store/readback far enough that firmware ACPI probes see a live timer and
power-button / SCI-enable stubs.

## Spec

- ACPI Specification — fixed hardware registers: `PM1a_EVT_BLK` (STS + EN),
  `PM1a_CNT_BLK`, `PM_TMR_BLK`; Power Management Timer frequency 3.579545 MHz;
  SCI generation from `(PM1_STS & PM1_EN)` while `SCI_EN` is set.
- Intel 82371AB (PIIX4) — ACPI function `8086:7113`, PMBASE at config `0x40`,
  64-byte I/O footprint.

## Behaviour

| Register | Offset | Behaviour |
|---|---|---|
| PM1_STS | +0 | Store/readback; host `acpi_assert_power_button` ORs PWRBTN_STS; timer MSB toggle ORs TMR_STS. Full write-1-to-clear deferred (MachineBus decode test still programs STS by store). |
| PM1_EN | +2 | R/W (TMR/GBL/PWRBTN enables in this stub) |
| PM1_CNT | +4 | Sticky SCI_EN / BM_RLD / SLP_TYP; SLP_EN write ignored (no sleep machine) |
| PM_TMR | +8 | 24-bit counter in `PciConfig::acpi_pm_io[8..12]`; guest loads accepted; advanced by `tick_acpi_pm` |

```rust
impl PciConfig {
    pub fn tick_acpi_pm(&mut self, ticks: u32);
    pub fn tick_acpi_pm_ns(&mut self, nanos: u64);
    pub fn acpi_pm_timer(&self) -> u32;
    pub fn acpi_assert_power_button(&mut self);
    pub fn acpi_sci_asserted(&self) -> bool; // level only — no PIC wire
    pub fn acpi_pm1_sts(&self) -> u16;
    pub fn acpi_pm1_en(&self) -> u16;
    pub fn acpi_pm1_cnt(&self) -> u16;
}
```

## Step-clock composition (integrator)

Register authority is **`acpi_pm_io[PM_TMR..]`** — there is no second timer field.
The machine R5 APM sibling advances PM_TMR from the instruction-count step clock
at **3 × PIT** (`ACPI_PM_CLOCKS_PER_PIT_CLOCK`) by writing those bytes under a
24-bit mask. That remains coherent with guest I/O reads.

Preferred long-term hook (also sets `TMR_STS` on MSB toggle) — **now the
integration path**:

```rust
// In Machine::advance_step_clock after PIT ticks:
pci.tick_acpi_pm((pit_clocks * 3) as u32);
```

Direct `acpi_pm_io` mutation remains coherent with guest I/O reads.

## Model choices

- Timer rate is the ACPI-specified 3.579545 MHz crystal; do **not** invent a
  second freerun clock — compose with the step clock (3 PM ticks per PIT).
- SCI is a polled level (`acpi_sci_asserted`); the machine layer may wire it to
  an IRQ later. No SMI path, GPE block, or sleep-state transition.

## Not implemented

- SCI delivery onto PIC/APIC, SMI#, GPE0/1, sleep states, ACPI tables / FADT.
