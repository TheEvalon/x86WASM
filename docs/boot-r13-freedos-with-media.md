# R13 FreeDOS measure with bootable media

Milestone 2, round 13, boot-guest lane, slice 2.

## Scope

Extend FreeDOS-path measure to **INT 19h-candidate** media and classify past
the POST no-media reboot-loop class:

| Piece | Role |
|---|---|
| `GUEST_OS_MEASURE_VERSION` **v6** | Adds `media_readiness` |
| `MediaBootReadiness` | `no-media` / `attached-not-candidate` / `int19-candidate` |
| `FreedosNextGap::MediaAttachedBeyondRebootLoop` | IVT+BDA OK + INT19 candidate |
| `Machine::measure_freedos_with_bootable_media` | Attach FreeDOS stub HD + measure |

## Honesty

- Still **not** a FreeDOS prompt.
- `media=int19-candidate` means firmware would leave the no-media CF9 loop class;
  it does **not** mean SeaBIOS guest INT 13h or OS boot succeeded.
- Host MBR handoff executes the stub MBR (`HLT`), not the partition VBR.

## Spec

IBM INT 19h; RBIL BDA/IVT; `docs/boot-r12-freedos-next-gap.md`,
`docs/boot-r13-int19-bootable-media.md`.
