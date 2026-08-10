# LPT2 secondary / LPT3 honesty — Milestone 2 Round 13 (platform-io)

## Why

LPT2 at `0x278`–`0x27A` mirrors LPT1 as an independent register file. Classic
LPT3 at `0x3BC` is **not** claimed — ISA open-bus honesty for that window.

## Spec

Same as `docs/lpt-r13-primary.md` / IBM PC Parallel Printer Adapter.

## Model

- `Machine.lpt2` is a separate `ParallelPort` (`LPT2_BASE`).
- Writes to LPT1 never affect LPT2 (and vice versa).
- `ParallelPort::is_lpt3_window` documents `0x3BC`–`0x3BE`; MachineBus leaves
  those ports as open-bus `0xFF`.

## Unsupported

- LPT3 device, IRQ7 on either port, ECP/EPP.
