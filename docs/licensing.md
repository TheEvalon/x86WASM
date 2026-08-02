# Licensing

## Project code

Dual-licensed under **Apache-2.0 OR MIT** (`LICENSE-APACHE`, `LICENSE-MIT`).

## Firmware and third-party binaries

- Live under `firmware/` with build scripts and license notices.
- Record provenance in `docs/sources.md` and `third_party/NOTICE`.
- Do not vendor GPL sources into MIT/Apache crates without an ADR.

## Provenance rules

- Do **not** copy implementation code from v86, QEMU, Bochs, VirtualBox, DOSBox, or other emulators.
- Specs, manuals, and behavioral oracles are allowed; their source is not.
- When adding a dependency, prefer permissive licenses consistent with dual Apache/MIT.
