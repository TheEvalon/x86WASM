# Option-ROM POST scan invoke (R9 display-fw)

Milestone 2, Round 9, display-fw lane — slice 3.

## Goal

Exercise the classic BIOS option-ROM **scan + far-call** path as a machine
helper: walk `0xC0000`–`0xDFFFF` on 2 KiB steps, validate `55 AA` + size +
checksum, and invoke each initialization entry (offset 3) with a host resume
frame. Builds on R7 `invoke_option_rom_entry`.

## API

| Helper | Role |
|--------|------|
| `Machine::scan_option_rom_region` | Discover valid ROMs already mapped |
| `OptionRomScanHit::{phys_base,blocks,next_scan_base}` | Hit + advance |
| `Machine::post_scan_invoke_option_roms` | Invoke each hit; step until resume |
| `Machine::post_scan_invoke_option_roms_default` | Resume `0000:0500` + `HLT` |

Scan advance: after a hit, next base = `phys_base + ceil(blocks*512 / 2KiB)*2KiB`.

## Spec refs

- PCI Firmware Specification / BIOS Boot Specification — PC-compatible
  expansion ROM header; init entry at offset 3; checksum over declared size.
- IBM PC memory map — option-ROM region `C0000`–`DFFFF`, 2 KiB scan step.
- Intel SDM — real-mode far CALL / RETF stack frame.

## Still unsupported

- PCI BDF / location registers passed in `AX`/`BX`/`DX` at call time
- PnP expansion header (`0x1A`), BEV/BCV, runtime-size shrink
- SeaBIOS POST ownership of the scan (this is a **host** mimic)
- Claiming SeaVGABIOS installs fonts or INT 10h

## Files

- `crates/machine-pc/src/option_rom_invoke.rs`
- `docs/firmware-r9-option-rom-post-scan.md` — this note
- Related: `docs/firmware-r7-option-rom-invoke.md`
