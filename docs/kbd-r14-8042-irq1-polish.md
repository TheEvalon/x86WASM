# 8042 / IRQ1 shift-flag + ring-full polish — Milestone 2 Round 14 (platform-kbd)

## Why

AH=02h/12h only help guests if IRQ1→BDA actually maintains shift/lock state.
R11 drained make codes into the ring but ignored modifiers and drop-on-full
semantics beyond "return None".

## Spec

- IBM PC/AT INT 09h path — `IN 60h` clears OBF/IRQ1; update BDA flags; enqueue
  typeahead for non-modifier makes
- RBIL `40:17`/`40:18`/`40:96`/`40:97`
- OSDev PS/2 Keyboard — Set LEDs mask (Scroll/Num/Caps) stored on the keyboard
  stub when lock bits toggle

## Model (bounded deepen)

| Event | Behavior |
|---|---|
| `E0` prefix | set `40:96` bit0; no enqueue |
| Left/Right Shift, Ctrl, Alt make/break | update `40:17`/`40:18`/`40:96`; no enqueue |
| Caps/Num/Scroll make | toggle lock in `40:17`; set pressed in `40:18`; mirror `40:97` + `I8042::set_kbd_leds_host` |
| Ordinary make | enqueue ASCII/scancode |
| Ordinary break | drop |
| Ring full + make | **still** drain OBF; drop key (no beep); modifiers still update |

## Unsupported

- Full AT keyboard / typematic autorepeat into the ring
- Buffer-full beep via port 61h
- Guest INT 09h IVT body / EOI
- Authentic SysReq / Pause multi-byte sequences
- Insert lock toggle (non-E0 `0x52` is also keypad `0` under NumLock)
- PS/2 mouse / aux beyond existing IRQ12 stub

## Tests

- `crates/machine-pc/src/bda_kbd.rs` — shift, Caps+LED, ring-full, E0 right Ctrl,
  INT16 AH=00/01 coherence after modifiers
- `devices::I8042::set_kbd_leds_host` used by the LED mirror path
