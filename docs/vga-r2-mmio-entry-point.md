# VGA guest-facing display-memory MMIO entry point

Milestone 2, round 2. Companion to `docs/vga-plane-memory-model.md`, which
describes the plane decode and the Graphics Controller data path themselves.

## Problem

Round 1 built the plane decode (`plane_access`) and the Graphics Controller
read/write data path (`gc_read_u8` / `gc_write_u8`), but nothing connected them
to a guest. `MachineBus` routed the static `0xB8000`–`0xBFFFF` range to
`VgaText::read_u8` / `write_u8`, which address the legacy interleaved text
buffer directly. A guest therefore could not reach write modes, the latches,
Map Mask, or any window other than `0xB8000`.

Round 2 makes the *device* side of that seam complete and single-entry. Bus
routing lives in `machine-pc` and is changed separately.

## API

```rust
// Static: the widest range the subsystem can ever decode.
VgaText::aperture() -> (u64, u64)          // (0x000A_0000, 0x000C_0000)
VgaText::in_aperture(addr: u64) -> bool

// Dynamic: what is claimed with the current programming.
vga.mmio_claims(&self, addr: u64) -> bool
vga.mmio_uses_text_buffer(&self, addr: u64) -> bool

// The entry points.
vga.mmio_read_u8(&mut self, addr: u64) -> Option<u8>
vga.mmio_write_u8(&mut self, addr: u64, value: u8) -> bool
```

`mmio_read_u8` takes `&mut self` because a graphics read loads all four
Graphics Controller latches.

`None` / `false` mean "not claimed" — a bus must fall through to open bus or
RAM, exactly as it already does for the text path.

## Claimed ranges

The aperture is fixed at `0xA0000`–`0xBFFFF` (128 KiB). The *claimed*
sub-range moves at runtime with two registers:

| Gate | Register | Effect when not satisfied |
|---|---|---|
| RAM Enable | Miscellaneous Output (`0x3C2`) bit 1 | nothing in the aperture is claimed |
| Memory Map Select | Graphics Controller Miscellaneous (`0x06`) bits 3:2 | addresses outside the selected window are not claimed |

Memory Map Select windows (IBM PS/2 Video Subsystems Figure 2-75):

| Bits 3:2 | Window |
|---|---|
| `00` | `0xA0000`–`0xBFFFF` (128 KB) |
| `01` | `0xA0000`–`0xAFFFF` (64 KB) |
| `10` | `0xB0000`–`0xB7FFF` (32 KB) |
| `11` | `0xB8000`–`0xBFFFF` (32 KB) — mode-03h reset default |

Because the claim is programming-dependent, a bus should register the whole
aperture once and let the device answer per access rather than re-registering
ranges whenever Miscellaneous changes.

## Pipeline

For a claimed access, one call runs:

1. Miscellaneous Output RAM Enable gating.
2. Graphics Controller Miscellaneous Memory Map Select window decode; the
   per-map offset is relative to the *selected window base*.
3. Sequencer Memory Mode plane addressing — Chain 4 / odd-even / planar — plus
   Graphics Controller Miscellaneous Chain Odd/Even as a second odd/even
   source, Extended Memory map sizing, and Map Mask write enables.
4. The Graphics Controller data path: reads load all four latches then apply
   read mode 0 (Read Map Select, Chain-4 address map, or host odd/even read
   addressing) or read mode 1 (Color Compare / Color Don't Care); writes apply
   write modes 0–3 with Set/Reset, Enable Set/Reset, Data Rotate + Function
   Select, Bit Mask, and Map Mask.

## Text-buffer split (model artifact)

This model keeps **two** backing stores:

- `VgaText::mem` — the 32 KiB interleaved character/attribute text buffer that
  backs `char_at` / `attr_at` / `put_char`, the CRTC Start Address / Offset
  viewport helpers, and the HELLO ROM path.
- `VgaText::planes` — four 64 KiB maps behind the Graphics Controller.

The entry point routes an address to the text buffer when that address lies in
`0xB8000`–`0xBFFFF` *and* the selected window covers it; everything else goes
to plane memory. So with Memory Map Select `00` (128 KB) the low
`0xA0000`–`0xB7FFF` half reaches plane memory and the high half reaches the
text buffer.

Consequences, stated plainly:

- Text-mode behavior at `0xB8000` is byte-identical to the legacy path. A text
  access does **not** load the Graphics Controller latches and does not apply
  write modes, which is what the existing tests and the HELLO ROM depend on.
- Real hardware has one memory. Software that programs a graphics window at
  `0xB8000` (Memory Map Select `11` with graphics write modes) hits the text
  buffer in this model, not plane memory.
- Unifying the two stores waits for a character generator and a display fetch
  from plane memory, neither of which exists.

## Wiring notes for `machine-pc`

Route the full aperture and honor the `Option` / `bool`:

```rust
// read path
if let Some(byte) = self.vga.mmio_read_u8(effective) {
    return byte;
}
// write path
if self.vga.mmio_write_u8(effective, val) {
    return;
}
```

The A20 gate must be applied before the call, as it already is for the text
path. `read_u8` / `write_u8` remain for host/test callers that must not disturb
the latches.

## Still unsupported

- No display fetch from plane memory: no character generator, no planar
  serialization, no renderer.
- `MachineBus` still routes the static text range to `read_u8` / `write_u8`, so
  a guest cannot reach the graphics path until that wiring lands.
- Shift Register Interleave / Shift256, CRTC byte/word (`Count by Two`)
  compensation, and QEMU's alternative `addr >> 2` Chain-4 offset are not
  modeled.
