# R15 Linux serial/media deepen

Milestone 2, round 15, boot-guest lane, slice 4.

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
| `LinuxNextGap` | synthetic-media-halt / setup-loaded-missing-entry / **setup-executed-missing-protected-kernel** |
| `classify_linux_next_gap` | Fold media + halt + armed flag |
| `synthetic_linux_bzimage_setup_hlt` | Header + `+0x200` jmp to COM1 `LX` + HLT |
| `Machine::arm_bzimage_realmode_entry` | `CS=dest>>4`, `IP=0x200` |
| `Machine::measure_linux_bzimage_setup_entry` | Load @ `0x90000` + arm + probe |
| `GuestBootMedia::BzImageSetup` | Measure media tag |
| `ElToritoPayloadClass` | `no-emul-hlt-stub` / **`no-emul-bzimage`** |
| `classify_eltorito_boot_payload` | Catalog candidacy + boot-image peek |
| `synthetic_eltorito_bzimage_iso` | El Torito ISO with bzImage-shaped boot image |

## Honesty

- Synthetic setup / El Torito peeks only — not a real kernel or earlyprintk path.
- Host load/arm ≠ SeaBIOS CD INT 13h or bootloader handoff.
- **Not** a Linux serial shell or Milestone 2 exit.
- INT 13h AH=02/42 disk-transfer deepen stays with the **storage** lane.

## Spec

Linux `Documentation/x86/boot.rst` — real-mode kernel at `0x90000`, entry at
offset `0x200` (protocol ≥ 2.00); El Torito 1.0 Validation + Default Entry;
`docs/boot-r14-linux-eltorito-measure.md`.
