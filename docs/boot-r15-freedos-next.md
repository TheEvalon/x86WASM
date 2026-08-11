# R15 FreeDOS next-gap past `ExecutedVbrMissingCommand`

Milestone 2, round 15, boot-guest lane, slice 3.

## Goal

Advance FreeDOS-path classify **past** R14 `executed-vbr-missing-command` by
host-walking a synthetic FAT12 root for `KERNEL.SYS` / `COMMAND.COM`.

Still **not** a FreeDOS prompt.

## Scope

| Piece | Role |
|---|---|
| `GUEST_OS_MEASURE_VERSION` **v8** | FAT12 kernel-name next-gap |
| `fat12` module | BPB parse + root walk + synthetic INT19 HD |
| `Fat12KernelLocate` | `kernel-sys-present` / `command-com-present` / … |
| `FreedosNextGap::KernelNameLocatedMissingLoad` | Past VBR-missing-command |
| `Machine::measure_freedos_fat12_root` | Attach FAT12 HD + ActiveVbr measure |

## Classify order (ActiveVbr + synthetic halt)

1. Host INT 13h CF → `host-int13-cf`
2. BDA mismatch → `bda-disk-mismatch`
3. IVT INT 13h null → `guest-int13-ivt-missing`
4. INT19 candidate + ActiveVbr + FAT12 name found → **`kernel-name-located-missing-load`**
5. INT19 candidate + ActiveVbr + no name → `executed-vbr-missing-command` (R14)
6. INT19 candidate + MbrSector → `media-attached-beyond-reboot-loop`
7. Else → `real-image-and-firmware`

## Synthetic image

`synthetic_int19_freedos_fat12_hd`: active FAT12 partition, BPB, FATs, root with
`KERNEL.SYS` (cluster 2 marker only). VBR prints `FD` then HLT.

## Honesty

- Name locate ≠ cluster load, kernel exec, or COMMAND.COM shell.
- Host FAT walk ≠ guest INT 13h / SeaBIOS disk path.
- **Not** a FreeDOS prompt.

## Spec

Microsoft FAT12 BPB / root directory; FreeDOS `KERNEL.SYS`; OSDev FAT;
IBM INT 13h/19h; `docs/boot-r14-freedos-next-gap.md`.
