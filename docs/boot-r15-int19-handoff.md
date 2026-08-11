# R15 INT 19h host handoff toward `0000:7C00`

Milestone 2, round 15, boot-guest lane, slice 2.

## Goal

Push measure/helpers so a **boot-sector execution** class (`guest-halted-at-boot-sector`
at `0000:7C00`) is reachable. Honesty: this is a **host** INT 19h-order path —
SeaBIOS POST-with-media still stops at `F000:C897` (`docs/post-r15-c897-with-media.md`).

## Scope

| Piece | Role |
|---|---|
| `Int19HandoffMedia` | `floppy-boot-sector` / `hd-mbr` / `hd-active-vbr` |
| `Machine::host_int19_load_boot_sector` | Floppy-then-HD order; optional VBR chain |
| `Machine::measure_host_int19_boot_sector` | Load + probe → classify halt at `7C00` |
| `Int19HandoffReport` | Media + `PostWithMediaClass` + honesty |

## Behavior

1. If a signed floppy boot sector is present → load to `0x7C00`, arm `CS:IP`.
2. Else if `chain_active_vbr` and an active partition exists → host VBR chain.
3. Else → IDE LBA0 (MBR) to `0x7C00`.
4. Short probe on synthetic HLT media classifies `guest-halted-at-boot-sector`.

## Honesty

- **Not** SeaBIOS INT 19h success (firmware path still `wait_irq` / C897).
- **Not** FreeDOS / Linux boot.
- Host load ≠ guest INT 13h disk BIOS.

## Spec

IBM PC BIOS INT 19h / OSDev Boot Sequence; classic MBR `80h` active partition;
`docs/boot-r14-mbr-vbr-chain.md`, `docs/post-r15-c897-with-media.md`.
