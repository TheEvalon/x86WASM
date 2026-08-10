# R11 FreeDOS measure — BDA equipment + host notes

Milestone 2, round 11, boot-guest lane, slice 3.

## Scope

When `Machine::measure_freedos_like` runs:

1. Seed classic BDA disk fields from attached media via
   `Machine::seed_bda_disk_equipment`:
   - `0040:0010` equipment list (low byte = `equipment_byte()`)
   - `0040:0075` hard-disk count (`1` if IDE image present)
2. If first-failure class is already `synthetic-halt`, append **host-notes**
   describing the next real gap (guest image / SeaBIOS POST), without claiming
   a FreeDOS prompt.

## Honesty

- Still **not** FreeDOS; reports always say NOT an OS boot.
- BDA seed is a host helper — not SeaBIOS POST equipment init.
- Synthetic `HLT` remains `bucket=halted`.

## Spec

RBIL BIOS Data Area (`0040:0010`, `0040:0075`); IBM equipment byte / CMOS `14h`;
`docs/boot-r10-freedos-first-failure.md`.
