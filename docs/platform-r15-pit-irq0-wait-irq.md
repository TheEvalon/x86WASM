# PIT IRQ0 / `wait_irq` honesty — Milestone 2 Round 15 (platform-post)

## Why

POST-with-media (R14) leaves the no-media `F000:9842` reboot class and stops at
`F000:C897` — SeaBIOS `wait_irq` (`sti; hlt; cli; cld; ret`) sampled mid-yield
(`docs/boot-r14-post-with-media.md`, `docs/post-c897-cf9-diagnosis.md`). Late
POST yields need **repeated** IRQ0 wakes under a mode-2/3 PIT, not a one-shot
mode-0 edge.

## Spec

- Intel 8254 — channel 0 mode 2 rate generator / mode 3 square wave OUT
- Intel 8259A — edge-triggered IR0 (PIIX ELCR reserves IRQ0 as edge)
- Intel SDM Vol. 2 HLT; Vol. 3A §6.8.1 (`IF=1` required to wake)
- SeaBIOS rel-1.16.3 `src/stacks.c` (`wait_irq`)

## Model (R15 deepen)

| Piece | Behavior |
|---|---|
| `Machine::tick_pit` | On ch0 rising OUT, pulse PIC IR0 low→high so a held-high mode-2/3 OUT still latches IRR after EOI |
| POST idle quantum | `POST_IDLE_TIMER_CLOCKS` advances PIT while `HLT`+`IF=1` |
| Guest shape | `sti; hlt; cli; cld` must wake more than once on mode-2 IRQ0 |

## Tests

- `pit_mode2_irq0_relatches_after_eoi_for_wait_irq_yields` — device/machine path
- `crates/machine-pc/tests/wait_irq_pit.rs` — two SeaBIOS-shaped yields → two IRQ0 wakes

## Unsupported / honesty

- Host-real-time PIT rate (instruction-count step clock is a model choice)
- Claiming POST-with-media past `F000:C897` (boot lane owns full C897 diagnosis)
- Guest SeaBIOS body; this lane only deepens IRQ0 device honesty

## See also

- `docs/platform-r15-pic-irr-isr-ocw.md`
- `docs/platform-r15-rtc-irq8.md`
- `docs/platform-r15-post-remeasure.md`
