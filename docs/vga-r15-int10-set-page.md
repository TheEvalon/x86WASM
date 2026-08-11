# Host INT 10h AH=05h select active display page (R15 display-fw)

Milestone 2, Round 15, display-fw lane — slice 1.

## Goal

Bounded host INT 10h AH=05h SELECT ACTIVE DISPLAY PAGE so guests that flip
text pages (and AH=0Fh BH) see coherent BDA + CRTC Start Address — still no
full multi-page regen API beyond viewport Start Address.

## Behavior

| Input | Stub behavior |
|-------|---------------|
| Text mode, AL = 0–7 | BDA `0040:0062` = AL; `0040:004E` = AL × page size; CRTC Start Address = page × (page_size/2) character cells; CRTC Cursor Location from that page's BDA cursor word |
| AL > 7 | No-op |
| Mode 13h / non-text | No-op |

AH=02h stores cursors for pages 0–7; CRTC sync only when BH equals the active
page. Page-scoped write/teletype/read services accept BH=0 or BH=active.

## Spec refs

- Ralf Brown's Interrupt List — INT 10h AH=05h "SELECT ACTIVE DISPLAY PAGE".
- FreeVGA CRT Controller — Start Address High/Low (`0x0C`/`0x0D`).
- Prior: `docs/vga-r14-text-font-crtc.md`, `docs/vga-r10-bda-video.md`.

## Still unsupported

- True absolute multi-page writes when BH ≠ active (BH=0 still means viewport)
- Soft-scroll-only page flip without Start Address
- Graphics pages

## Files

- `crates/machine-pc/src/int10.rs`
- `docs/vga-r15-int10-set-page.md` — this note
