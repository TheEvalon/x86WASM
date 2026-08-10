# SeaVGABIOS build path (R7 display/boot)

Milestone 2, campaign R7, display/boot lane — slice 1.

## Goal

Reproduce a pinned **SeaVGABIOS** VGA option ROM (`vgabios.bin`) under `firmware/seavgabios/` without vendoring LGPL sources into MIT/Apache crates.

## Pin and script (already landed)

| Item | Value |
|------|--------|
| Upstream | SeaBIOS `vgasrc/` (not a separate repo) |
| Mirror | https://gitlab.com/qemu-project/seabios.git |
| Ref / commit | `rel-1.16.3` / `a6ed6b701f0a57db0569ab98b0661c12a6ec3ff8` |
| Manifest | `firmware/manifests/seavgabios.json` |
| Build | `firmware/build-scripts/build-seavgabios.sh` |
| Header check | `firmware/build-scripts/check-option-rom.py` |
| License notice | `firmware/seavgabios/LICENSE.notice` |
| NOTICE | `third_party/NOTICE` (SeaVGABIOS section) |
| Licensing | `docs/licensing.md` (SeaVGABIOS section) |

Build config (honest for this emulator): `CONFIG_QEMU=y`, `CONFIG_VGA_STANDARD_VGA=y`, `CONFIG_VGA_VBE=n`, `CONFIG_VGA_PCI=n`.

## Windows in-session status (2026-08-10)

This R7 worktree session ran on **Windows 10** without a usable WSL bash (`wsl … bash` → no bash). Native Win32 lacks the i386/`-m32` GNU toolchain SeaVGABIOS expects. **The ROM was not built in-session.** Artifacts remain gitignored; no binary is committed.

R9 smoke: `firmware/build-scripts/smoke-seavgabios-linux.sh --preflight` validates
scripts/pin/header checker without a full build;
`--build` remains Linux/WSL-only (`docs/firmware-r9-seavgabios-linux-smoke.md`).

## Reproducible Linux / WSL2 steps

On Ubuntu 22.04+ (native or WSL2):

```bash
sudo apt-get update
sudo apt-get install -y build-essential gcc-multilib python3 git make

cd /path/to/x86WASM   # or this worktree
chmod +x firmware/build-scripts/build-seavgabios.sh
./firmware/build-scripts/build-seavgabios.sh
```

Expected outputs:

- `firmware/seavgabios/vgabios.bin` (gitignored)
- `firmware/seavgabios/manifest.json` (size + sha256 + pin)
- `firmware/seavgabios/COPYING.SeaVGABIOS` (upstream license text)
- refreshed `firmware/manifests/seavgabios.json`

Verify the option-ROM header without the full build:

```bash
python3 firmware/build-scripts/check-option-rom.py firmware/seavgabios/vgabios.bin
```

Optional: reuse a SeaBIOS checkout instead of cloning twice:

```bash
SEAVGABIOS_SRC_DIR=firmware/seabios/.src ./firmware/build-scripts/build-seavgabios.sh
```

## Policy reminders

- Sources stay under `firmware/seavgabios/.src/` (gitignored).
- Do **not** copy LGPL C into `crates/**`.
- Do **not** commit `vgabios.bin` until licensing review allows it.
- Mapping/executing the ROM is a separate host path (`Machine::map_vga_option_rom` / option-ROM invoke); building alone is not SeaBIOS POST or INT 10h.

## Spec / provenance refs

- SeaVGABIOS overview: https://www.seabios.org/SeaVGABIOS
- PCI Firmware / BIOS Boot Specification — PC-compatible expansion ROM header (`55 AA`, size, checksum, entry at offset 3)
- ADR-0004 firmware separation; `.cursor/rules/licensing.mdc`
