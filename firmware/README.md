# Firmware

Third-party firmware binaries live here, separate from the emulator core (see plan.md section 16.4 and ADR-0004).

## Layout

| Path | Place |
|------|--------|
| `seabios/bios.bin` | SeaBIOS ROM (build output; gitignored until licensing review) |
| `seabios/manifest.json` | Per-build metadata written by the build script |
| `seabios/LICENSE.notice` | Short licensing pointer (committed) |
| `seabios/COPYING.SeaBIOS` | Upstream COPYING copied by the build script |
| `seabios/.src/` | Upstream SeaBIOS git checkout (gitignored; not a crate) |
| `manifests/seabios.json` | Pinned revision for reproducible builds (committed) |
| `seavgabios/vgabios.bin` | SeaVGABIOS / compatible VGA option ROM (later) |
| `ovmf/OVMF_CODE.fd` | OVMF code firmware (later; out of scope for this slice) |
| `ovmf/OVMF_VARS_TEMPLATE.fd` | OVMF vars template (later) |
| `build-scripts/build-seabios.sh` | Fetch + build SeaBIOS into `seabios/` |

Do not commit binary blobs unless licensing review is complete. `*.bin` under SeaBIOS/SeaVGABIOS and `*.fd` under OVMF are gitignored.

**Licensing:** SeaBIOS is LGPL-3.0-or-later. Keep sources, binaries, and build scripts under `firmware/` only — never vendor SeaBIOS sources into MIT/Apache crates (`firmware-interface`, `machine-pc`, …).

## Build SeaBIOS (reproducible)

Pinned release: **`rel-1.16.3`** (`a6ed6b701f0a57db0569ab98b0661c12a6ec3ff8`) — see `manifests/seabios.json`.

### Linux (preferred / CI)

```bash
# Debian/Ubuntu-style toolchain
sudo apt-get update
sudo apt-get install -y build-essential gcc-multilib python3 git make

# From the repository root
chmod +x firmware/build-scripts/build-seabios.sh
./firmware/build-scripts/build-seabios.sh
```

Artifacts:

- `firmware/seabios/bios.bin`
- `firmware/seabios/manifest.json` (size + sha256)
- `firmware/seabios/COPYING.SeaBIOS`
- refreshed `firmware/manifests/seabios.json` pin record

Optional overrides:

| Env | Default |
|-----|---------|
| `SEABIOS_REPO` | `https://gitlab.com/qemu-project/seabios.git` |
| `SEABIOS_REF` | `rel-1.16.3` |
| `SEABIOS_COMMIT` | `a6ed6b701f0a57db0569ab98b0661c12a6ec3ff8` |
| `SEABIOS_SRC_DIR` | `firmware/seabios/.src` |
| `PYTHON` | `python3` |
| `JOBS` | host CPU count |

### Windows

Native MSVC cannot build SeaBIOS. Use one of:

1. **WSL2 (Ubuntu)** — install the Linux packages above, then run the same script from the repo mounted in WSL.
2. **Git Bash** — only if you already have a working `gcc` that accepts `-m32` (or an i686 ELF cross compiler) plus `make` and `python3` on `PATH`. Most stock Git-for-Windows installs do **not**; prefer WSL2.
3. **Linux CI** — workflow [`.github/workflows/firmware-seabios.yml`](../.github/workflows/firmware-seabios.yml) runs on `ubuntu-latest` when `firmware/**` changes (or via `workflow_dispatch`). It installs `build-essential` / `gcc-multilib` / `python3`, runs the script, checks the pin commit, and uploads an ephemeral `bios.bin` artifact (7-day retention). Do not commit the binary without licensing review.

Manual steps (equivalent to the script):

```bash
git clone https://gitlab.com/qemu-project/seabios.git firmware/seabios/.src
git -C firmware/seabios/.src checkout --detach a6ed6b701f0a57db0569ab98b0661c12a6ec3ff8
make -C firmware/seabios/.src -j"$(nproc)" PYTHON=python3
cp firmware/seabios/.src/out/bios.bin firmware/seabios/bios.bin
```

Upstream overview: <https://www.seabios.org/> (build with standard GNU tools; default `make` produces `out/bios.bin` for QEMU-style use).

## ROM mapping (emulator)

`firmware_interface::prepare_bios_rom` / `Machine::load_bios_rom` / `Machine::with_bios_rom` place a BIOS image at the top of 4 GiB and alias the last ≤128 KiB below 1 MiB (`0xF0000` for a 64 KiB image). This is mapping only — **SeaBIOS POST is not booted yet**. Lab HELLO ROM still uses `load_rom` (high map only).

## When adding binaries

1. Record provenance and exact revision in `docs/sources.md` and `firmware/manifests/`.
2. Preserve upstream license notices alongside the blobs (or under this tree).
3. Update `third_party/NOTICE`.
4. Follow `docs/licensing.md` — do not embed third-party firmware without documented review.
