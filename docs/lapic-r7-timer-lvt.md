# Local APIC timer + LVT/spurious stub — Milestone 2 Round 7

## Why

Round-6 claimed the Local APIC MMIO page (ID/Version only). Firmware and early
SMP/timer bring-up need programmable timer registers and software enable.

## Spec

- Intel SDM Vol. 3A Chapter 10
  - §10.9 — Spurious Interrupt Vector Register (`F0H`); bit 8 = APIC software enable
  - §10.5.1 — LVT Timer (`320H`): vector, mask (bit 16), periodic (bit 17)
  - §10.5.4 — Timer ICR (`380H`), CCR (`390H`), DCR (`3E0H`)
  - §10.8.5 — EOI (`B0H`)

## Model

`devices::LocalApicMmio`:

| Register | Behavior |
|---|---|
| SVR | Vector + software-enable store/readback; reset `0xFF` (enable clear) |
| LVT Timer | Vector / mask / periodic; reset masked |
| ICR write | Loads ICR and CCR; starts countdown |
| CCR | Counts down on host `tick_timer`; RO (writes ignored) |
| DCR | Bits 3/1/0 divide encodings per SDM |
| EOI | Clears stub in-service |

`take_interrupt()` moves a latched vector to in-service. `Machine::tick_lapic_timer`
is a thin host helper.

## Wired

- One-shot and periodic timer countdown → local vector latch
- Software-enable and LVT mask gating

## Not wired (explicit)

- Injection into the CPU interpreter / INTR pin / IDT delivery
- Full IRR/ISR bitmaps, TPR/PPR priority, ICR/IPI, LINT0/1, thermal, perfmon
- x2APIC MSR interface; CPUID `APIC` bit remains clear
- Auto-tick from the instruction step clock

## Tests

- `crates/devices/src/lapic.rs`
- `crates/machine-pc/tests/lapic_mmio.rs`
