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

### SeaVGABIOS

| Item | Value |
|------|--------|
| Component | VGA BIOS option ROM (`firmware/seavgabios/vgabios.bin`) |
| Upstream | [SeaVGABIOS](https://www.seabios.org/SeaVGABIOS) — the `vgasrc/` tree of SeaBIOS, not a separate repository |
| Source mirror | https://gitlab.com/qemu-project/seabios.git |
| Pinned release | `rel-1.16.3` (`a6ed6b701f0a57db0569ab98b0661c12a6ec3ff8`) — `firmware/manifests/seavgabios.json` |
| License | **LGPL-3.0-or-later**, dual copyright |
| Build | `firmware/build-scripts/build-seavgabios.sh` (see `firmware/README.md`) |

The dual copyright matters and is easy to miss. Upstream `vgasrc/` headers carry
both "Copyright (C) 2009-2013 Kevin O'Connor" and "Copyright (C) 2001-2008 the
LGPL VGABios developers Team": **this ROM descends from the older LGPL VGABios
project as well as from SeaBIOS**, so a licensing review of SeaBIOS alone does
not cover it. Both attributions must survive into any distribution.

Policy is the same as SeaBIOS: sources only under `firmware/seavgabios/.src/`
(gitignored), never copied into `crates/**`, and the binary is gitignored until
an explicit review allows committing it.

**No `vgabios.bin` is committed.** R7 (2026-08-10) confirmed the Windows host
cannot build in-session; use Linux/WSL2 per `docs/firmware-r7-seavgabios-build.md`.
The pin and the notices remain the committed surface ahead of a CI/local Linux build.

## Provenance rules

- Do **not** copy implementation code from v86, QEMU, Bochs, VirtualBox, DOSBox, or other emulators.
- Specs, manuals, and behavioral oracles are allowed; their source is not.
- When adding a dependency, prefer permissive licenses consistent with dual Apache/MIT.
