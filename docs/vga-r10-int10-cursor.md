# Host INT 10h AH=02h/03h cursor (R10 display-fw)

Milestone 2, Round 10, display-fw lane — slice 1.

## Goal

Extend the host INT 10h stub with cursor set/get against the BIOS Data Area.
Prefer BDA fields and existing VGA CRTC scanline helpers; do **not** claim full
hardware CRTC Location High/Low sync.

## API

| Service | Behavior |
|---------|----------|
| AH=02h | BH=page, DH=row, DL=col → write `0040:0050` (page 0 only; clamp to cols×25) |
| AH=03h | BH=page in; DH/DL = BDA cursor; CH/CL = BDA cursor type (`0040:0060`) |

Mode 03h via AH=00h also seeds `0040:0060` with classic `0607h` (start=6, end=7).

## Spec refs

- Ralf Brown's Interrupt List — INT 10h AH=02h "SET CURSOR POSITION", AH=03h
  "GET CURSOR POSITION AND SIZE"; BDA `0040:0050` / `0040:0060`.
- FreeVGA CRT Controller — Cursor Start/End (`0x0A`/`0x0B`) as fallback read
  helpers when BDA type is unread.

## Still unsupported

- Full CRTC Cursor Location High/Low (`0x0E`/`0x0F`) sync on AH=02h
- Multi-page cursor tables (`0040:0050`..`005F`)
- AH=01h SET CURSOR TYPE
- Guest LFB / VBE `4Fxx`

## Files

- `crates/machine-pc/src/int10.rs`
- `docs/vga-r10-int10-cursor.md` — this note
