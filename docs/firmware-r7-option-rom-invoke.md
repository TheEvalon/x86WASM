# Option-ROM invoke (R7 display/boot)

Milestone 2, campaign R7, display/boot lane — slice 2.

## Goal

Give the host a bounded path to **map** a PC-compatible expansion ROM and
**far-call** its initialization entry at offset 3 — the call a classic BIOS
makes during option-ROM scan — without claiming SeaBIOS POST or INT 10h.

## API

| Helper | Role |
|--------|------|
| `firmware_interface::OPTION_ROM_ENTRY_OFFSET` | Constant `3` |
| `firmware_interface::option_rom_entry_cs_ip(base)` | `CS:IP = (base>>4):(base&0xF)+3` |
| `Machine::map_option_rom` / `map_vga_option_rom` | Validate + map (unchanged) |
| `Machine::invoke_option_rom_entry(base, resume_cs, resume_ip)` | Far-call mapped ROM |
| `Machine::map_and_invoke_option_rom` | Map then invoke |
| `Machine::map_and_invoke_vga_option_rom` | VGA base + resume `HLT` at `0x0500` |

Far-call model:

1. Re-validate the mapped image (`55 AA`, size, checksum).
2. Ensure `SS:SP` can hold four bytes (`0000:7C00` when `SP < 4`).
3. Push `resume_ip` then `resume_cs` (Intel far-CALL frame).
4. Set `CS:IP` to the entry.

A synthetic ROM with `RETF` at offset 3 returns to the resume point; the VGA
convenience plants `HLT` at physical `0x0500`.

## Spec refs

- PCI Firmware Specification / BIOS Boot Specification — PC-compatible
  expansion ROM header; initialization entry at offset 3; checksum over the
  declared size.
- Intel SDM Vol. 1 / Vol. 2 — real-mode far CALL / RETF stack frame.
- Classic PC memory map — video BIOS conventionally at `0xC0000`.

## SeaBIOS interaction gaps (explicit)

This R7 slice did **not**:

- Scan `0xC0000`–`0xDFFFF` on 2 KiB steps (BIOS discovery loop) — **added in R9**
  (`docs/firmware-r9-option-rom-post-scan.md`).
- Pass PCI BDF / location in `AX`/`BX`/`DX` as SeaBIOS/PCI firmware do.
- Parse the PnP expansion header at offset `0x1A`, BEV/BCV, or runtime size.
- Run SeaVGABIOS to completion or claim fonts / INT 10h / mode set.
- Replace SeaBIOS POST option-ROM dispatch.

Building `firmware/seavgabios/vgabios.bin` (slice 1) and invoking it here are
related bring-up steps; success of the far call alone is not a VGA BIOS claim.

## Files

- `crates/firmware-interface/src/lib.rs` — entry CS:IP helper + tests
- `crates/machine-pc/src/option_rom_invoke.rs` — Machine helpers + tests
- `docs/firmware-r7-option-rom-invoke.md` — this note
