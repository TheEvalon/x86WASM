# R12 INT 13h AH=41h extensions bitmap honesty

Milestone 2, round 12, boot-guest lane, slice 2.

## Scope

Deepen host INT 13h **AH=41h check extensions** so the `CX` support bitmap
matches the functions this tree actually implements:

| Bit | Meaning | Advertised? | Backed by |
|---|---|---|---|
| 0 (`INT13_EXT_CX_PACKET`) | Packet device access | yes | HD AH=42h/43h; CD AH=42h |
| 1 (`INT13_EXT_CX_LOCKING`) | Removable lock/eject | **no** (always clear) | — |
| 2 (`INT13_EXT_CX_EDD`) | EDD drive parameters | yes | AH=48h |

`AH` returns major version [`INT13_EXT_VERSION`] (`01h`); `BX` magic handshake
unchanged (`55AAh` → `AA55h`).

## Honesty

- Packet bit0 historically covers AH=42h–44h/47h. This host implements
  **AH=42h/43h** (HD) and **AH=42h** (CD) only.
- **AH=44h** extended verify and **AH=47h** extended seek return `CF` + `AH=01h`.
- CD AH=43h extended write also returns invalid (no write path).
- Still **not** a guest IVT BIOS / SeaBIOS body.

## Spec

IBM/Microsoft INT 13h Extensions (Phoenix EDD) / RBIL INT 13h AH=41h;
`docs/storage-r8-int13-extensions.md`.
