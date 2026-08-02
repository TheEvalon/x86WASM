# Development cadence

Operational loop after the Milestone 1 HELLO ROM path.

## Milestone 2 tracker

Authoritative checkboxes: `plan.md` §21 Milestone 2. Summary:

- **Done so far:** early 16-bit real-mode opcode/interrupt foundation on `feat/real-mode-int-iret` (see plan progress list). Devices still COM1 + debug port `0x402` only.
- **Remaining for M2 exit:** complete real-mode gaps, protected mode + tables, exceptions/hardware interrupts, 32-bit paging, legacy devices (PIC/PIT/RTC/DMA/PS2/PCI/IDE/ATAPI/VGA), SeaBIOS + boot paths, FreeDOS/Linux serial shell.

## Per session

1. `next-slice` against `plan.md` §21 / §25
2. Plan Mode: files + acceptance tests only
3. Implement that slice only
4. `quality-gate` (`fmt`, `clippy -D warnings`, `cargo test --workspace`, Wasm when touched)
5. Branch → PR → `main`
6. cursor10x `storeMilestone`; jcodemunch `index_folder` when structure changes

## PR expectations

- One bounded slice per PR when practical
- Spec citations for instruction/device behavior
- Tests for new semantics before claiming done
- No opportunistic opcodes/devices outside the slice

## Re-index triggers

- New crate or public API surface
- Decoder metadata schema changes
- CPU state layout changes
