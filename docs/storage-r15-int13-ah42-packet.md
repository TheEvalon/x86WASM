# R15 INT 13h AH=42h packet read deepen

Milestone 2, round 15, storage-int13 lane, slice 2.

## Scope

Deepen host IBM/MS INT 13h Extensions **AH=42h** (Disk Address Packet) for
hard disk `DL=80h`:

| Case | Behavior |
|---|---|
| Exact end-of-media LBA range | `CF` clear; DAP block count rewritten to transferred |
| LBA+count past media end | `CF` + `AH=04h`; DAP block count cleared to `0` |
| Zero block count | `CF` + `AH=01h` (invalid) |
| Flat 64-bit buffer `FFFF:FFFF` | `CF` + `AH=01h` (unsupported) |
| Short DAP (`size < 10h`) | `CF` + `AH=01h` (unchanged) |

DAP count writeback follows IBM/Microsoft INT 13h Extensions: after the call the
packet's block-count field holds the number of blocks successfully transferred.

## Honesty / unsupported

- Host subset only — not a guest IVT BIOS body.
- Classic `seg:off` buffer only; EDD device-path / 64-bit flat buffer out.
- Partial mid-transfer success (some blocks then fail) is not modelled —
  transfers are all-or-nothing, so failure always rewrites count to `0`.
- AH=43h DAP count writeback not changed in this slice.

## Spec

IBM/Microsoft INT 13h Extensions AH=42h; Phoenix EDD; RBIL INT 13h AH=42h;
`docs/storage-r8-int13-extensions.md`.
