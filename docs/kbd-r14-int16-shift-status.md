# INT 16h AH=02h shift status — Milestone 2 Round 14 (platform-kbd)

## Why

FreeDOS and DOS utilities poll shift/lock state via INT 16h AH=02h before
interactive keyboard paths. R9/R11 covered AH=00h/01h + BDA ring but left
`40:17` unread.

## Spec

- Ralf Brown's Interrupt List — INT 16h AH=02h "GET SHIFT FLAGS"
- Table 00582 / MEM `0040h:0017h`:
  - bit0 right Shift, bit1 left Shift, bit2 Ctrl, bit3 Alt
  - bit4 Scroll Lock, bit5 Num Lock, bit6 Caps Lock, bit7 Insert

## Model

| Piece | Behavior |
|---|---|
| `Machine::set_bda_kbd_flag1` / `bda_kbd_flag1` | host R/W of `0040:0017` |
| `Machine::service_int16` AH=`02h` | `AL` ← `40:17`; ZF cleared |
| IRQ1 modifier path | updates `40:17` so AH=02 reflects live state |

## Unsupported

- Guest IVT INT 16h body (host dispatch only)
- AH=02h does not invent flags from the host `int16_buf`
- Insert lock bit tracking (bit7) not driven by IRQ1 in this slice

## Tests

- `crates/machine-pc/src/int16.rs` — `int16_ah02_returns_bda_shift_flags`,
  `int16_ah02_after_modifier_inject`
