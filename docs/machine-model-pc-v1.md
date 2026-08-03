# Machine model — PC v1

Classic PC subset for firmware and OS bring-up. See ADR `docs/adr/0001-machine-model.md`.

## Memory (Milestone 1 lab)

- Contiguous RAM from physical `0` (default size configurable; CLI default 16 MiB).
- ROM window mapped at `0xFFFF_0000` (64 KiB) so the Intel reset vector at `CS.base + 0xFFF0` = `0xFFFF_FFF0` fetches ROM.
- Optional alias of the same ROM image at `0x000F_0000` for real-mode `F000:xxxx` tooling later.

## Port I/O (M1 + M2 partial)

| Port | Device |
|---|---|
| `0x3F8`–`0x3FF` | COM1 (THR write emits guest serial bytes) |
| `0x402` | Debug console (Bochs/QEMU-style; write = one output byte) |
| `0x20` / `0x21` | 8259A master PIC (command / data) — **ICW1–ICW4 only** |
| `0xA0` / `0xA1` | 8259A slave PIC (command / data) — **ICW1–ICW4 only** |

Unimplemented ports: read `0xFF…`, write ignored (traced when tracing is enabled).

PIC unit model lives in `devices::DualPic` (`crates/devices/src/pic.rs`). It is **not** yet wired into `machine-pc` / `MachineBus`.

## MMIO

Stub dispatcher in M1; no VGA/IDE BARs yet.

## Interrupts

- Software INT/IRET and real-mode IVT fault delivery: see interpreter / M2 CPU work.
- Hardware IRQ path: CPU stub `pending_irq` / `Bus::poll_external_irq` exists; **no** PIC→CPU delivery yet.
- 8259A dual PIC: **ICW1–ICW4 initialization only** (vector base, cascade ICW3, ICW4 8086-mode bit). **Not yet:** OCW1–OCW3, EOI, IRR/ISR, priority, IRQ assertion, or `MachineBus` integration.
- APIC deferred to later milestones.

## Spec / oracle notes

- Serial: 16550-compatible programming model (subset).
- Debug port `0x402`: widely used by SeaBIOS/QEMU guests for early console; treat as write-only byte sink for M1.
- 8259A: Intel 8259A Programmable Interrupt Controller datasheet (ICW1–ICW4); classic PC cascade on IRQ2.
