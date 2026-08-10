# ACPI PM1a sleep stub (SLP_EN / SLP_TYP) — Milestone 2 Round 8

## Why

Firmware and OSPM write `PM1a_CNT` with `SLP_TYP` + `SLP_EN` to enter a sleep
state or soft-off. Round 5 ignored `SLP_EN` entirely. This slice latches a
**host-visible** request without inventing S3 resume, GPE wake, or a full power
controller.

## Spec

- ACPI Specification — fixed hardware `PM1a_CNT_BLK`: sticky `SCI_EN` / `BM_RLD`
  / `SLP_TYPx`; `SLP_EN` is a **write-only** trigger (does not read back as 1).
- Intel 82371AB (PIIX4) — ACPI PM I/O at `PMBASE+4` (`PM1_CNT`).
- `SLP_TYP` encoding is **platform-defined** (normally published in FADT). This
  tree has no FADT (ADR-0008), so the mapping below is an explicit model choice.

## Model

| Guest write | Register readback | Host latch |
|---|---|---|
| `SLP_TYP` only | Sticky in `PM1_CNT` bits [12:10] | None |
| `SLP_EN=1` and `SLP_TYP == 0` (`ACPI_SLP_TYP_S5`) | `SLP_EN` cleared; `SLP_TYP` sticky | `acpi_power_off_pending` (soft-off / S5) |
| `SLP_EN=1` and `SLP_TYP != 0` | Same | `acpi_sleep_requested` + captured typ (no resume) |

Helpers on `PciConfig`:

- `acpi_power_off_pending` / `take_acpi_power_off_request`
- `acpi_sleep_request` / `take_acpi_sleep_request` → `Option<u8>` typ

`Machine` may poll the take helpers the same way it services 8042/`0x92` reset
latches. No automatic VM exit is required in this slice.

## Not implemented

- S1–S4 enter/exit, resume vectors, waking from GPE/RTC/PWRBTN while asleep
- SMI path when `SCI_EN=0`
- Real power-rail cut / browser "powered off" UI (host may react to the latch)
- FADT `SLP_TYP` table (still absent; see `docs/fwcfg-r8-system-states.md`)
