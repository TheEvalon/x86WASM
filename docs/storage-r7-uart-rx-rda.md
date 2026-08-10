# R7 UART RX + RDA IRQ

Milestone 2, round 7, storage/guest lane, slice 3.

## Scope

NS16550A receive path on COM1 (`0x3F8`) and COM2 (`0x2F8`):

| Piece | Behavior |
|---|---|
| `Serial16550::push_rx` | Host injects one RBR byte (no FIFO) |
| LSR bit0 (DR) | Set while RBR holds data; cleared on RBR read |
| IER bit0 (ERBFI) | Gates received-data-available interrupt |
| IIR `100b` (0x04) | RDA ID; priority above THRE `010b` |
| `irq_line` | RDA and/or THRE → MachineBus PIC IR4 / IR3 |
| `Machine::com1_push_rx` / `com2_push_rx` | Inject + sync PIC line |

## Spec

NS16550A Interrupt Enable / Identification / Line Status registers; IBM PC/AT
ISA COM1→IRQ4, COM2→IRQ3.

## Still unsupported

- Hardware RX FIFO / character timeout (`IIR 110b`)
- Line-status / modem-status interrupt sources
- Baud-rate timed shifting (host inject is instantaneous)
