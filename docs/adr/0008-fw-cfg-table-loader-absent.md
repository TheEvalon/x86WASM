# ADR-0008: `etc/table-loader` stays absent forever (no empty loader)

- Status: Accepted
- Date: 2026-08-10

## Context

SeaBIOS and QEMU-compatible firmware may look for the fw_cfg named file
`etc/table-loader`. That file is a command stream (allocate / add-pointer /
add-checksum / write-pointer) that installs ACPI tables published as other
fw_cfg blobs (RSDP, XSDT/RSDT, FADT, …). Interface facts for the file name and
command layout come from ADR-0005 (headers as interface reference only).

This machine implements no ACPI tables. Milestone 2 still has only a bounded
PIIX4 PM I/O stub (`PM1a_*` / `PM_TMR`) with no FADT, RSDP, or DSDT. Publishing
`etc/table-loader` would force one of two dishonest answers:

1. A zero-entry loader blob — still advertises the loader protocol while
   listing nothing to load.
2. Invented RSDP/FADT contents — would claim ACPI fixed-hardware layout the
   tree does not yet own end-to-end.

## Decision

**`etc/table-loader` is omitted permanently until real ACPI tables exist.**

- `FwCfg::new()` and `Machine::sync_firmware_configuration` never publish the
  name.
- `FwCfg::file_selector("etc/table-loader")` returns `None`.
- There is no host setter that invents tables. A host may still use the generic
  `add_file` / `set_file` APIs for experiments; default and sync paths must not.

When ACPI tables are eventually published, a new ADR (or an update to this one)
must describe the loader command stream and the table blobs together. Until
then, absence is the truthful answer.

## Consequences

Easier: firmware that treats a missing directory entry as “no ACPI via fw_cfg”
behaves correctly; agents stop re-litigating empty vs omit.

Harder: guests that *require* `etc/table-loader` to find any ACPI will not see
tables through fw_cfg on this machine. That is intentional until tables are
real.
