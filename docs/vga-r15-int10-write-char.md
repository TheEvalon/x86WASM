# Host INT 10h AH=09h/0Ah write-char polish (R15 display-fw)

Milestone 2, Round 15, display-fw lane — slice 3.

## Goal

Polish AH=09h/0Ah count/page/attribute edges FreeDOS-style guests use after
AH=05h page select — still text viewport helpers only.

## Polish

| Topic | Behavior |
|-------|----------|
| BH | Must be `0` or BDA active page (viewport = Start Address after AH=05h) |
| CX=`0` | No-op |
| AH=09h BL=`00h` | Valid attribute (black on black); written as-is |
| Wrap | Horizontal wrap within the 80×25 page; stops at page end |
| Cursor | Unchanged (RBIL) |
| Inactive BH | No-op (no absolute multi-page write) |

## Spec refs

- Ralf Brown's Interrupt List — INT 10h AH=09h / AH=0Ah.
- Prior: `docs/vga-r12-int10-write-char.md`, `docs/vga-r15-int10-set-page.md`.

## Still unsupported

- Absolute writes when BH ≠ active (and BH ≠ 0)
- Graphics write-char
- Scroll on page overflow (AH=09/0A stop at page end)

## Files

- `crates/machine-pc/src/int10.rs`
- `docs/vga-r15-int10-write-char.md` — this note
