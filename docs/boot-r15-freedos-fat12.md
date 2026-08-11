# R15 FreeDOS past VBR — host FAT12 kernel-name locate

Milestone 2, round 15, boot-guest lane, slice 2.

Canonical write-up for the FAT12 next-gap (also `docs/boot-r15-freedos-next.md`).

## Context

R14 FreeDOS measure v7 stops at **`ExecutedVbrMissingCommand`** after the host
MBR→VBR chain (`docs/boot-r14-freedos-next-gap.md`).

## Scope

| Piece | Role |
|---|---|
| `fat12` module | BPB parse + root walk (FAT12 subset) |
| `locate_freedos_kernel_on_image` | Find `KERNEL.SYS` then `COMMAND.COM` |
| `synthetic_int19_freedos_fat12_hd` | INT19 HD + FAT12 VBR + root `KERNEL.SYS` |
| `GUEST_OS_MEASURE_VERSION` **v8** | FAT12 name next-gap |
| `FreedosNextGap::KernelNameLocatedMissingLoad` | Past `executed-vbr-missing-command` |
| `Machine::measure_freedos_fat12_root` | Attach FAT12 HD + ActiveVbr measure |

## Honesty

- Still **not** a FreeDOS prompt.
- Name locate ≠ loading clusters or executing `KERNEL.SYS`.
- Host FAT walk ≠ guest INT 13h AH=02h (`docs/storage-r15-int13-read-chs.md`).

## Spec

Microsoft FAT / FreeDOS `KERNEL.SYS`; OSDev FAT; `docs/boot-r14-freedos-next-gap.md`.
