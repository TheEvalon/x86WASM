# INT 16h AH=12h extended shift status — Milestone 2 Round 14 (platform-kbd)

## Why

Enhanced-keyboard guests (FreeDOS, modern DOS tools) call AH=12h for left/right
Ctrl/Alt and lock-key *pressed* state. AH=02h alone is insufficient.

## Spec

- Ralf Brown's Interrupt List — INT 16h AH=12h "GET EXTENDED SHIFT STATES"
- Return: `AL` = same as AH=02h (`40:17`); `AH` = Table 00588:
  - bit0 left Ctrl, bit1 left Alt, bit2 right Ctrl, bit3 right Alt
  - bit4 ScrollLock pressed, bit5 NumLock pressed, bit6 CapsLock pressed
  - bit7 SysReq pressed
- Note: `AH` is **not** a raw copy of `40:18` (RBIL / Tech Help). This stub
  synthesizes Table 00588 from `40:18` pressed bits + `40:96` right Ctrl/Alt.

## Model

| Piece | Behavior |
|---|---|
| `Machine::bda_kbd_flag2` / `set_bda_kbd_flag2` | R/W `0040:0018` |
| `Machine::int16_extended_shift_ah` | Table 00588 synthesis |
| `Machine::service_int16` AH=`12h` | `AX` = (`AH` synth, `AL`=`40:17`) |

## Unsupported

- SysReq / Pause full key protocol
- AH=09h "keyboard functionality" query
- Guest BIOS body

## Tests

- `crates/machine-pc/src/int16.rs` — `int16_ah12_extended_shift_status`,
  `int16_ah12_right_ctrl_via_e0`
