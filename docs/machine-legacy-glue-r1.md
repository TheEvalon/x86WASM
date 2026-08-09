# Machine glue notes — Milestone 2, Round 1 (legacy support devices + firmware wiring)

Spec citations for the machine/firmware wiring slices on `slice/r1-machine-glue`.
Each section records what the machine now supports and what it explicitly does
**not**. No emulator source was consulted; only the specifications listed here
and in `docs/sources.md`.

## 1. COM1/COM2 THRE → PIC IRQ4 / IRQ3

Specs

- National Semiconductor NS16550A UART — Interrupt Enable Register bit 1
  (ETBEI, transmitter-holding-register-empty interrupt enable) and Interrupt
  Identification Register interrupt ID `010b` (THRE), cleared by reading IIR or
  by writing THR.
- IBM PC/AT Technical Reference — ISA interrupt assignment: serial port 1
  (`0x3F8`–`0x3FF`) is IRQ4 on 8259A master IR4; serial port 2
  (`0x2F8`–`0x2FF`) is IRQ3 on master IR3.
- Intel 8259A datasheet — edge-triggered IR latching (ICW1.LTIM = 0, PIIX ELCR
  bit clear): IRR latches on the low→high transition and is cleared at INTA, so
  a held-high pin does not redeliver after EOI.
- Intel SDM Vol. 3 §6.8.1 — maskable interrupt delivery; vector = ICW2 base | IR.

Supported

- `MachineBus::poll_external_irq` drives master IR4 from `com1.irq_line()` and
  IR3 from `com2.irq_line()` (level follow) before acknowledge, alongside the
  existing PIT0/keyboard/FDC/CMOS/aux/IDE sources.
- `Machine::sync_com1_irq4` / `Machine::sync_com2_irq3` expose the same host-side
  level follow used by the other device sync helpers.
- A THRE interrupt with `IF=1` vectors through real-mode IVT entry `0x0C`
  (COM1) or `0x0B` (COM2) with the AT ICW2 base of `0x08`.

Not supported

- Received-data-available (IER bit 0 / IIR `100b`): the 16550 subset has no
  receive path, so `RBR` reads 0 and the line is never raised by RX.
- Receiver line status (IER bit 2 / IIR `110b`), modem status (IER bit 3 /
  IIR `000b`), FIFO control and FIFO-timeout interrupts, and MCR OUT2 gating of
  the ISA interrupt driver.
- IRQ4/IRQ3 sharing between COM1/COM3 and COM2/COM4 (no COM3/COM4 exist).
