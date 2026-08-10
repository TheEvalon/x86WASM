# fw_cfg `etc/system-states` honesty — Milestone 2 Round 8

## Why

Round 4 left `etc/system-states` absent because the PIIX PM block had no sleep
machine. Round 8’s PM1a `SLP_EN` stub now latches soft-off for `SLP_TYP=0`
(S5) and a host sleep request for other types (`docs/acpi-r8-pm1-sleep.md`).
That is enough to publish a **minimal truthful** blob from the machine model.

## Spec / interface

- ADR-0005 — file name and 6-byte layout (index = S-state; bit 7 = supported;
  bits 6:4 = `SLP_TYP` to write into `PM1_CNT`).
- ACPI — S0 is the working state; S5 is soft-off. S1–S4 require resume paths
  this tree does not implement.

## Blob (`FW_CFG_DEFAULT_SYSTEM_STATES`)

| Index | State | Value | Meaning |
|---|---|---|---|
| 0 | S0 | `0x80` | Supported (running) |
| 1–4 | S1–S4 | `0x00` | Not supported (no resume) |
| 5 | S5 | `0x80` | Soft-off; `SLP_TYP=0` (`ACPI_SLP_TYP_S5`) |

## Who publishes

| Layer | Behaviour |
|---|---|
| Bare `FwCfg::new()` | File **absent** |
| `Machine::sync_firmware_configuration` | Publishes the default blob |
| Host `set_system_states` | May override with another truthful encoding |

Unlike `etc/table-loader` (ADR-0008 — omitted forever until real ACPI tables),
`etc/system-states` is published because the PM1a path can actually honor S5.

## Not implemented

- S1–S4 support bits, hibernate (S4), or wake sources
- FADT / DSDT that would otherwise describe `SLP_TYP` (still absent)
- SMBIOS Type 0 as an alternative honesty path (not needed once this file exists)
