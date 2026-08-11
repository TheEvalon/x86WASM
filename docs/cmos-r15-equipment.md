# CMOS equipment / floppy bytes ↔ BDA — Milestone 2 Round 15

## Spec

RBIL CMOS `14h` / `10h`; MEM `0040:0010`.

## Model

`cmos_equipment_coherent` + `seed_bda_equipment_from_cmos`. Keyboard bit2
(R14) always preserved by `equipment_byte()` / sync.

## Honesty

Host helpers only; BDA not auto-refreshed on floppy attach.

## Tests

`crates/machine-pc/src/cmos_equipment.rs`
