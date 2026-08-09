# One VGA display memory

Milestone 2, round 3, slice 2. Retires the model artifact
`docs/vga-r2-mmio-entry-point.md` recorded under "Text-buffer split".

## What was there

Two backing stores:

- `VgaText::mem` — a 32 KiB interleaved character/attribute buffer that served
  every `0xB8000`–`0xBFFFF` access and backed `char_at` / `attr_at` /
  `put_char`, the CRTC viewport helpers and the HELLO ROM path.
- `VgaText::planes` — four 64 KiB maps behind the Graphics Controller.

A guest write at `0xB8000` therefore never reached plane memory, never loaded a
latch and never saw a write mode. Round 2 deferred unification until a
character generator existed, because until then nothing could show that the
maps held a screen.

## What is there now

One display memory: `VgaText::planes`. The `mem` field is gone.

| Caller | Path |
|---|---|
| Guest (`mmio_read_u8` / `mmio_write_u8`, and therefore `MachineBus`) | RAM Enable → Memory Map Select window → Sequencer plane addressing → the Graphics Controller data path, for **every** claimed address including `0xB8000` |
| Host (`read_u8` / `write_u8`) | the alphanumeric character/attribute interleave: even byte → map 0, odd byte → map 1, both at the same even offset relative to the selected window base. No read mode, no write mode, no Map Mask, no latch disturbance |
| Host (`char_at` / `attr_at` / `put_char`) | the CRTC address counter (`Start Address + row * pitch + col`) through `display_map_offset`, i.e. exactly what the character generator fetches |
| Renderer | the same maps |

## Why removing the split is safe under mode 03h

Under the reset programming the Graphics Controller resolves a text access to
the same map and offset the alphanumeric view uses, so the byte-for-byte
contract survives:

- **Write.** Sequencer Memory Mode default `0x02` selects odd/even, so an even
  address decodes to maps 0+2 and an odd address to maps 1+3 (IBM Figure 2-33),
  at the host offset with A0 cleared. Map Mask default `0x03` (Figure 2-29)
  narrows that to map 0 and map 1. Write mode 0 with Enable Set/Reset `0`,
  rotate count `0`, Function Select "replace" and Bit Mask `0xFF` stores the
  raw byte (Figure 2-73).
- **Read.** Read mode 0 with Graphics Mode bit 4 (Host Odd/Even read
  addressing, set by the mode-03h default `0x10`) substitutes host address bit
  A0 for bit 0 of Read Map Select, so even addresses return the character map
  and odd addresses the attribute map — the round-2 fix in
  `docs/vga-r2-gc-datapath-fixes.md` is what makes this line up.
- **Display.** The word-mode address multiplier from slice 1 puts CRTC counter
  *n* at map offset `2n`, which is where the CPU odd/even write to
  `0xB8000 + 2n` put the character.

`crates/devices/tests/vga_mmio_entry.rs::text_window_guest_and_host_paths_address_the_same_bytes`
walks all 32 KiB of the window and asserts this byte for byte.

## What genuinely differs — documented, not papered over

1. **A guest text read loads all four Graphics Controller latches.** It always
   should have; the separate buffer hid it. Real hardware loads the latches on
   every system read (OSDev VGA Hardware, "The Latches"). Host `read_u8` still
   does not, because a host/test caller must not perturb device state.
2. **Write modes, Map Mask and the Bit Mask now apply at `0xB8000`.** A guest
   that programs Map Mask `0x02` and writes an even text address no longer
   changes the character map. This is the hardware behavior; the old text
   buffer ignored all of it.
3. **The window base decides the offset.** With Memory Map Select `00`
   (`0xA0000`, 128 KB), `0xB8000` is display-memory offset `0x18000`, wrapped
   to `0x8000` in a 64 KiB map — not offset 0. The text buffer used to serve
   `0xB8000` from its own offset 0 whatever window was selected. Both the guest
   path and the host alphanumeric view now agree on `0x8000`, and the character
   generator, which follows the CRTC counter rather than a CPU address, is
   unaffected.
4. **`VgaText::mmio_uses_text_buffer` is gone.** It reported which backing
   store served an address; there is only one now.
5. **`VgaText::mem` is gone.** Nothing outside the device read it.

## Reset

`reset` still produces the 80×25 blank screen, but into display memory: map 0
gets `0x20` and map 1 gets `0x07` at even offsets `0`–`3998`, and everything
else — maps 2 and 3, the odd offsets, the rest of each map — is zero. That is
a **model choice, not hardware**: real display memory holds whatever was there
at power-on. It is kept because the HELLO ROM path, `emulator-cli --vga-text`
and a long tail of tests rely on a blank screen after reset.

The visible consequence is that maps 0 and 1 are no longer all-zero after
reset, which several existing tests asserted while the text fill lived
elsewhere. They now assert the fill.

Map 2 is **not** filled: no font is installed at reset.

## Still unsupported

- Everything in `docs/vga-r3-character-generator.md` "Still unsupported": no
  VBE, no host display, no timing-accurate raster, no planar graphics renderer.
- Shift Register Interleave / Shift256, the word-mode Address Wrap rotation,
  and QEMU's alternative `addr >> 2` Chain-4 offset remain unmodeled.
- `read_u8` / `write_u8` still only speak for `0xB8000`–`0xBFFFF`, so a host
  helper cannot address the low half of a 128 KB window; a host that wants
  arbitrary display memory uses `plane_byte` / `set_plane_byte`.
