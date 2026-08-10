# Local APIC TMR (Trigger Mode Register) — Milestone 2 Round 10

## Why

R8 exposed IRR/ISR bitmaps for EOI honesty, but firmware that probes TMR still
saw zeros. Level vs edge Fixed accepts need a readable TMR stub matching the
SDM layout so SeaBIOS / early OS probes do not invent APIC behavior.

## Spec

- Intel SDM Vol. 3A Chapter 10
  - §10.8.6 — Trigger Mode Register (TMR) at `180H`–`1F0H`
  - On acceptance into IRR: TMR bit set for level, cleared for edge
  - EOI does not modify TMR; software writes are ignored

## Model

`devices::LocalApicMmio`:

| Event | TMR bit for vector |
|---|---|
| `inject_fixed` / edge `inject_fixed_trigger(..., false)` | cleared |
| `inject_fixed_trigger(..., true)` (level) | set |
| Local timer fire (edge) | cleared |
| `take_interrupt` / EOI | unchanged |
| MMIO write to TMR window | claimed, ignored |

Dword reads of TMR offsets return the bitmap.

## Not wired (explicit)

- TPR / PPR priority masking (separate R10 slice)
- Automatic ExtINT EOI broadcast semantics beyond existing IOAPIC Remote IRR
- CPU IDT injection; CPUID leaf 1 EDX bit 9 (`APIC`) remains clear

## Tests

- `crates/devices/src/lapic.rs`
  - `tmr_tracks_edge_vs_level_accept_eoi_unchanged`
