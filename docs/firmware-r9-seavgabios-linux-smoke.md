# SeaVGABIOS Linux/WSL smoke (R9 display-fw)

Milestone 2, Round 9, display-fw lane — slice 4.

## Goal

Document and script a **Linux/WSL smoke** for the pinned SeaVGABIOS build path.
Confirm Windows native remains infeasible (R7), keep LGPL sources out of
`crates/`, and give CI an optional `--build` hook.

## Smoke script

```bash
# Preflight only (works on Git Bash / WSL / Linux; no gcc/network required)
chmod +x firmware/build-scripts/smoke-seavgabios-linux.sh
./firmware/build-scripts/smoke-seavgabios-linux.sh --preflight

# Full build + option-ROM header check (Linux or WSL2)
./firmware/build-scripts/smoke-seavgabios-linux.sh --build
```

| Mode | Checks |
|------|--------|
| `--preflight` | Build script, `check-option-rom.py`, pin JSON, `LICENSE.notice`, docs present; validates a synthetic `55 AA` ROM |
| `--build` | Runs `build-seavgabios.sh`, then `check-option-rom.py` on `vgabios.bin` |

On non-Linux hosts, `--build` warns and exits 0 after preflight unless
`SEAVGABIOS_SMOKE_REQUIRE_BUILD=1`.

## Windows status (unchanged from R7)

Native Win32 lacks a reliable i386/`-m32` GNU toolchain for SeaVGABIOS.
**Use WSL2/Ubuntu or Linux CI.** See `docs/firmware-r7-seavgabios-build.md`.

## Optional CI note

Path-filtered `ubuntu-latest` firmware jobs (already used for SeaBIOS) can add:

```yaml
- name: SeaVGABIOS Linux smoke
  run: |
    sudo apt-get install -y build-essential gcc-multilib python3 git make
    ./firmware/build-scripts/smoke-seavgabios-linux.sh --build
```

Do **not** commit `vgabios.bin` without licensing review. Artifact upload only.

## Licensing

- LGPL-3.0-or-later; dual copyright (Kevin O'Connor + LGPL VGABios developers Team)
- Sources under `firmware/seavgabios/.src/` only (gitignored)
- Notices: `third_party/NOTICE`, `docs/licensing.md`, `firmware/seavgabios/LICENSE.notice`
- **No GPL/LGPL C into `crates/**`**

## Spec / provenance

- SeaVGABIOS: https://www.seabios.org/SeaVGABIOS
- PCI Firmware / BIOS Boot Spec — option-ROM header (`55 AA`, size, checksum)
- ADR-0004 firmware separation; `.cursor/rules/licensing.mdc`
