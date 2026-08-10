# R10 Linux serial measure → first-failure classify

Milestone 2, round 10, boot-guest lane, slice 4.

## Scope

Extend `Machine::measure_linux_serial_path` with the same v4 structured
first-failure report used by the FreeDOS-like harness:

| Field | Role |
|---|---|
| `first_failure` | Fine class from guest stop / INT 13h probe |
| `failure_bucket` | `decode-ud` / `device` / `int13-cf` / `hang` / `halted` / `other` |
| `failure_site` | Hang location `CS:EIP` |
| `int13_probe` | Host AH=41h on `DL=80h` after stop |

CLI `--guest-linux-serial-measure` prints the v4 report (bucket + site + INT13
probe). Synthetic stub may print `LX\r\n` to COM1; that is **not** earlyprintk
or a Linux shell.

## Honesty

- **Not** a bzImage boot, userspace, or Milestone 2 exit.
- Decode/#UD and hang location are triage aids only.
- Host INT 13h CF classification uses the in-tree host dispatcher, not SeaBIOS.

## Spec

IBM PC BIOS INT 19h handoff; 16550 COM1 capture via POST probe; IBM/MS INT 13h
Extensions AH=41h; `docs/boot-r9-linux-serial-measure.md`.
