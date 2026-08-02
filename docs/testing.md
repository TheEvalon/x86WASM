# Testing

## Hierarchy

1. **Unit tests** — flags, decode, device registers, bus bounds.
2. **Instruction semantic tests** — result, flags, exceptions, widths, memory forms, modes.
3. **ROM harness** — reset vector → serial/debug output comparison.
4. **Oracle / differential** (later) — XED decode, QEMU lockstep, interpreter↔JIT.
5. **Firmware / OS boot** (later) — SeaBIOS, FreeDOS, Linux, Windows.

## Quality gates

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
# Wasm target build when touching emulator-web / web/
```

## Milestone 1 acceptance

- Custom ROM at the reset vector prints `HELLO FROM EMULATOR` via COM1 and port `0x402`.
- Native CLI and browser worker both surface that string.
- Every implemented opcode has at least one semantic or decode test.
