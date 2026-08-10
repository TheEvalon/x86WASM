# COM3/COM4 probe UART stubs — Milestone 2 Round 13 (platform-io)

## Why

R5/R11 POST measurements listed unclaimed IER probes at `0x3E9` / `0x2E9`.
Those addresses are COM3/COM4 `base+1`, not LPT. R6 left them open-bus on
purpose; R13 claims the classic UART windows so firmware sees a live 16550
register file instead of ISA `0xFF`.

## Spec

- NS16550A / classic PC COM map:
  - COM3 `0x3E8`–`0x3EF`
  - COM4 `0x2E8`–`0x2EF`
- Same THR/RBR/IER/IIR/LSR subset as COM1/COM2 (`devices::Serial16550`).

## Model

| Port | Device | IRQ |
|---|---|---|
| `0x3E8`–`0x3EF` | `Machine.com3` | **not wired** (shared IRQ4 deferred) |
| `0x2E8`–`0x2EF` | `Machine.com4` | **not wired** (shared IRQ3 deferred) |

IER reads `0` at reset; LSR reports THRE|TEMT. THR appends to a per-port sink
(`com3_text` / `com4_text`).

## Honesty vs open-bus

R13 chose **stub claim** over open-bus so measured POST probes stop appearing
as unclaimed. Shared-IRQ routing with COM1/COM2 is explicitly unsupported.

## Unsupported

- COM3→IRQ4 / COM4→IRQ3 sharing, FIFOs, baud-timed shifting, modem status IRQs.
