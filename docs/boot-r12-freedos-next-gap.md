# R12 FreeDOS measure — next-gap classify

Milestone 2, round 12, boot-guest lane, slice 3.

## Scope

Extend `Machine::measure_freedos_like` / schema **v5** with a structured
**next-gap** class beyond `synthetic-halt`, without claiming a FreeDOS prompt:

| `FreedosNextGap` | When |
|---|---|
| `host-int13-cf` | Host INT 13h AH=41h probe returned CF |
| `bda-disk-mismatch` | BDA `0040:0010` / `0040:0075` disagree with attached media |
| `guest-int13-ivt-missing` | IVT vector `0x13` is null (no SeaBIOS / host stub) |
| `real-image-and-firmware` | Fixture halted with BDA OK + IVT present — need real image + POST |
| `see-first-failure` | Non-halt stop — use `first_failure` |

Priority on synthetic-halt: INT13 CF → BDA mismatch → IVT null → real image.

## Honesty

- Still **not** FreeDOS; reports always say NOT an OS boot / NOT a prompt.
- Host INT 13h ≠ guest IVT body.
- Installing an IVT pointer alone does **not** mean SeaBIOS disk services exist.

## Spec

RBIL IVT + BIOS Data Area disk fields; IBM INT 13h;
`docs/boot-r10-freedos-first-failure.md`, `docs/boot-r11-freedos-bda-equipment.md`.
