# CMOS configuration bytes POST reads

Milestone 2 round 2, configuration-data slices 2 and 4. Covers the CMOS RAM
bytes BIOS POST reads to learn how much memory the machine has and whether the
rest of the register file can be trusted.

## Spec

- Ralf Brown's Interrupt List, CMOS memory map:
  - `0Eh` IBM PS/2 "DIAGNOSTIC STATUS BYTE" (Table C0005)
  - `14h` IBM "EQUIPMENT BYTE" (Table C0019)
  - `15h`/`16h` IBM "BASE MEMORY IN KB"
  - `17h`/`18h` and `30h`/`31h` IBM "EXTENDED MEMORY IN KB"
  - `2Eh`/`2Fh` IBM "Standard CMOS Checksum, High/Low Byte"
  - `34h`/`35h` "EXTENDED MEMORY >16M", in 64 KB blocks
- RBIL INT 15h AX=E801h "GET MEMORY SIZE FOR >64M CONFIGURATIONS" — fixes the
  split between the two memory windows.
- IBM PC/AT — CMOS RAM is battery backed.

## Memory size

`CmosRtc::set_memory_size(ram_bytes)` fills all four pairs:

| Registers | Contents | Clamp |
|---|---|---|
| `15h`/`16h` | base memory, KB | 640 KB (the DOS area) |
| `17h`/`18h` | extended memory above 1 MB, KB | `3C00h` = 15 MB |
| `30h`/`31h` | same value as `17h`/`18h` | `3C00h` |
| `34h`/`35h` | memory above 16 MB, 64 KB blocks | `FFFFh` |

All pairs are little-endian low byte first. RBIL documents `17h`/`18h` as the
user-configured figure and `30h`/`31h` as the POST-measured one; with no setup
utility in the loop, both report the same value.

The `34h`/`35h` split follows INT 15h AX=E801h ("AX = extended memory between
1M and 16M, in K (max 3C00h = 15MB)", "BX = extended memory above 16M, in 64K
blocks") rather than the AMI reading of `34h`/`35h` as *total* extended memory,
because that is the only reading consistent with the 15 MB cap on the KB pairs.

Memory above 4 GB is not reported. The CMOS indices conventionally used for it
(`5Bh`–`5Dh`) appear only in emulator documentation, not in any authoritative
register map, so the model saturates rather than inventing an encoding. Any
approved source for those bytes would need a `docs/sources.md` entry first.

## Diagnostic status, equipment byte, checksum

All three are plain storage with host accessors:

- `diagnostic_status()` / `set_diagnostic_status()` for `0Eh`.
- `equipment_byte()` / `set_equipment_byte()` for `14h`, composed from the
  `EQUIP_*` constants. `CmosRtc::equipment_floppy_field(drives)` encodes the
  awkward part: bits 7-6 count from `00b` = 1 Drive, and bit 0 reports that a
  drive is installed at all, so zero drives clears both and counts above four
  saturate.
- `standard_checksum()`, `store_standard_checksum()`, and
  `standard_checksum_valid()` for `2Eh`/`2Fh`. The sum is byte-wise additive
  over `10h`–`2Dh` inclusive, stored high byte at `2Eh` and low byte at `2Fh`.
  Thirty bytes of `FFh` cannot overflow 16 bits, so the sum is exact.

The device never sets a diagnostic bit and never recomputes a checksum on its
own. Turning a stale checksum into `DIAG_BAD_CHECKSUM` is POST's decision, and
modelling it here would put firmware policy in a register file.

## Battery-backed bytes

`CmosRtc::reset` preserves `0Eh`, `0Fh`, `14h`, `15h`–`18h`, `2Eh`, `2Fh`,
`30h`, `31h`, `34h`, `35h`. Everything else returns to its power-on state.

That set is chosen so a soft reset cannot erase the memory map or invalidate a
checksum the host just stored. It is still incomplete: the floppy-type byte
`10h`, the hard-disk type bytes `12h` and `19h`–`2Ch`, and the boot-device byte
`2Dh` are inside the checksum range but are *not* preserved, so a host that
programs them has to call `store_standard_checksum` again after a reset. Those
bytes are not populated by this slice at all.

## Wiring

`Machine::new` currently constructs `CmosRtc::new()`, which leaves every byte in
this document zero. The machine layer needs to call `set_memory_size` with its
configured RAM size, then `set_equipment_byte` and `store_standard_checksum`,
for a guest to see anything.
