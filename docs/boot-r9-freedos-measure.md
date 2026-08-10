# R9 FreeDOS-like guest measure harness

Milestone 2, round 9, boot-guest lane, slice 3.

## Scope

Extend the R8 guest-measure v2 harness with a **FreeDOS-*like*** path:

| Entry | Role |
|---|---|
| `synthetic_freedos_like_disk` | Signed MBR (`FD` to COM1 + VGA glyph + HLT) + LBA1 payload marker |
| `Machine::measure_freedos_like` | Attach fixture if needed; run measure; honesty + gaps |
| `GuestOsMeasure` (v3) | Wraps v2 + `honesty` + `gaps` |
| CLI `--guest-freedos-measure` | Run without requiring a host image file |

Checkpoints include R8 serial markers plus **`vga-observed`** when printable
VGA text exists at stop.

## Honesty (required)

- This is **not** FreeDOS. It does **not** claim a FreeDOS prompt,
  `COMMAND.COM`, or Milestone 2 exit.
- The payload marker (`FREEDOS-LIKE-PAYLOAD`) proves a multi-sector fixture
  only; it is not executed by the MBR stub (no guest INT 13h chain).
- Real FreeDOS still needs SeaBIOS INT 13h, fuller devices/opcodes, and a
  real image supplied by the host (`--ide-image` / `--floppy-image`).

## Spec

IBM PC BIOS INT 19h handoff; existing `Machine::probe_post` diagnostics;
R8 measure schema in `docs/boot-r8-guest-measure-v2.md`.
