# Option-ROM / SeaVGABIOS honesty polish (R10 display-fw)

Milestone 2, Round 10, display-fw lane — slice 4.

## Goal

One bounded honesty improvement on the host option-ROM path:

1. **Bad-checksum `55 AA` headers are skipped** by the POST-style scan (already
   via `prepare_option_rom`; now covered by a dedicated test).
2. **Synthetic option-ROM `RETF` does not mutate host INT 10h BDA video fields
   or install a VGA font** — host INT 10h ≠ SeaVGABIOS / VGA BIOS.
3. **Windows native SeaVGABIOS remains infeasible** (reaffirmed; use WSL2/Linux).

## Tests

| Test | Asserts |
|------|---------|
| `post_scan_skips_bad_checksum` | `55 AA` + wrong checksum → not in scan hits |
| `option_rom_retf_preserves_bda_and_font` | After AH=00h + cursor + no font: map/invoke RETF ROM leaves BDA mode/cols/page size/cursor and `text_font_installed` unchanged |

## Windows / SeaVGABIOS

Native Win32 still lacks a reliable i386/`-m32` GNU toolchain for SeaVGABIOS
(`docs/firmware-r7-seavgabios-build.md`, `docs/firmware-r9-seavgabios-linux-smoke.md`).
Smoke path:

```bash
./firmware/build-scripts/smoke-seavgabios-linux.sh --preflight
# Full build: Linux or WSL2 only
./firmware/build-scripts/smoke-seavgabios-linux.sh --build
```

Do **not** vendor LGPL SeaVGABIOS sources into `crates/`.

## Spec refs

- PCI Firmware / BIOS Boot Spec — option-ROM signature, size, checksum
- RBIL INT 10h — host stub vs option-ROM-installed BIOS (this tree uses host)

## Still unsupported

- SeaVGABIOS completion, font load, or IVT\[10h\] install
- PCI BDF args / PnP BEV/BCV at option-ROM entry
- Windows native SeaVGABIOS build

## Files

- `crates/machine-pc/src/option_rom_invoke.rs`
- `docs/firmware-r10-option-rom-bda-honesty.md` — this note
- `firmware/build-scripts/smoke-seavgabios-linux.sh` — preflight checks R10 doc
