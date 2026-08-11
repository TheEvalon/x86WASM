# BDA timer ticks (`40:6C`) from PIT IRQ0 — Milestone 2 Round 15

## Spec

RBIL MEM `0040:006C` / `0070`; IBM PC BIOS INT 08h wrap at `0x1800B0`.

## Model

`Machine::tick_pit` rising ch0 OUT → `advance_bda_timer_tick`.

## Honesty

Not a guest INT 08h body; does not claim POST past `F000:C897`.

## Tests

`crates/machine-pc/src/bda_timer.rs`
