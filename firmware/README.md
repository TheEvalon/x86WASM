# Firmware

Third-party firmware binaries live here, separate from the emulator core (see plan.md section 16.4 and ADR-0004).

## Layout

| Path | Place |
|------|--------|
| `seabios/bios.bin` | SeaBIOS ROM (build output; gitignored until licensing review) |
| `seabios/manifest.json` | Optional build/revision metadata |
| `seavgabios/vgabios.bin` | SeaVGABIOS / compatible VGA option ROM |
| `ovmf/OVMF_CODE.fd` | OVMF code firmware (later) |
| `ovmf/OVMF_VARS_TEMPLATE.fd` | OVMF vars template (later) |
| `build-scripts/` | Scripts to rebuild firmware from upstream |
| `manifests/` | Shared firmware manifests |

Do not commit binary blobs unless licensing review is complete. `*.bin` under SeaBIOS/SeaVGABIOS and `*.fd` under OVMF are gitignored.

**Licensing:** SeaBIOS is GPL. Keep binaries and build scripts under `firmware/` only — never vendor SeaBIOS sources into MIT/Apache crates (`firmware-interface`, `machine-pc`, …).

## ROM mapping (emulator)

`firmware_interface::prepare_bios_rom` / `Machine::load_bios_rom` / `Machine::with_bios_rom` place a BIOS image at the top of 4 GiB and alias the last ≤128 KiB below 1 MiB (`0xF0000` for a 64 KiB image). This is mapping only — SeaBIOS POST is not booted yet. Lab HELLO ROM still uses `load_rom` (high map only).

## When adding binaries

1. Record provenance and exact revision in `docs/sources.md`.
2. Preserve upstream license notices alongside the blobs (or under this tree).
3. Update `third_party/NOTICE`.
4. Follow `docs/licensing.md` — do not embed third-party firmware without documented review.
