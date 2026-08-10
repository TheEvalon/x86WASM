# R7 FreeDOS/Linux guest measure-first harness

Milestone 2, round 7, storage/guest lane, slice 4.

Superseded for report schema by **v2**: see `docs/boot-r8-guest-measure-v2.md`
(checkpoints, serial capture, El Torito media). The R7 media helpers and CLI
`--guest-measure` entry point remain.

## Scope (historical)

Measure the **first** stop after a host boot-sector handoff to `0x7C00`.
Does **not** claim FreeDOS or Linux boot success.

| Entry | Role |
|---|---|
| `Machine::measure_guest_boot` | `load_*_7c00` then `probe_post` |
| `GuestBootMedia::{IdePrefer,FloppyFirst}` | Which handoff helper |
| CLI `--ide-image` / `--floppy-image` | Attach media |
| CLI `--guest-measure` | Run measure-first report |
| CLI `--guest-floppy-first` | Force floppy handoff |

## Honesty / blockers

- No FreeDOS or Linux guest image is vendored in-tree. Tests use a synthetic
  512-byte MBR (`HLT` or `UD2`) to prove the harness records a clear stop.
- Real FreeDOS/Linux first failure will surface once a host provides
  `--ide-image` / `--floppy-image`; expected blockers remain missing SeaBIOS
  INT 13h guest path, incomplete devices, and unimplemented opcodes.
- Halt / step-budget / first `PostFailure` are reported; none of those equal
  “OS booted”.

## Spec

IBM PC BIOS INT 19h handoff + existing POST probe diagnostics
(`docs` / `Machine::probe_post`).
