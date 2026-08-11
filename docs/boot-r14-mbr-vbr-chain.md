# R14 MBR → VBR chain to `0x7C00`

Milestone 2, round 14, boot-guest lane, slice 2.

## Scope

| Piece | Role |
|---|---|
| `find_active_partition` | Parse MBR for first `80h` entry (LBA + type) |
| `ActivePartition` | Slot / type / start LBA / sector count |
| `Machine::load_active_vbr_to_7c00` | Copy active partition VBR → phys `0x7C00`, `CS:IP=0000:7C00` |
| `GuestBootMedia::ActiveVbr` | Measure harness handoff selector |

## Behavior

1. Require IDE image with signed MBR (`0x55AA`).
2. Find first active partition (`80h` at `0x1BE+16*i`).
3. Read that LBA’s 512-byte sector; require `0x55AA`.
4. Install at `0x7C00` and arm `CS:IP`.

Errors: `NoBootMedia`, `InvalidMbrSignature`, `NoActivePartition`,
`IncompletePartitionSector`, `MbrRamTooSmall`.

## Honesty

- Host simulation of the classic MBR→VBR load — **does not** execute MBR code
  and **does not** claim SeaBIOS INT 19h success.
- R13 `load_mbr_to_7c00` still loads LBA0 only (MBR HLT on synthetic fixtures).

## Spec

IBM PC BIOS / OSDev Boot Sequence — after MBR, active partition boot sector to
`0000:7C00`; classic MBR boot indicator `80h`. See
`docs/boot-r13-int19-bootable-media.md`.
