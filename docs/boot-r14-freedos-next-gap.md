# R14 FreeDOS next-gap after media (VBR chain)

Milestone 2, round 14, boot-guest lane, slice 3.

## Scope

Extend FreeDOS-path measure **past** `MediaAttachedBeyondRebootLoop` (v6) by
using the host MBR→VBR chain:

| Piece | Role |
|---|---|
| `GUEST_OS_MEASURE_VERSION` **v7** | VBR-chain next-gap |
| `FreedosHandoff::{MbrSector,ActiveVbr}` | Which sector was loaded to `0x7C00` |
| `FreedosNextGap::ExecutedVbrMissingCommand` | VBR ran + synthetic halt; no COMMAND.COM |
| `Machine::measure_freedos_vbr_chain` | Attach FreeDOS stub HD + ActiveVbr measure |

## Classify order (synthetic-halt path)

1. Host INT 13h CF → `host-int13-cf`
2. BDA mismatch → `bda-disk-mismatch`
3. IVT INT 13h null → `guest-int13-ivt-missing`
4. INT19 candidate + **ActiveVbr** → `executed-vbr-missing-command`
5. INT19 candidate + **MbrSector** → `media-attached-beyond-reboot-loop` (unchanged)
6. Else → `real-image-and-firmware`

## Honesty

- Still **not** a FreeDOS prompt.
- `executed-vbr-missing-command` means the stub VBR printed/halted under a host
  chain — **not** that SeaBIOS loaded the VBR or that COMMAND.COM exists.
- Real FreeDOS image + guest INT 13h remain open.

## Spec

OSDev Boot Sequence MBR→VBR; RBIL BDA/IVT; IBM INT 13h/19h.
`docs/boot-r13-freedos-with-media.md`, `docs/boot-r14-mbr-vbr-chain.md`.
