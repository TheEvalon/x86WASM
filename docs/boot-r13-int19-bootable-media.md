# R13 INT 19h bootable media attach

Milestone 2, round 13, boot-guest lane, slice 1.

## Context

After CF9, SeaBIOS POST reaches INT 19h and, with **no** disk/CD, hits
`boot_fail` → `qemu_reboot` → CF9 → reboot loop (`F000:9842`). Attaching a
minimal signed boot image is the host-side unblocker for that class.

## Scope

| API | Role |
|---|---|
| `synthetic_int19_bootable_hd` | Signed MBR + active FAT12 partition + HLT VBR |
| `synthetic_int19_freedos_stub_hd` | Same + FreeDOS-like VBR (COM1 `FD`) |
| `synthetic_int19_bootable_floppy` | 1.44MB signed HLT VBR |
| `classify_int19_boot_image` | `too-short` / `missing-signature` / `hd-signature-only` / `hd-active-partition` / `floppy-boot-sector` |
| `Machine::attach_bootable_*_for_int19` | Wire IDE/FDC helpers |

## Honesty

- **Not** FreeDOS, COMMAND.COM, or SeaBIOS INT 19h success.
- Active partition marks INT 19h candidacy; host `load_mbr_to_7c00` still runs the MBR.
- No real filesystem / BPB beyond the partition table bytes.

## Spec

IBM PC BIOS INT 19h / OSDev Boot Sequence; classic MBR boot indicator `80h`;
floppy `0x55AA`. See `docs/post-c897-remeasure.md`.
