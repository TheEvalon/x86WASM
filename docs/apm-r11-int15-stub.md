# APM INT 15h AH=53h stub — Milestone 2 Round 11

## Why

Guests (FreeDOS, some firmware) probe Advanced Power Management through
`INT 15h, AH=53h` before using power services. Round 9 already stub-completed
the APM_CNT/APM_STS (`0xB2`/`0xB3`) SMI handshake without real SMM. This slice
adds a **host** APM BIOS 1.2 installation/connect subset so the software
interface check does not falsely report "APM absent".

## Spec

- Advanced Power Management (APM) BIOS Interface Specification 1.2 §4 —
  INT 15h AH=53h:
  - `AL=00h` Installation Check (`BX=0000h`) → version in `BH.BL`, flags in `CX`
  - `AL=01h` Connect Real-Mode Interface
  - `AL=04h` Disconnect Interface
  - `AL=02h`/`03h` protected-mode connect (unsupported here)
- Ralf Brown's Interrupt List — INT 15h AH=53h

## Model (R11)

| Call | Result |
|---|---|
| `AX=5300h`, `BX=0` | CF=0, `AH=00`, `BH/BL=1.2`, `CX=0` |
| `AX=5301h`, `BX=0` | connect once; second → `AH=02h`, CF=1 |
| `AX=5304h` | disconnect; idle → `AH=03h`, CF=1 |
| `AX=5302h`/`5303h` | `AH=86h`, CF=1 (unsupported) |

Helpers: `Machine::service_int15_apm`, `Machine::install_int15_ivt_pointer`
(pointer only), `Machine::apm_bios_connected`.

## Honesty

- **Not real SMM** — no SMBASE, no SMRAM entry, no RSM, no power-state change.
- Does not replace SeaBIOS APM; guest IVT still needs a real handler or an
  explicit host call into `service_int15_apm`.
- Protected-mode APM entry points are explicitly unsupported.

## Tests

- `crates/machine-pc/src/int15_apm.rs` — install check, connect/disconnect,
  PM unsupported, IVT pointer + reset clears connect.
