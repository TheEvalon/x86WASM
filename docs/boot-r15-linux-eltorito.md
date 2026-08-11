# R15 Linux bzImage / El Torito deepen

Milestone 2, round 15, boot-guest lane, slice 4.

Canonical write-up (also summarized in `docs/boot-r15-linux-next.md`).

## Goal

One bounded deepen past R14 El Torito / serial stub halt:

1. **bzImage setup entry** — host-load synthetic real-mode setup, arm protocol
   entry at `+0x200`, measure → `setup-executed-missing-protected-kernel`.
2. **El Torito catalog → payload** — after no-emul candidacy, peek at boot
   image bytes for HLT stub vs bzImage-shaped header.

Still **not** a Linux serial shell.

## Scope

| Piece | Role |
|---|---|
| `LinuxNextGap` | … / **setup-executed-missing-protected-kernel** |
| `Machine::measure_linux_bzimage_setup_entry` | Load @ `0x90000` + arm + probe |
| `ElToritoPayloadClass` | `no-emul-hlt-stub` / **`no-emul-bzimage`** |
| `classify_eltorito_boot_payload` | Catalog candidacy + boot-image peek |
| `synthetic_eltorito_bzimage_iso` | El Torito ISO with bzImage-shaped boot image |

## Honesty

- Synthetic setup / El Torito peeks only — not a real kernel.
- Host load/arm ≠ SeaBIOS CD INT 13h.
- **Not** a Linux serial shell or Milestone 2 exit.

## Spec

Linux `Documentation/x86/boot.rst`; El Torito 1.0;
`docs/boot-r14-linux-eltorito-measure.md`.
