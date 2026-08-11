# Option-ROM / SeaVGABIOS honesty polish (R10 display-fw)

Milestone 2, Round 10, display-fw lane — slice 4.

## Goal

Deepen the host option-ROM POST path without claiming SeaVGABIOS/Windows
native builds:

1. **Checksum gate before far-call** — scan skips bad checksum; `invoke_option_rom_entry`
   re-validates via `prepare_option_rom` and refuses a corrupted mapped image.
2. **Map-size honesty** — `check-option-rom.py` reports `declared_map` (header
   size×512) vs file length and any trailing unmapped bytes.
3. **BDA / font honesty** — synthetic `RETF` option ROM does **not** mutate host
   INT 10h BDA video fields or clear/replace fonts. R14 host AH=00h mode `03h`
   installs the bring-up font; RETF preserves that state (still not SeaVGABIOS).
4. **Windows native SeaVGABIOS remains infeasible** (reaffirmed; use WSL2/Linux).

## Tests

| Test | Asserts |
|------|---------|
| `post_scan_skips_bad_checksum` | `55 AA` + wrong checksum → not in scan hits |
| `invoke_rejects_bad_checksum_image` | Far-call path fails `BadChecksum` (RAM-visible image; ROM windows ignore writes) |
| `option_rom_retf_preserves_bda_and_font` | After AH=00h + cursor (font installed by R14 mode-set): RETF leaves BDA + `text_font_installed` unchanged |

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
Do **not** claim a guest LFB aperture (R9 VBE PhysBasePtr honesty stands).

## Spec refs

- PCI Firmware / BIOS Boot Spec — option-ROM signature, size, checksum; init at +3
- RBIL INT 10h — host stub vs option-ROM-installed BIOS (this tree uses host)

## Still unsupported

- SeaVGABIOS completion, font load, or IVT\[10h\] install
- PCI BDF args / PnP BEV/BCV at option-ROM entry
- Windows native SeaVGABIOS build
- Guest LFB / VBE `4Fxx` ModeAttributes LFB bit

## Files

- `crates/machine-pc/src/option_rom_invoke.rs`
- `firmware/build-scripts/check-option-rom.py` — declared map size reporting
- `firmware/build-scripts/smoke-seavgabios-linux.sh` — preflight checks R10 doc
- `docs/firmware-r10-option-rom-bda-honesty.md` — this note
