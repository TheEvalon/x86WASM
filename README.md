# x86WASM

Browser-capable full-system x86/x64 PC emulator (Rust core → WebAssembly) aimed at eventually booting Windows 10 x64.

## Status

Milestone 0–1 (HELLO ROM) complete on `main`. Milestone 2 in progress: early real-mode foundation on `feat/real-mode-int-iret` — see `plan.md` §21 checkboxes. SeaBIOS/FreeDOS/devices exit criteria not met.

CI: see GitHub Actions on `main`.

## Docs for contributors and agents

- [AGENTS.md](AGENTS.md) — session rules for Cursor agents
- [plan.md](plan.md) — product and engineering roadmap (source of truth)
- [docs/scope.md](docs/scope.md) — goals and non-goals summary

## Quick start (native)

```bash
cargo run -p emulator-cli -- --rom path/to/hello.bin
```

## Quick start (browser)

```bash
cd web
npm install
npm run build:wasm
npm run dev
```

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
