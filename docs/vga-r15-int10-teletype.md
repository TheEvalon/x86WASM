# Host INT 10h AH=0Eh teletype edge deepen (R15 display-fw)

Milestone 2, Round 15, display-fw lane — slice 2.

## Goal

Deepen host INT 10h AH=0Eh teletype edge cases beyond R13 scroll/attr: CR/LF/BS
column rules and honest BEL without a speaker path. Active-page BH after AH=05h.

## Behavior

| Input | Behavior |
|-------|----------|
| Printable | Write at active-page cursor using cell attr (default `07h`); advance; wrap-scroll as R13 |
| `0Dh` CR | Column → 0; row unchanged |
| `0Ah` LF | Row + 1 (or scroll at bottom with fill `07h`); column unchanged |
| `08h` BS | Column − 1 if &gt; 0; **no erase**; **no wrap** to prior row |
| `07h` BEL | Increments `VgaText::host_tty_bell_count`; no PC-speaker path; cursor unchanged |
| BH | `0` or BDA active page; other pages no-op |

## Spec refs

- Ralf Brown's Interrupt List — INT 10h AH=0Eh "TELETYPE OUTPUT".
- Prior: `docs/vga-r13-int10-teletype.md`, `docs/vga-r14-int10-scroll.md`.

## Still unsupported

- Graphics teletype / BL as graphics foreground
- Real PC-speaker / PIT channel 2 drive for BEL
- Soft-scroll via CRTC Start Address instead of cell copy

## Files

- `crates/machine-pc/src/int10.rs`
- `crates/devices/src/vga.rs` — `note_host_tty_bell` / `host_tty_bell_count`
- `docs/vga-r15-int10-teletype.md` — this note
