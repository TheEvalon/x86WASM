# PIC IRR/ISR / OCW polish for `wait_irq` — Milestone 2 Round 15

## Why

SeaBIOS late-POST yields (`wait_irq` at `F000:C897`) and `check_irqs` /
`clock_poll_irq` touch the 8259A around IRQ0/IRQ8: non-specific EOI on `0x20`
and occasional OCW3 IRR/ISR reads. POST-with-media still samples `C897` mid-yield
(`docs/boot-r14-post-with-media.md`); this slice locks OCW3 honesty for those
paths without claiming a C897 exit.

## Spec

- Intel 8259A datasheet — OCW3 `RR`/`RIS` sticky read-register select; OCW2
  non-specific EOI clears the highest-priority ISR bit and does **not** reset
  the OCW3 select
- Classic AT cascade — IRQ8 → slave IR0 → master IR2; EOI slave then master
- PIIX ELCR — IRQ0/IRQ8 reserved edge (held-high must not re-latch IRR)

## Model (R15 polish)

| Sequence | Expected |
|---|---|
| IRQ0 INTA → OCW3 `0x0B` | ISR bit0 set; IRR bit0 clear |
| OCW2 `0x20` EOI | ISR clear; sticky ISR select still returns ISR (=0) |
| OCW3 `0x0A` after EOI with IR0 held high | IRR bit0 clear (edge) |
| New IR0 edge | IRR bit0 set again under sticky IRR select |
| IRQ8 INTA | Master ISR IR2 + slave ISR IR0; OCW3 reads match snapshot |

## Tests

- `irq0_ocw3_irr_isr_sticky_around_eoi_for_wait_irq`
- `irq8_ocw3_cascade_irr_isr_view_for_wait_irq`

## Unsupported / honesty

- Host-visible INT-raise vs INTA race beyond the existing pin-low-at-ack model
- Claiming POST past `F000:C897` (boot lane owns full with-media diagnosis)

## See also

- `docs/platform-r15-pit-irq0-wait-irq.md`
- `docs/platform-r15-rtc-irq8.md`
- `docs/platform-r15-post-remeasure.md`
