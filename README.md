# x86WASM

Browser-capable full-system x86/x64 PC emulator (Rust core → WebAssembly) aimed at eventually booting Windows 10 x64.

## Status

Milestone 0–1 bootstrap: repository, CPU laboratory, buses, decoder framework, minimal interpreter, and a serial HELLO ROM.

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
