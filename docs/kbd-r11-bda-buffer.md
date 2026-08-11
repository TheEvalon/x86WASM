# IRQ1 → BDA keyboard buffer — Milestone 2 Round 11

## Why

Round 9 added a host INT 16h typeahead buffer and an 8042 scancode queue, but
left the classic BIOS Data Area keyboard ring empty. FreeDOS / BIOS keyboard
paths expect words at `0040:001E` with head/tail at `0040:001A` / `0040:001C`.

## Spec

- IBM PC/AT Technical Reference — BDA keyboard buffer:
  - `0040:001A` buffer head (offset within segment `40h`)
  - `0040:001C` buffer tail
  - `0040:001E`–`0040:003D` circular ring of 16 words (`AL=ASCII`, `AH=Set-1`)
  - empty when head == tail == `001Eh`; wrap at `003Eh`
- Ralf Brown's Interrupt List memory map `0040h`

## Model (R11)

| Helper | Behavior |
|---|---|
| `Machine::init_bda_kbd_buffer` | head=tail=`001Eh` |
| `Machine::bda_kbd_inject_key` | enqueue without 8042 |
| `Machine::bda_kbd_peek` / `dequeue` | INT 16h-shaped head access |
| `Machine::service_kbd_irq1_bda` | `IN 60h` keyboard OBF → enqueue make; drop Set-1 breaks |
| `Machine::kbd_inject_scancode_to_bda` | 8042 inject + drain to BDA |
| `Machine::service_int16` | host `int16_buf` first, then BDA ring fallback |

Ring holds **15** usable words (classic empty≠full via one wasted slot). ASCII
translation is a bounded unshifted US Set-1 make table (letters/digits/
space/Enter); other makes store ASCII `0`.

## Unsupported

- **No guest INT 09h BIOS body** — host helper only; IVT[9] unchanged.
- No typematic / full E0 extended-key composition into the ring (R14 tracks
  E0 for right Ctrl/Alt flags only — `docs/kbd-r14-8042-irq1-polish.md`).
- Host `int16_buf` and BDA remain distinct stores; INT 16h only falls back
  when the host buffer is empty.
- Shift/ctrl/alt/`40:17` tracking deepened in R14 (was absent here).

## Tests

- `crates/machine-pc/src/bda_kbd.rs` — init, enqueue, full ring, IRQ1 drain,
  break drop, combined inject helper, peek/dequeue.
- `crates/machine-pc/src/int16.rs` — AH=00/01 BDA fallback.
