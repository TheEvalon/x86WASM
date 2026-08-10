# R12 Linux bzImage early load / classify

Milestone 2, round 12, boot-guest lane, slice 4.

## Scope

Host helpers that inspect a bzImage-shaped buffer far enough to name the
**next failure mode**, optionally copying the real-mode setup blob into guest
RAM — without executing setup, jumping to protected mode, or claiming a serial shell.

| API | Role |
|---|---|
| `classify_bzimage_early` | Parse header + check setup size → `BzImageEarlyClass` |
| `load_bzimage_realmode_setup` | Copy `(setup_sects+1)×512` bytes to a phys dest (default `0x90000`) |
| `linux_setup_sect_count` / `linux_realmode_bytes` | Protocol size helpers (`setup_sects==0` → 4) |

### `BzImageEarlyClass`

| Tag | Meaning |
|---|---|
| `bad-header` | Truncated / bad `0xAA55` / missing `HdrS` |
| `incomplete-setup` | Header OK but buffer shorter than real-mode blob |
| `setup-loadable` | Setup size OK; `next` names the following gap |

### `BzImageNextStep` (when setup-loadable)

| Tag | Meaning |
|---|---|
| `run-real-mode-setup` | Next would be execute real-mode setup (out of scope) |
| `load-high-protected-kernel` | `LOADED_HIGH` — next is high kernel load / `code32_start` jump |
| `unsupported-old-protocol` | Defensive: version &lt; 2.00 |

## Honesty

- **Not** a Linux shell, earlyprintk path, or Milestone 2 exit.
- Does **not** vendor a real bzImage fixture.
- Does **not** arm `CS:IP` or run setup code after the host copy.
- Synthetic serial measure still uses the COM1 stub; host notes point at these helpers.

## Spec

Linux `Documentation/x86/boot.rst` (boot protocol ≥ 2.00);
`docs/boot-r11-linux-boot-protocol-inspect.md`.
