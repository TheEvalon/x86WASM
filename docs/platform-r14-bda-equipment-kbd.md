# BDA equipment / keyboard flags for FreeDOS — Milestone 2 Round 14 (platform-kbd)

## Why

FreeDOS interactive paths probe BDA equipment and keyboard status early.
R11 seeded diskette/HD equipment (`40:10` / `40:75`) but left shift/mode/LED
keyboard fields zero and undocumented. See also
`docs/boot-r11-freedos-bda-equipment.md`.

## Spec

- RBIL BIOS Data Area / INT 11h equipment list word at `0040:0010`
- RBIL CMOS `14h` Table C0019 (same low-byte layout as BDA equipment):
  - **bit 0** = floppy drive installed (**not** "keyboard present")
  - **bit 2** = keyboard enabled ([`EQUIP_KEYBOARD_ENABLED`])
- RBIL MEM `0040:0017`/`0018` shift flags; `0040:0096` enhanced keyboard
  (bit4); `0040:0097` LED mirror

## Model

`Machine::seed_bda_keyboard_flags`:

1. Write equipment low byte = `equipment_byte()` (includes keyboard bit2;
   bit0 only if floppy media attached)
2. Clear `40:17` / `40:18`
3. Set `40:96` bit4 (enhanced 101/102-key present)
4. Clear `40:97` LED mirror
5. Ensure empty BDA keyboard ring

## Honesty

| Claim | Truth |
|---|---|
| "Equipment bit0 = keyboard" | **False** on IBM PC — bit0 is floppy; keyboard is bit2 |
| Full SeaBIOS POST equipment init | **No** — host helper only |
| Guest INT 09h/16h body | **No** |
| Auto-call from FreeDOS measure | **No** this lane — helper is explicit (boot measure core not edited) |

## Tests

- `crates/machine-pc/src/bda_kbd.rs` — `seed_bda_keyboard_flags_for_freedos`
