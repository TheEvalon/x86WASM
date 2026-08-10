# R13 El Torito media boot classify

Milestone 2, round 13, boot-guest lane, slice 4 (CD half).

## Scope

Host classify for attached ATAPI El Torito media:

| `ElToritoMediaBootClass` | Meaning |
|---|---|
| `no-medium` | No ATAPI CD image |
| `catalog-error` | ISO/catalog parse failed |
| `not-bootable` | Default entry not `88h` |
| `unsupported-emulation` | Floppy/HDD media type |
| `no-emul-candidate` | Bootable no-emul (RBA/sectors/seg) |

`classify_eltorito_media_boot(&Machine)` — inspect only; load remains
`load_eltorito_to_7c00`.

## Honesty

- **Not** SeaBIOS CD INT 13h stack or OS boot.
- Past empty-medium / no-media class only when `no-emul-candidate`.

## Spec

El Torito 1.0; `docs/storage-r10-int13-cd-eltorito.md`, `docs/boot-r8-eltorito-load.md`.
