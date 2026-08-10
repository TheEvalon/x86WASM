# R11 Linux serial — boot-protocol header inspect

Milestone 2, round 11, boot-guest lane, slice 4.

## Scope

Host-side **inspect-only** helper for the Linux real-mode boot-protocol header:

| API | Role |
|---|---|
| `inspect_linux_boot_protocol_header` | Parse `setup_sects`, `boot_flag`, `HdrS`, version, `loadflags`, optional `code32_start` |
| `synthetic_linux_boot_protocol_header` | Tiny harness buffer (not a bzImage) |
| `measure_linux_serial_path` host-notes | On `synthetic-halt`, point at the inspect helper |

Does **not** vendor, load, or execute a bzImage. COM1 stub path unchanged.

## Honesty

- **Not** a Linux shell, earlyprintk driver, or Milestone 2 exit.
- Missing `HdrS` / `0xAA55` / short buffers return typed errors.
- Protected-mode jump / setup code execution remain out.

## Spec

Linux `Documentation/x86/boot.rst` (boot protocol ≥ 2.00 header fields);
`docs/boot-r10-linux-serial-first-failure.md`.
