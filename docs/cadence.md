# Development cadence

Operational loop after the Milestone 1 HELLO ROM path.

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
