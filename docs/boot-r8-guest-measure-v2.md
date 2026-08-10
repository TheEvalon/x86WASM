# R8 Guest boot measure harness v2

Milestone 2, round 8, boot-guest lane, slice 4.

## Scope

Extend the R7 measure-first harness toward a FreeDOS/Linux **serial-path**
bring-up workflow. Still measure-first only — **does not** claim OS boot or
Milestone 2 exit.

| Entry | Role |
|---|---|
| `Machine::measure_guest_boot` | Media handoff + `probe_post` |
| `GuestBootMedia::{IdePrefer,FloppyFirst,ElTorito}` | Handoff helper |
| `GuestBootMeasure` v2 | Version, checkpoints, COM1/debug capture, report |
| CLI `--guest-measure` | Run v2 report |
| CLI `--cdrom-image` / `--guest-eltorito` | El Torito measure path |
| CLI `--guest-floppy-first` | Floppy-first handoff |

## Checkpoints

Ordered markers (subset may omit `serial-observed`):

1. `media-loaded`
2. `cs-ip-armed`
3. `probe-started`
4. `serial-observed` (only if COM1 or `0x402` bytes exist at stop)
5. `stop-recorded`

## Honesty / remaining gaps (not M2 exit)

- No FreeDOS or Linux guest image is vendored in-tree; tests use synthetic MBR /
  El Torito HLT / COM1 OUT stubs.
- Guest `INT 13h` still needs SeaBIOS (host INT 13h HD subset is not an IVT
  body).
- El Torito path is host no-emul load only — not guest CD BIOS.
- Serial capture proves the harness can see COM1/`0x402` traffic; it does **not**
  mean FreeDOS reached a prompt or Linux reached userspace.
- Expected real-guest blockers remain: incomplete firmware POST, missing opcodes,
  incomplete devices, no full INT 13h stack.

## Spec

IBM PC BIOS INT 19h handoff; El Torito 1.0 no-emul load; existing
`Machine::probe_post` diagnostics.
