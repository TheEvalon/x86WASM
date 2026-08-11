# RTC IRQ8 / CMOS interrupt-enable honesty — Milestone 2 Round 15

## Why

SeaBIOS `clock_poll_irq` / `check_irqs` around `wait_irq` program Status B
PIE/AIE/UIE and clear Status C. POST-with-media still samples `F000:C897`
(`docs/boot-r14-post-with-media.md`); this slice makes IRQ8 follow guest CMOS
port I/O without requiring an extra RTC quantum after enabling PIE on a latched
PF.

## Spec

- Motorola MC146818A — Status B PIE/AIE/UIE; Status C PF/AF/UF/IRQF read-to-clear;
  IRQF = (PF∧PIE) ∨ (AF∧AIE) ∨ (UF∧UIE); IRQ pin follows IRQF
- IBM PC/AT — RTC IRQ → 8259A slave IR0 (ISA IRQ8), reserved edge via PIIX ELCR

## Model (R15)

| Path | Behavior |
|---|---|
| `CmosRtc` port `0x71` Status B write | Routes through `write_reg` (24/12 convert + `recompute_irqf`) |
| Enabling PIE with PF already set | IRQF/IRQ pin assert immediately |
| Clearing PIE | IRQF clears; PF remains until Status C read |
| `MachineBus` CMOS data R/W | `set_irq_line(8, irq_line())` so PIC sees rises/falls without waiting for `poll_external_irq` |

## Tests

- `port_write_pie_with_latched_pf_asserts_irq` / `port_write_clear_pie_deasserts_irqf` (devices)
- `cmos_port_enable_pie_with_pf_delivers_irq8` / `cmos_status_c_read_then_tick_redelivers_irq8` (machine-pc)

## Unsupported / honesty

- Host wall-clock / NTP sync; exact crystal UIP width
- Claiming C897-with-media exit (boot lane owns full diagnosis; IRQ8 is optional wake)

## See also

- `docs/platform-r15-pit-irq0-wait-irq.md`
- `docs/platform-r15-pic-irr-isr-ocw.md`
- `docs/platform-r15-post-remeasure.md`
