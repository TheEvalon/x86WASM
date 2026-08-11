# R14 Linux / El Torito media measure deepen

Milestone 2, round 14, boot-guest lane, slice 4.

## Scope

Deepen Linux + El Torito classify/measure **with media attached**:

| Piece | Role |
|---|---|
| `LinuxMediaBootClass` | `no-media` / `eltorito` / `bzimage` / `eltorito-plus-bzimage` |
| `classify_linux_media_boot(machine, bzimage?)` | Fold CD classify + optional bzImage deepen |
| `synthetic_eltorito_linux_hlt_iso` | No-emul HLT boot image for measure |
| `Machine::measure_linux_with_eltorito_media` | Attach CD + `load_eltorito_to_7c00` + first-failure |

`classify_linux_media_boot` uses [`classify_eltorito_media_boot`] and, when a
buffer is supplied, [`classify_bzimage_setup_deeper`].

## Honesty

- **Not** a Linux serial shell, userspace, or Milestone 2 exit.
- El Torito measure runs a synthetic HLT sector — not a real bzImage.
- Host `load_eltorito_to_7c00` ≠ SeaBIOS CD INT 13h stack.
- First-failure is recorded honestly (typically `synthetic-halt` on the stub).

## Spec

El Torito 1.0; Linux `Documentation/x86/boot.rst`;
`docs/boot-r13-eltorito-media-classify.md`, `docs/boot-r13-linux-setup-deeper.md`,
`docs/sources.md`.
