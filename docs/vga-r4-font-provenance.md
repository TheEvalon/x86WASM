# The reset font question: why this device ships no character ROM

Milestone 2, round 4, slice 4. Round 3 recorded a real problem: the character
generator works, but **no font is installed at reset and no character ROM is
vendored**, so a freshly reset device renders no glyphs and the renderer is
useless without a guest VGA BIOS. This slice resolves that question.

## The decision

**This crate ships no font.** The "no font" state is instead made explicit in
the host API, and a host that has a font it is entitled to use can install one.

That is the outcome the licensing rules point to, and it is also the
architecturally correct one. Both reasons are below, because either alone would
be a weaker argument.

## Reason 1: licensing

`.cursor/rules/licensing.mdc` forbids vendoring GPL sources into the MIT/Apache
crates without an explicit license decision recorded in an ADR, and
`.cursor/rules/emulator-core.mdc` forbids copying implementation from other
emulator projects. An 8×16 IBM-compatible CP437 glyph set is *data*, but it is
data with a lineage, and every candidate this slice considered fails at least
one test:

| Candidate | Licence | Why it was not used |
|---|---|---|
| Glyphs read out of an IBM PC/AT, VGA adapter or vendor BIOS image | proprietary | Explicitly ruled out by the task and by `licensing.mdc`. Extracting the bitmaps does not launder them. |
| A glyph table copied out of v86, QEMU, Bochs, VirtualBox or DOSBox | varies | Ruled out by `emulator-core.mdc` regardless of the licence text. |
| `u_vga16` / Uni-VGA | GPL-2.0 | Copyleft into MIT/Apache crates. `licensing.mdc` requires an ADR decision first, and `docs/adr/**` is outside this slice's ownership. |
| The Ultimate Oldschool PC Font Pack (VileR) | CC BY-SA 4.0 | The licence covers VileR's reproduction work; the underlying bitmaps are reproductions of IBM ROM glyphs. Share-alike also propagates into a code repository in a way that needs an ADR, not a slice. |
| X11 `misc-fixed` / `vga` BDF fonts | X11-ish | Plausible, but the specific 8×16 CP437 file, its exact provenance chain, and its notice text could not be verified from within this session. Vendoring bytes whose provenance is asserted rather than checked is the failure mode `docs/sources.md` exists to prevent. |
| Drawing 256 glyphs by hand | clean | 4,096 bytes of bitmap invented from memory of what CP437 looks like is exactly the "never invent behavior from memory" failure, and it would be an unreviewable blob. |

Nothing here is a *permanent* obstacle — a properly licensed font with a
recorded `docs/sources.md` entry, a `third_party/NOTICE` addition and (for a
copyleft or share-alike licence) an ADR is a perfectly reasonable future slice.
It is simply not something to assert on the way past. Per the task's own
instruction, **"no font, clearly reported" is the correct and honest outcome
when clean provenance cannot be established.**

## Reason 2: a font in map 2 is guest state, not device state

Even with a clean font in hand, installing one at reset would model something
the hardware does not do.

Real VGA display memory holds whatever was in the DRAM at power-on. The glyphs
a PC displays come from the **video BIOS**, which copies them into map 2 during
`INT 10h` mode set; the adapter's character ROM is part of the option ROM, not
part of the CRTC/Sequencer/GC/ATC/DAC this device models. So:

- A device model that pre-loads map 2 is fabricating guest state. The very
  first BIOS mode set overwrites it, so the fabrication is visible only in the
  window before the guest does anything — precisely the window in which an
  honest emulator should be reporting "nothing has set up a display yet".
- The reset fill of 80×25 spaces with attribute `07h` in maps 0 and 1 is already
  flagged in `docs/vga-r3-character-generator.md` as a model choice rather than
  hardware. Adding a font would be a second, larger one, and unlike the blank
  fill it would change what a *correct* guest sees.
- This tree already has the right owner for a font lined up: the pinned
  SeaVGABIOS build recorded in `plan.md` §21 (round 3, item 15). When that ROM
  is built and executed, it loads the font the way a PC does, with the
  licensing recorded where firmware licensing belongs.

## What replaces the missing font: saying so

A blank alphanumeric frame is ambiguous — no glyph data, or no text on screen —
and round 3 left a front end no way to tell. It now has one:

```rust
VgaText::font_bank_is_blank(bank_offset) -> bool  // one 8 KiB bank of map 2
VgaText::text_font_installed()           -> bool  // the selected sets, per Char Map Select
VgaFrame::font_installed: Option<bool>            // on every frame
VgaText::install_font_bank(bank_offset, glyph_height, glyphs) -> bool
```

`VgaFrame::font_installed` is the unmistakable one, because a front end already
has the frame in hand:

| Value | Meaning |
|---|---|
| `Some(false)` | alphanumeric frame, **no font installed** — every cell is background, so the blank picture is not content and a front end should say so |
| `Some(true)` | alphanumeric frame with glyph data in a selected character set |
| `None` | graphics frame; the character generator took no part |

`text_font_installed` follows the actual selection rules rather than scanning
all of map 2: it checks the two banks Character Map Select and attribute bit 3
name (both collapsing to bank `0000h` when Sequencer Memory Mode Extended
Memory is clear), so a font sitting in an unselected bank correctly reads as
"not installed" — that is what the character generator would fetch.

`install_font_bank` writes 256 glyphs at 32-byte stride into a bank of map 2 and
zeroes the scan lines past the font height, validating the bank alignment, the
map size, the glyph height and the buffer length first. It exists so the font
stays **the host's** to supply and to license: the emulator provides the
mechanism, the front end provides the data.

## Recorded consequences

- A freshly reset device still renders no glyphs. That is not a bug and is now
  reported rather than inferred.
- `docs/sources.md` needs **no** new entry for this slice: no font was adopted,
  so there is nothing to record. `third_party/NOTICE` needs no addition for the
  same reason.
- No ADR is required, because no licensing decision was taken — the decision
  was to take none.
- A front end that wants glyphs before firmware runs must call
  `install_font_bank` with a font it has the right to ship, and record that
  font in `docs/sources.md`, `docs/licensing.md` and `third_party/NOTICE` per
  `.cursor/rules/licensing.mdc`.
- `emulator-cli --vga-frame` does not yet surface `font_installed`; adding it is
  a one-line change in a crate this slice does not own.
