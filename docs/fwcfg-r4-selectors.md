# fw_cfg numeric selectors and named files

What `devices::FwCfg` publishes, what it deliberately does not, and the rule
that decides which.

## Authority

- [QEMU Firmware Configuration (fw_cfg) Device] specification — the
  selector/data protocol, the DMA interface, the file directory layout, and the
  rule that a read past the end of an item returns `0x00`.
- `docs/adr/0005-fw-cfg-key-list-interface-reference.md` — QEMU's `fw_cfg.h`
  and SeaBIOS's headers are approved as an **interface reference only**: key
  numbers, field widths, blob layouts, firmware file names. No implementation
  logic was read or copied, and the approval covers fw_cfg interface
  definitions and nothing else.

The specification itself defines only `0x0000` (signature), `0x0001`
(revision/feature bitmap) and `0x0019` (file directory), and refers the reader
to the QEMU source for the rest. ADR-0005 exists precisely because that leaves
an emulator two bad options — invent key numbers, or read a header and be
unsure whether it broke the no-copying rule — and picks a third.

[QEMU Firmware Configuration (fw_cfg) Device]: https://www.qemu.org/docs/master/specs/fw_cfg.html

## The rule

A selector or file is published only when this machine can fill it
**truthfully**. Anything else stays absent, and firmware reading it gets the
specification's `0x00`. An empty-but-present item is worse than an absent one:
it looks like an answer.

## Published unconditionally

| Key / file | Width | Value | Why it is truthful |
|---|---|---|---|
| `0x0005` NB_CPUS | LE16 | `1` | One execution context, no SMP anywhere in the tree |
| `0x000F` max-cpus | LE16 | `1` | Same count; there is no CPU hotplug |
| `etc/max-cpus` | LE16 | `1` | The file form of the same fact, for firmware that reads it instead |

`FwCfg::set_cpu_count` writes all three together, so a guest can never see two
different answers, and clamps zero to one — a machine with no CPU cannot be
running the firmware asking the question.

## Host-settable, absent by default

Each of these describes a machine fact the device cannot state on its own, so
it is absent until a host supplies it.

| Key / file | Setter | Layout |
|---|---|---|
| `0x0002` UUID | `set_system_uuid` / `clear_system_uuid` | 16 raw bytes |
| `0x0004` nographic | `set_nographic` | LE16, 1 = no graphics adapter |
| `bootorder` | `set_boot_order` / machine sync | newline-separated paths, trailing newline, NUL-terminated |
| `etc/system-states` | `set_system_states` / machine sync | 6 bytes indexed by S-state; bit 7 supported, bits 6:4 `SLP_TYP` |

`etc/system-states`: the bare device leaves it absent. Round 8’s PM1a soft-off
stub makes S0+S5 publishable — `Machine::sync_firmware_configuration` writes
`FW_CFG_DEFAULT_SYSTEM_STATES` (docs/fwcfg-r8-system-states.md). S1–S4 stay
unsupported. `set_boot_order(&[])` removes the bootorder file rather than
publishing an empty policy, for the same reason `set_e820_entries(&[])` removes
`etc/e820`.

### Machine-default `bootorder`

The bare `FwCfg` device still leaves `bootorder` absent. A running
`Machine::sync_firmware_configuration` publishes `FW_CFG_DEFAULT_BOOT_ORDER`:

1. `/pci@i0cf8/ide@1,1/drive@0/disk@0` — primary master HDD
2. `/pci@i0cf8/ide@1,1/drive@2/disk@0` — secondary master (ATAPI CD-ROM slot)
3. `/pci@i0cf8/isa@1/fdc@03f0/floppy@0` — ISA floppy

Host override: `Machine::set_fw_cfg_boot_order` (empty removes the file) survives
sync/reset until `use_default_fw_cfg_boot_order`.

## Deliberately absent: `etc/table-loader`

Policy authority: **ADR-0008** (accepted 2026-08-10).

| File | Policy | Why |
|---|---|---|
| `etc/table-loader` (`FW_CFG_FILE_TABLE_LOADER`) | **Omitted forever** until real ACPI tables exist — never present in the file directory | The QEMU/SeaBIOS table-loader blob is a command stream (allocate / add-pointer / add-checksum / write-pointer) that installs ACPI tables from other fw_cfg files. This tree has no RSDP/XSDT/FADT (or any other ACPI table). Publishing a zero-entry loader would still advertise the loader protocol while listing nothing honest to load; inventing RSDP/FADT would lie about fixed hardware. Omitting the name is the truthful answer. |

Name lookup (`FwCfg::file_selector("etc/table-loader")`) returns `None`. A guest
that never saw the name in the directory has no selector to read; probing an
unknown selector still yields the specification's `0x00`.
`Machine::sync_firmware_configuration` must not invent the file.

## Not implemented

- Every other numeric key. Absent items read `0x00`.
- Item writeability: selector bit 14 and DMA control bit 4 (write) are rejected
  with the spec's error bit rather than modelled.

## Wiring status

`FwCfg::new()` and `Machine::sync_firmware_configuration` both publish the
CPU-count views (`NB_CPUS` / `max-cpus` / `etc/max-cpus` = 1). Sync also
publishes the machine-default `bootorder` (HDD → CD → floppy) unless the host
has overridden it, and `etc/system-states` (S0 + S5 — docs/fwcfg-r8-system-states.md).
UUID and nographic remain absent until the host supplies a truthful value.
**`etc/table-loader` stays absent through sync** (ADR-0008).
