# Keyboard buffer / host INT 16h — Milestone 2 Round 9

## Why

FreeDOS measure paths need a keystroke check/get without a full SeaBIOS
keyboard stack. The 8042 already had a one-byte OBF; burst injects overwritten
prior make codes. This slice adds a bounded scancode backlog and a host INT 16h
AH=00/01 stub.

## Spec

- IBM PC/AT 8042 — one-byte output buffer; device bytes wait until host `IN 60h`.
- Ralf Brown's Interrupt List — INT 16h:
  - **AH=00h** get keystroke → `AH`=Set-1 scancode, `AL`=ASCII; remove from buffer.
  - **AH=01h** check keystroke → ZF=1 if empty; ZF=0 and `AX` loaded if ready
    (key remains).

## Model

| Piece | Behavior |
|---|---|
| `I8042::inject_scancode` | If OBF free → present; else queue up to [`KBD_SCAN_QUEUE_CAP`] (16); drain on `0x60` read |
| `Machine::int16_push_key` | Host typeahead `(ascii, scancode)` up to [`INT16_BUFFER_CAP`] (16) |
| `Machine::service_int16` | AH=00/01 only; empty AH=00 sets ZF (no busy-wait) |
| `Machine::install_int16_ivt_pointer` | IVT far pointer only — no BIOS body |

## Unsupported

- No guest IVT INT 16h body / IRQ1 → BDA `40:1E` ring.
- No AH=02+ (shift status, services beyond get/check).
- No typematic autorepeat into the INT 16h buffer.
- Empty AH=00 does not spin — harness must push first.

## Tests

- `crates/devices/src/i8042.rs` — `inject_scancode_queues_behind_full_obf`.
- `crates/machine-pc/src/int16.rs` — AH=00/01, cap, IVT pointer, reset.
