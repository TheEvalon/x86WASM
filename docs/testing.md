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
cargo build -p emulator-web --target wasm32-unknown-unknown --release
cd web
npm ci
npm run build
```

The production web build runs the repository's `build:wasm` script before
`vite build`. No separate browser automation or decoder/native oracle command
is currently configured; add those gates here when they land.

## Milestone 2 integration coverage

- Protected-mode CPU tests cover 16-bit IDT interrupt/trap gates, error-code
  frames, NMI/IRQ ordering, same-CPL `IRET16`, direct far `JMP16`, GDT-backed
  segment loads, `MOV SS`/`POP SS` interrupt shadowing, and atomic failure paths.
- Device tests cover fw_cfg ID/RAM selectors and reset persistence, PIT refresh
  detect, 8237 software requests/status, VGA Color Select/P54S composition,
  bounded multi-PRD BMIDE reads, FDC DSR/DOR reset behavior, and UART THRE
  interrupt latching.
- CLI tests keep successful HELLO output stable and require deterministic CPU
  context plus the original error source on execution failure.

## Milestone 1 acceptance

- Custom ROM at the reset vector prints `HELLO FROM EMULATOR` via COM1 and port `0x402`.
- Native CLI and browser worker both surface that string.
- Every implemented opcode has at least one semantic or decode test.
