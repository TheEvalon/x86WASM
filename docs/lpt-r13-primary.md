# LPT1 primary deepen — Milestone 2 Round 13 (platform-io)

## Why

R6 claimed LPT1 `0x378`–`0x37A` for SeaBIOS POST probes. R13 deepens the
control register idle default so RMW of the printer control port starts from
the classic adapter state (`/INIT` inactive + Select).

## Spec

- IBM PC Technical Reference — Parallel Printer Adapter control port:
  - bit0 Strobe
  - bit1 Auto Line Feed
  - bit2 `/INIT` (**active low**)
  - bit3 Select Input
  - bit4 IRQ enable (IRQ7; **not delivered** in this stub)
- [OSDev Wiki — Parallel Port](https://wiki.osdev.org/Parallel_Port)

## Model

| Register | Behavior |
|---|---|
| Data `0x378` | store / readback |
| Status `0x379` | fixed `LPT_STATUS_NO_PRINTER` (`0xDF`); Busy# inactive |
| Control `0x37A` | store / readback; **reset default `0x0C`** (`LPT_CONTROL_DEFAULT`) |

Named bit constants: `LPT_CTRL_STROBE` / `AUTOLF` / `INIT_N` / `SELECT` /
`IRQ_ENABLE`.

## Unsupported

- IRQ7, ECP/EPP, DMA, bidirectional nibble mode, actual printer handshake.
