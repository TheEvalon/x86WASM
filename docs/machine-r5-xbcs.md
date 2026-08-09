# PIIX XBCS BIOS write-protect — Milestone 2 Round 5

## Spec

Intel 82371AB (PIIX4) datasheet §4.1.9 — **XBCS** (X-Bus Chip Select) at
ISA-bridge configuration offsets `4E–4Fh`:

| Field | Meaning |
|---|---|
| Default | `03h` |
| Bit 2 | BIOSCS# Write Protect Enable — `1` asserts BIOSCS# for BIOS **writes**; `0` (default) asserts BIOSCS# for **reads only** |
| Bits 6–7 | Lower / Extended BIOS enables (stored; decode side effects deferred) |

Round 4 already dropped writes into mapped ROM windows. That approximated the
protect-in-force case. This slice makes **XBCS the register that owns the
policy**, with the datasheet default visible to guests.

## Model

- `machine_pc::Xbcs` holds the low byte; reset value `0x03`.
- `MachineBus` overlays Mechanism #1 accesses to `00:01.0` offset `4Eh` so
  reads return the XBCS value (the PCI stub's register file is not edited in
  this ownership slice).
- `PhysMem::bios_write_protect` mirrors bit2 inverted (`true` when bit2 clear).
- ROM writes remain [`WriteDisposition::DroppedRom`] whether or not protect is
  lifted (mask ROM / unsequenced flash still stores nothing).

## Unsupported

- High byte of XBCS (`4Fh`): APIC / mouse / FERR / 1M extended BIOS enables.
- Actually asserting BIOSCS# onto an ISA/X-Bus (no discrete ROM chip model).
- Mutating ROM image bytes when write-protect is lifted.
