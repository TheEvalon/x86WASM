# VGA plane (map) memory model

Scope note for the `devices::VgaText` VGA model. This file records the
specification text each behavior is derived from and the places where this
emulator makes an explicit model choice because the specification is silent.

## Approved sources used here

- IBM PS/2 Hardware Interface Technical Reference — Video Subsystems
  (form 42G2193, Sep 1992), chapter 2 "VGA Function":
  - Figure 2-29 Map Mask Register, index hex 02
  - Figure 2-33 Memory Mode Register, index hex 04
  - Figure 2-34 Map Selection, Chain 4
  - Figures 2-66 … 2-77 Graphics Controller registers, including
    Figure 2-73 Write Mode Definitions and Figure 2-75 Video Memory Assignments
- OSDev Wiki "VGA Hardware" — "Addressing Logic" (Chain 4 / Odd-Even offset
  forms observed on real hardware and emulators) and "Read/Write logic"
  (latch behavior, per-step write-mode algorithms)
- FreeVGA Sequencer / Graphics Registers (already listed in `docs/sources.md`)

`docs/sources.md` should gain the IBM PS/2 Video Subsystems reference; that file
is owned by the integration coordinator for this round.

## Address decode (Sequencer Memory Mode, index `0x04`)

| Programming | Maps selected | Map offset |
|---|---|---|
| Chain 4 (bit3 set) | `1 << (A[1:0])` — Figure 2-34 | host offset with A1:A0 cleared |
| Odd/Even (bit2 clear) | even → maps 0+2, odd → maps 1+3 — Figure 2-33 | host offset with A0 cleared |
| Planar (bit2 set, bit3 clear) | all four maps | host offset unchanged |

The host offset is measured from the base of the CPU display window.

The selected maps are then ANDed with the Map Mask register (Figure 2-29;
OSDev VGA Hardware, Write Mode 0: "The Memory Plane Write Enable field is ANDed
with the input from the address logic"). IBM notes that all maps should be
enabled while Chain 4 is selected; this model still applies the Map Mask, which
matches the QEMU / ATI / NVidia column of the OSDev comparison table.

Extended Memory (bit1) selects 64 KB or 256 KB of video memory. IBM documents
the size, not what happens to a host access beyond the 64 KB configuration, so
this model wraps the per-map offset within the enabled region
(16 KiB per map when clear, 64 KiB when set).

## CPU display window (Graphics Controller Miscellaneous, index `0x06`)

Memory Map Select (bits 3:2) chooses which host addresses the video subsystem
claims — IBM Figure 2-75 Video Memory Assignments:

| Field | Window |
|---|---|
| `00` | `0xA0000`–`0xBFFFF` (128 KB) |
| `01` | `0xA0000`–`0xAFFFF` (64 KB) |
| `10` | `0xB0000`–`0xB7FFF` (32 KB) |
| `11` | `0xB8000`–`0xBFFFF` (32 KB, mode-03h default) |

Host offsets used by the plane decode are relative to the base of the selected
window. Misc Output RAM Enable still gates every claim.

`VgaText::read_u8` / `write_u8` are backed only by the legacy 32 KiB text buffer
at `0xB8000`, so they claim the intersection of the selected window with that
range: selecting `10` or `01` makes `0xB8000` accesses fall through to the bus,
while addresses below `0xB8000` decode for the plane path but have no text
buffer behind them.

Chain Odd/Even (bit1, IBM Figure 2-74 OE) is an independent source of odd/even
host addressing: either it or Sequencer Memory Mode bit2 selects odd/even, and
Chain 4 still overrides both.

## Graphics Controller data path

`VgaText::gc_read_u8` / `VgaText::gc_write_u8` operate on the four maps in
`VgaText::planes` and on the four latches in `VgaText::gc_latches`.

Reads always load all four latches from the addressed map offset (OSDev VGA
Hardware, "The Latches"), then:

- read mode 0 returns the map named by Read Map Select (Figure 2-71), or the map
  named by A1:A0 when Chain 4 is set (Figure 2-72, RM description);
- read mode 1 returns the color compare of the maps selected by Color Don't Care
  (Figures 2-68 and 2-76).

Writes follow Figure 2-73:

| Write mode | Data source | Function Select | Bit Mask |
|---|---|---|---|
| 0 | rotated system data, or Set/Reset for maps enabled in Enable Set/Reset | applied | applied |
| 1 | the latches | not applied | not applied |
| 2 | data bit *n* expanded across map *n* | applied | applied |
| 3 | Set/Reset value (Enable Set/Reset ignored) | not applied | rotated data AND Bit Mask forms the mask |

The resulting bytes are stored only in maps selected by the address decode AND
the Map Mask.

Write mode 3 does not apply Function Select in this model. IBM Figure 2-73 says
the function select operation is "performed on system data for modes 0, 2, and
3", but in mode 3 the system data forms the bit mask rather than the written
value, and the step-by-step description in OSDev VGA Hardware "Write mode 3"
contains no ALU stage.

## Explicitly not modeled

- QEMU's alternative Chain-4 offset (`addr >> 2`); this model follows the
  officially documented / hardware-observed form.
- Chain-4 effects on the *display* fetch path (Bochs models one; real hardware
  per OSDev does not). There is no renderer in this model at all.
- Doubleword / word CRTC addressing (`Count by Two`, byte/word mode), which is
  what compensates for the odd/even offset form on the display side.
- Routing CPU MMIO through the Graphics Controller data path. `VgaText::read_u8`
  / `VgaText::write_u8`, which `MachineBus` calls, still address the legacy
  interleaved text buffer.
- Graphics Mode bit4 (host odd/even memory read addressing) as an input to read
  mode 0 map selection, plus Shift Register Interleave and 256-Color Shift Mode.
- Any display fetch from plane memory (character generator, planar pixel
  serialization, attribute path).
