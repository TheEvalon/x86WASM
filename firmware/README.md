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
| `seavgabios/vgabios.bin` | SeaVGABIOS VGA option ROM (build output; gitignored) |
| `seavgabios/manifest.json` | Per-build metadata written by the build script |
| `seavgabios/LICENSE.notice` | Short licensing pointer (committed) |
| `seavgabios/COPYING.SeaVGABIOS` | Upstream license text copied by the build script |
| `seavgabios/.src/` | Upstream SeaBIOS git checkout for the VGA ROM (gitignored) |
| `seavgabios/.build/` | Out-of-tree build directory for the VGA ROM (gitignored) |
| `manifests/seavgabios.json` | Pinned revision + build config (committed) |
| `ovmf/OVMF_CODE.fd` | OVMF code firmware (later; out of scope for this slice) |
| `ovmf/OVMF_VARS_TEMPLATE.fd` | OVMF vars template (later) |
| `build-scripts/build-seabios.sh` | Fetch + build SeaBIOS into `seabios/` |
| `build-scripts/build-seavgabios.sh` | Fetch + build SeaVGABIOS into `seavgabios/` |

Do not commit binary blobs unless licensing review is complete. `*.bin` under SeaBIOS/SeaVGABIOS and `*.fd` under OVMF are gitignored.

**Licensing:** SeaBIOS and SeaVGABIOS are LGPL-3.0-or-later. Keep sources, binaries, and build scripts under `firmware/` only — never vendor their sources into MIT/Apache crates (`firmware-interface`, `machine-pc`, …).

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

## Build SeaVGABIOS (reproducible)

SeaVGABIOS is the VGA BIOS option ROM built from the `vgasrc/` tree of the same
SeaBIOS repository — there is no separate upstream project. The pin is therefore
the same release: **`rel-1.16.3`** (`a6ed6b701f0a57db0569ab98b0661c12a6ec3ff8`) —
see `manifests/seavgabios.json`.

```bash
# Same toolchain as SeaBIOS (build-essential, gcc-multilib, python3, git, make)
chmod +x firmware/build-scripts/build-seavgabios.sh
./firmware/build-scripts/build-seavgabios.sh
```

Artifacts:

- `firmware/seavgabios/vgabios.bin`
- `firmware/seavgabios/manifest.json` (size + sha256 + build config)
- `firmware/seavgabios/COPYING.SeaVGABIOS`
- refreshed `firmware/manifests/seavgabios.json` pin record

### Build configuration and why

The script writes a Kconfig fragment and lets `make olddefconfig` fill the rest.
The chosen symbols were read from `vgasrc/Kconfig` at the pinned revision:

| Symbol | Value | Reason |
|---|---|---|
| `CONFIG_QEMU` | `y` | the "Build Target" choice `VGA_STANDARD_VGA` depends on it |
| `CONFIG_VGA_STANDARD_VGA` | `y` | "QEMU/Bochs Original IBM 256K VGA" — the plain 256 KB stdvga this emulator models |
| `CONFIG_VGA_VBE` | `n` | the emulator has no VBE; a ROM must not advertise one |
| `CONFIG_VGA_PCI` | `n` | the emulator's VGA is a legacy port-mapped device, not a PCI function, so no PCI ROM header |

`CONFIG_BUILD_VGABIOS` is not user-settable (`default !NO_VGABIOS`); selecting a
VGA hardware type enables it. The build output is `$(OUT)vgabios.bin`.

The script keeps its own checkout, build directory, and Kconfig file so it never
clobbers a `build-seabios.sh` build; set `SEAVGABIOS_SRC_DIR=firmware/seabios/.src`
to reuse that checkout instead of cloning twice.

Optional overrides:

| Env | Default |
|-----|---------|
| `SEAVGABIOS_REPO` | `https://gitlab.com/qemu-project/seabios.git` |
| `SEAVGABIOS_REF` | `rel-1.16.3` |
| `SEAVGABIOS_COMMIT` | `a6ed6b701f0a57db0569ab98b0661c12a6ec3ff8` |
| `SEAVGABIOS_SRC_DIR` | `firmware/seavgabios/.src` |
| `SEAVGABIOS_BUILD_DIR` | `firmware/seavgabios/.build` |
| `SEAVGABIOS_CONFIG` | `firmware/seavgabios/.config.seavgabios` |
| `PYTHON` | `python3` |
| `JOBS` | host CPU count |

Before installing, the script checks the image is a well-formed option ROM:
`55 AA` signature, non-zero 512-byte block count that fits the image, and a
whole-image checksum of zero mod 256.

**Windows:** the same constraints as SeaBIOS apply — use WSL2 or Linux CI. There
is no CI workflow for this ROM yet. R7 display/boot recorded reproducible
Linux/WSL2 steps and the Windows infeasibility note in
`docs/firmware-r7-seavgabios-build.md` (native Win32 / missing WSL bash cannot
run this script).

## ROM mapping (emulator)

`firmware_interface::prepare_bios_rom` / `Machine::load_bios_rom` / `Machine::with_bios_rom` place a BIOS image at the top of 4 GiB and alias the last ≤128 KiB below 1 MiB (`0xF0000` for a 64 KiB image). This is mapping only — **SeaBIOS POST is not booted yet**. Lab HELLO ROM still uses `load_rom` (high map only).

## When adding binaries

1. Record provenance and exact revision in `docs/sources.md` and `firmware/manifests/`.
2. Preserve upstream license notices alongside the blobs (or under this tree).
3. Update `third_party/NOTICE`.
4. Follow `docs/licensing.md` — do not embed third-party firmware without documented review.
