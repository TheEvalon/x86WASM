# Host INT 10h AH=0Eh teletype / attr polish (R15 display-fw)

Milestone 2, Round 15, display-fw lane — slice 3.

## Goal

Small deepen of AH=0Eh so scroll+attribute behavior stays coherent with R14
AH=06h/07h window scroll, and BH matches the active page after AH=05h.

## Polish

| Topic | Behavior |
|-------|----------|
| Scroll path | One-row teletype scroll calls the same cell-copy helper as AH=06h |
| Wrap fill attr | Written cell's attribute (R13) |
| LF fill attr | Default `07h` (R13) |
| CR / BS / BEL | CR → col 0; BS → col−1 no erase/no wrap; BEL → host counter only |
| BH | Accept `0` or BDA active page; other pages no-op |

Moved rows keep character+attribute pairs. Soft-scroll via CRTC Start Address
alone is still out (cell copies).

## Spec refs

- Ralf Brown's Interrupt List — INT 10h AH=0Eh "TELETYPE OUTPUT".
- Prior: `docs/vga-r13-int10-teletype.md`, `docs/vga-r14-int10-scroll.md`,
  `docs/vga-r15-int10-teletype.md` (CR/LF/BS/BEL edges).

## Still unsupported

- Graphics teletype / BL as graphics foreground
- Real PC-speaker / PIT channel 2 for BEL
- Soft-scroll replacing cell copies

## Files

- `crates/machine-pc/src/int10.rs`
- `crates/devices/src/vga.rs` — `note_host_tty_bell` / `host_tty_bell_count`
- `docs/vga-r15-int10-teletype-polish.md` — this note
