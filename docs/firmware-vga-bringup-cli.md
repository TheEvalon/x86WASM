# VGA / POST bring-up loop with `emulator-cli`

Milestone 2, round 2. What a developer can currently observe about the video
path from the native CLI, and what is still invisible.

## Flags

| Flag | Effect |
|---|---|
| `--post-probe` | POST first-contact report (unchanged format — other tooling parses it) |
| `--vga-text` | dump the 80×25 VGA text buffer after the run or probe |
| `--option-rom PATH` | map an option ROM image and report its header |
| `--option-rom-base ADDR` | base for `--option-rom`; decimal or `0x`, default `0xC0000` |

New diagnostics are printed **after** the existing run / `--post-probe` output,
in the fixed order `option-rom` then `vga-text`, and only when their flag is
given. Passing neither leaves stdout byte-identical to before.

## Typical loop

```bash
# 1. What stopped POST this round?
cargo run -p emulator-cli -- --bios firmware/seabios/bios.bin --post-probe

# 2. Did anything reach the screen before it stopped?
cargo run -p emulator-cli -- --bios firmware/seabios/bios.bin --post-probe --vga-text

# 3. Is the VGA BIOS build a well-formed option ROM, and does mapping it change
#    where POST stops?
./firmware/build-scripts/build-seavgabios.sh          # Linux / WSL2 only
cargo run -p emulator-cli -- --bios firmware/seabios/bios.bin \
    --option-rom firmware/seavgabios/vgabios.bin --post-probe --vga-text
```

## Output shapes

```text
option-rom: base=0x000C0000 size=32768 signature=55AA blocks=64 declared=32768 checksum=0x00 status=ok
```

`status=invalid` when the `55 AA` signature is missing, the 512-byte block
count is zero or claims more than the file holds, or the declared extent does
not checksum to zero mod 256 — the same rules as
`firmware/build-scripts/check-option-rom.py`. A malformed image is still
mapped, because inspecting a broken ROM is a legitimate bring-up step.

```text
vga-text: cols=80 rows=25 cursor=(0,0) start=0x0000 pitch=80 nonblank_rows=1
00 |POST OK.                                                                        |
01 |                                                                                |
...
```

Rows are bracketed with `|` so trailing spaces stay visible. `nonblank_rows` is
the quick "did anything get written" signal. The viewport follows the CRTC
Start Address and Offset the guest programmed, matching `VgaText::char_at`.

## What this does *not* show

- **Attributes and color.** Only characters are dumped.
- **CP437 glyphs.** Bytes outside printable ASCII (`0x20`–`0x7E`) render as `.`.
- **Graphics modes.** The dump reads the text buffer only. There is no renderer
  and no display fetch from plane memory, so a guest in a graphics mode shows
  nothing here.
- **Option ROM execution.** `--option-rom` maps an image. Nothing scans the
  `0xC0000`–`0xEFFFF` region for `55 AA` headers and nothing calls a ROM's init
  entry point, so mapping a VGA BIOS does not make INT 10h work.
- **Guest writes through the Graphics Controller.** `MachineBus` still routes
  CPU MMIO to the legacy text path; see `docs/vga-r2-mmio-entry-point.md`.
- **Timing.** `--post-probe` advances no time source, so firmware that polls
  the PIT or RTC exhausts the step budget instead of progressing.
