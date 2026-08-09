# fw_cfg firmware files: `etc/e820` and the named-file directory

Milestone 2 round 2, configuration-data slice 3. Adds the host-driven memory-map
blob and hardens the named-file directory that carries it.

## Spec

- QEMU Firmware Configuration (fw_cfg) Device,
  <https://www.qemu.org/docs/master/specs/fw_cfg.html> — "File Directory (Key
  0x0019, FW_CFG_FILE_DIR)" and "Data Register".
- ACPI Specification §15 "System Address Map Interfaces", Table 15.4 "Address
  Range Descriptor Structure" and §15.2 "Address Range Types".

## `etc/e820`

`FwCfg::set_e820_entries(&[E820Entry])` publishes the map, returning the
assigned selector, or `None` when the slice is empty. Each entry becomes the
20-byte ACPI address range descriptor:

| Offset | Field | Width |
|---|---|---|
| 0 | `BaseAddrLow` / `BaseAddrHigh` | 64-bit base address |
| 8 | `LengthLow` / `LengthHigh` | 64-bit length in bytes |
| 16 | `Type` | 32-bit range type |

Fields are little-endian, because the fw_cfg data register is
"string-preserving" and hands the blob to the guest byte for byte. Range types
come from ACPI §15.2: 1 `AddressRangeMemory`, 2 `AddressRangeReserved`,
3 `AddressRangeACPI`, 4 `AddressRangeNVS`, 5 `AddressRangeUnusable`. The ACPI
3.0 Extended Attributes dword at offset 20 is not emitted; the specification
makes 20 bytes the required minimum.

The device **never synthesizes entries** from the RAM-size item. The fw_cfg
specification defines the transport, not what a machine model must place in
this blob — in particular it does not say whether the file is the complete map
or a supplement to what firmware derives elsewhere. Publishing a guessed map
would risk a guest double-counting RAM it already learned about from CMOS, so
the host supplies the map it can justify and an empty map removes the file
outright rather than advertising a zero-length one.

## Named-file directory

- `add_file(name, data)` inserts and rejects a duplicate name; the directory is
  keyed by the `char name[56]` field, so two entries with one name would be
  ambiguous.
- `set_file(name, data)` replaces contents in place, keeping the selector stable
  so a guest that already walked the directory is not invalidated.
- `remove_file(name)` drops the item and its entry. The freed selector is not
  recycled, so a stale guest reference reads an unknown item (all `0x00`) rather
  than another file's contents.
- `file_selector(name)` / `file_names()` are host lookups.
- Names are limited to 55 characters plus the NUL terminator.

## Not implemented, and why

The specification documents only the signature (`0x0000`), the revision/feature
bitmap (`0x0001`), and the file directory (`0x0019`). Of everything else it
says: "Please consult the QEMU source for the most up-to-date and authoritative
list of selector keys and their respective items' purpose, format and
writeability."

This repository does not treat QEMU source as a specification, so the following
are **absent** rather than guessed, and firmware probing them receives the
specification's `0x00` for a read past the end of an item:

- Numeric keys `0x0002` UUID, `0x0004` nographic, `0x0005` NB_CPUS, `0x000F`
  max-cpus, and the kernel/initrd/cmdline group `0x0007`–`0x0018`.
- Named files `etc/max-cpus`, `etc/system-states`, `etc/table-loader`, and
  `bootorder`.

`etc/table-loader` in particular could not be filled truthfully in any case:
there are no ACPI tables to load. Implementing the rest needs an approved
interface definition added to `docs/sources.md` — the QEMU `fw_cfg.h` key list
that the specification itself defers to, or the equivalent SeaBIOS interface
header — because the key numbers alone do not fix each item's width or
semantics.

Item writeability is still unmodelled, so the DMA write direction remains
rejected with the specification's error bit.
