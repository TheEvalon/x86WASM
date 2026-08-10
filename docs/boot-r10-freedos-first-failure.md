# R10 FreeDOS-like measure → first-failure classify

Milestone 2, round 10, boot-guest lane, slice 3.

## Scope

Extend `Machine::measure_freedos_like` (R9 harness) with a structured
**first-failure class** and hang location, without claiming a FreeDOS prompt:

| Field | Role |
|---|---|
| `first_failure` | Fine class (`synthetic-halt`, `unsupported-opcode`, `step-budget`, `int13-cf`, …) |
| `failure_bucket` | Coarse bucket: `decode-ud` / `device` / `int13-cf` / `hang` / `halted` / `other` |
| `failure_site` | `CS:EIP` hang / stop location |
| `int13_probe` | Host INT 13h AH=41h snapshot (`DL=80h`) after the guest stop |

Classification uses the existing POST probe stop reason plus an optional host
INT 13h CF probe. Schema version is **v4** (`GUEST_OS_MEASURE_VERSION = 4`).

## Honesty

- Still **not** FreeDOS; reports always say NOT an OS boot / NOT Milestone 2 exit.
- Synthetic `HLT` is `bucket=halted` (fixture complete), not a success claim.
- Host INT 13h ≠ SeaBIOS IVT body.

## Spec

IBM PC BIOS INT 19h handoff; IBM/MS INT 13h Extensions AH=41h CF/AH status;
Intel SDM decode / `#UD`; `docs/boot-r8-guest-measure-v2.md`,
`docs/boot-r9-freedos-measure.md`.
