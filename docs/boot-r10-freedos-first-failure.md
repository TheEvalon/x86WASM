# R10 FreeDOS measure → first-failure classify

Milestone 2, round 10, boot-guest lane, slice 3.

## Scope

Extend `Machine::measure_freedos_like` / CLI `--guest-freedos-measure` so the
report carries a **structured first-failure class** (schema v4):

| Class tag | Meaning |
|---|---|
| `synthetic-halt` | Fixture printed banner then `HLT` (not FreeDOS progress) |
| `step-budget` | Budget exhausted without halt/failure |
| `unsupported-opcode` / `unsupported-encoding` | CPU decode/form gap |
| `arch-fault` / `pm-delivery` | Exception delivery gap |
| `unclaimed-io` / `unmapped-mmio` | Device/MMIO signal on budget stop |
| … | See `GuestFirstFailureClass` |

API: `classify_guest_first_failure`, `GuestOsMeasure.first_failure`,
`GuestFirstFailureClass::tag` / `gap_note`.

## Measured first failure (synthetic fixture)

With the in-tree FreeDOS-*like* MBR, the classified stop is **`synthetic-halt`**
after COM1 `FD` + VGA glyph. That is **not** a FreeDOS prompt and **not**
Milestone 2 exit.

No tiny INT13/BDA fix was required to keep the synthetic fixture green; real
FreeDOS still needs SeaBIOS INT 13h + fuller devices/opcodes.

## Honesty

Reports always say **NOT an OS boot / NOT Milestone 2 exit** and retain the
“does NOT claim a FreeDOS prompt” honesty line.

## Spec

IBM PC BIOS INT 19h handoff; existing POST probe failure kinds
(`docs/boot-r9-freedos-measure.md`).
