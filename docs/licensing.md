# Licensing

## Project code

Dual-licensed under **Apache-2.0 OR MIT** (`LICENSE-APACHE`, `LICENSE-MIT`).

## Firmware and third-party binaries

- Live under `firmware/` with build scripts and license notices.
- Record provenance in `docs/sources.md` and `third_party/NOTICE`.
- Do not vendor GPL/LGPL sources into MIT/Apache crates without an ADR.

### SeaBIOS

| Item | Value |
|------|--------|
| Component | Legacy BIOS ROM (`firmware/seabios/bios.bin`) |
| Upstream | [SeaBIOS](https://www.seabios.org/) |
| Source mirror | https://gitlab.com/qemu-project/seabios.git |
| Pinned release | `rel-1.16.3` (`a6ed6b701f0a57db0569ab98b0661c12a6ec3ff8`) — `firmware/manifests/seabios.json` |
| License | **LGPL-3.0-or-later** (upstream `COPYING`; copied to `firmware/seabios/COPYING.SeaBIOS` by the build script) |
| Build | `firmware/build-scripts/build-seabios.sh` (see `firmware/README.md`) |

Policy:

- Checkout sources only under `firmware/seabios/.src/` (gitignored).
- Never copy SeaBIOS C sources into `crates/**`.
- Emulator crates may load a **binary** ROM blob via `Machine::load_bios_rom`; that does not relicense the ROM or pull LGPL code into the crate tree.
- `bios.bin` remains gitignored until an explicit licensing review allows committing binaries.
- OVMF / EDK II is a later firmware path (separate license/provenance when added).

## Provenance rules

- Do **not** copy implementation code from v86, QEMU, Bochs, VirtualBox, DOSBox, or other emulators.
- Specs, manuals, and behavioral oracles are allowed; their source is not.
- When adding a dependency, prefer permissive licenses consistent with dual Apache/MIT.
