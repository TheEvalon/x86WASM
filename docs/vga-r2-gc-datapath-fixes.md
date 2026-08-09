# VGA Graphics Controller data-path corrections (M2 round 2)

Two gaps that round 1 recorded in `plan.md` §21 Milestone 2 are closed here.
Both are CPU-side data-path behavior; neither adds a renderer or a display
fetch from plane memory.

## 1. Write mode 3 applies Function Select

### Before

`gc_write_u8` computed write mode 3 as

```text
mask   = rotate_right(cpu_data, RotateCount) & BitMask
result = (expand(SetReset[plane]) & mask) | (Latch[plane] & !mask)
```

The Data Rotate register's Function Select field (bits 4:3) was read for write
modes 0 and 2 but skipped for mode 3.

### After

The expanded Set/Reset byte passes through the same ALU as modes 0 and 2 before
the synthesized mask selects between it and the latch:

```text
mask   = rotate_right(cpu_data, RotateCount) & BitMask
alu    = FunctionSelect(expand(SetReset[plane]), Latch[plane])
result = (alu & mask) | (Latch[plane] & !mask)
```

### Sources, including the conflict

The sources disagree, so the choice is recorded rather than assumed:

- **Michael Abrash, *Graphics Programming Black Book*, chapter 26 "VGA Write
  Mode 3"** — the `DrawChar` helper's header documents "Forces ALU function to
  'move'." and its Data Rotate setup comment reads "In write mode 3, this is
  the rotation of CPU data before it is ANDed with the Bit Mask register to
  form the bit mask. Force the ALU function to 'move'." Forcing the function is
  only necessary if the ALU stage is live in write mode 3.
- **IBM PS/2 Hardware Interface Technical Reference — Video Subsystems
  (42G2193) Figures 2-69 / 2-70 / 2-73** — one Function Select ALU sits between
  the Set/Reset multiplexer and the Bit Mask multiplexer in the Graphics
  Controller write path; the write mode selects the multiplexer inputs, not
  whether the ALU exists.
- **OSDev VGA Hardware, "Write mode 3"** — its step list goes straight from the
  expanded Set/Reset value to the bit-mask multiplexer with no ALU step. That
  page also carries a "Todo: more write modes, read modes" note, so its write
  mode 3 summary is treated as incomplete rather than contradictory.

This model follows Abrash. The reset default Function Select is replace/move
(`00`), so a driver that programs the register normally sees identical results
either way; the difference is only visible when a non-zero Function Select is
left in the Data Rotate register across a write-mode-3 write.

Write mode 1 (latch copy) still bypasses the ALU entirely, and write mode 3
still ignores Enable Set/Reset.

## 2. Graphics Mode bit 4 steers read-mode-0 map selection

### Before

Read mode 0 returned `Latch[ReadMapSelect]` (or the Chain-4 address map).
Graphics Mode bit 4 was stored and never consulted.

### After

When Chain 4 is not active and Graphics Mode bit 4 is set, the host address
bit A0 replaces bit 0 of Read Map Select:

```text
plane = (ReadMapSelect & !1) | A0
```

`A0` is taken from the host address relative to the selected Memory Map Select
window base, *not* from the decoded per-map offset, because odd/even addressing
has already cleared A0 there.

### Sources

- **FreeVGA Graphics Registers, Graphics Mode (index 05h) bit 4 "Host O/E —
  Host Odd/Even Memory Read Addressing Enable"**: "When set to 1, this bit
  selects the odd/even addressing mode used by the IBM Color/Graphics Monitor
  Adapter. Normally, the value here follows the value of Memory Mode register
  bit 2 in the sequencer."
- Sequencer Memory Mode bit 2 governs odd/even plane selection for host
  *writes*; Graphics Mode bit 4 is the *read*-side counterpart, which is why
  both exist. Odd/even addressing pairs maps 0+2 (even) against 1+3 (odd), so
  only bit 0 of Read Map Select is replaced — Read Map Select bit 1 still picks
  which pair.
- **IBM PS/2 Video Subsystems Figures 2-33 / 2-71 / 2-72** for the odd/even
  map pairing and Read Map Select.

### Behavior change

The mode-03h reset default for Graphics Mode is `0x10`, so this bit is **set**
after reset. Under that default a read-mode-0 access through the Graphics
Controller now returns the character map at even addresses and the attribute
map at odd addresses regardless of Read Map Select bit 0 — the CGA text
emulation this bit exists for. One round-1 unit test asserted the pre-fix
behavior and was updated.

Chain 4 still takes precedence (A1:A0 select the map), read mode 1 is
unaffected because the color compare consults every participating map, and a
read still loads all four latches.

## Still unsupported

- No display fetch from plane memory: no character generator, no planar
  serialization, no renderer.
- Shift Register Interleave and 256-Color Shift Mode have no effect.
- CRTC byte/word (`Count by Two`) compensation and QEMU's alternative
  `addr >> 2` Chain-4 offset are not modeled.
