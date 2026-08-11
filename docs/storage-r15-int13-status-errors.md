# R15 INT 13h status / error-code polish

Milestone 2, round 15, storage-int13 lane, slice 3.

## Scope

Polish host INT 13h **CF / AH** boundary codes and BDA last-status mirroring so
SeaBIOS-style callers see consistent results:

| Condition | `CF` | `AH` | BDA |
|---|---|---|---|
| Success | clear | `00h` | floppy `0040:0041` or HD/CD `0040:0074` ← `00h` |
| Invalid / bad drive / bad DAP | set | `01h` | mirrored |
| Write protected (floppy) | set | `03h` | mirrored |
| Sector / LBA not found | set | `04h` | mirrored |
| No media / not ready | set | `80h` | mirrored |

`DL < 80h` selects the floppy BDA byte; `DL ≥ 80h` (HD `80h`, CD `E0h`) selects
the hard-disk BDA byte. AH=08h mirrors **before** overwriting `DL` with the
drive-count return value.

Transfer failures also clear `AL` to `0` (AH=02h/03h/04h).

## Honesty / unsupported

- Host subset only — not SeaBIOS INT 13h body / AH=01h get-status service.
- Partial-transfer `AL` (sectors before error) remains out.
- AH=15h returns a **type** code in `AH` with CF clear and does **not** treat
  that type as a BDA disk-status code.

## Spec

IBM PC BIOS INT 13h status codes; RBIL INT 13h + BDA `0040:0041` / `0040:0074`;
`docs/storage-r13-int13-reset.md`.
