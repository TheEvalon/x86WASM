# i440FX PMC Programmable Attribute Map (PAM0–PAM6)

Milestone 2 round 2, configuration-data slice 1. Implements the shadow-RAM
attribute register file on the host bridge at `00:00.0`, config offsets
`0x59`–`0x5F`, plus the decoded host accessor the machine layer needs in order
to steer physical memory at ROM or at DRAM.

## Spec

Intel 440FX PCIset (82441FX PMC / 82442FX DBX) datasheet, order 290549-001,
May 1996:

- §3.2.18 "PAM — Programmable Attribute Map Registers (PAM[6:0])" — address
  offset PAM0 `59h` … PAM6 `5Fh`, default value `00h`, attribute Read/Write.
- Table 2 "Attribute Bit Assignment" — bits `[7, 6, 3, 2]` Reserved, bits
  `[5, 1]` WE, bits `[4, 0]` RE, and the four field encodings.
- Table 3 "PAM Registers and Associated Memory Segments" — the thirteen
  attribute-controlled segments.

Reserved-bit treatment follows the PCI Local Bus Specification rule that
reserved configuration fields are read-only and return zero.

## Register file

Each register holds two independent 4-bit attribute fields, the low nibble for
the lower segment and the high nibble for the upper segment:

| Bits within a field | Meaning |
|---|---|
| 3:2 | Reserved (read 0) |
| 1 | WE — write enable |
| 0 | RE — read enable |

`RE = 1` directs CPU reads of the segment to main memory; `RE = 0` directs them
to PCI. `WE = 1` directs CPU writes to main memory; `WE = 0` directs them to
PCI. The four encodings are therefore Disabled (`00`), Read Only (`01`),
Write Only (`10`), and Read/Write (`11`).

Table 3, in the ascending address order this implementation uses:

| Index | Segment | Register / nibble | Datasheet comment |
|---|---|---|---|
| 0 | `0C0000-0C3FFF` | PAM1[3:0] | ISA Add-on BIOS |
| 1 | `0C4000-0C7FFF` | PAM1[7:4] | ISA Add-on BIOS |
| 2 | `0C8000-0CBFFF` | PAM2[3:0] | ISA Add-on BIOS |
| 3 | `0CC000-0CFFFF` | PAM2[7:4] | ISA Add-on BIOS |
| 4 | `0D0000-0D3FFF` | PAM3[3:0] | ISA Add-on BIOS |
| 5 | `0D4000-0D7FFF` | PAM3[7:4] | ISA Add-on BIOS |
| 6 | `0D8000-0DBFFF` | PAM4[3:0] | ISA Add-on BIOS |
| 7 | `0DC000-0DFFFF` | PAM4[7:4] | ISA Add-on BIOS |
| 8 | `0E0000-0E3FFF` | PAM5[3:0] | BIOS Extension |
| 9 | `0E4000-0E7FFF` | PAM5[7:4] | BIOS Extension |
| 10 | `0E8000-0EBFFF` | PAM6[3:0] | BIOS Extension |
| 11 | `0EC000-0EFFFF` | PAM6[7:4] | BIOS Extension |
| 12 | `0F0000-0FFFFF` | PAM0[7:4] | BIOS Area |

`PAM0[3:0]` is Reserved, so twelve 16 KiB segments plus one 64 KiB segment give
the thirteen segments the datasheet describes.

## Host accessor

```rust
pub struct PamRegion {
    pub start: u32,          // inclusive guest-physical start
    pub end: u32,            // inclusive guest-physical end
    pub read_from_ram: bool, // RE=1 -> reads go to DRAM, not PCI/ROM
    pub write_to_ram: bool,  // WE=1 -> writes go to DRAM, not PCI/ROM
}

impl PciConfig {
    pub fn pam_register(&self, index: usize) -> Option<u8>;
    pub fn pam_registers(&self) -> [u8; 7];
    pub fn pam_regions(&self) -> [PamRegion; 13];
    pub fn pam_region_for_addr(&self, phys: u32) -> Option<PamRegion>;
}
```

`pam_regions` recomputes from the register file on every call, so a host that
refreshes after any configuration write always sees current attributes; there
is no change notification to subscribe to.

## Model choices

- Reserved bits are masked off on write and therefore read back as zero rather
  than storing what the guest wrote. Real silicon may latch them; a guest cannot
  distinguish this from hardwired zero without relying on undefined behavior.
- The decoded regions are reported in ascending address order rather than in
  register order, so a memory-attribute loop can walk them directly.

## Not implemented

- Any actual memory remapping. Programming PAM changes this register file and
  the accessor output only, so BIOS shadowing has no effect until the machine
  layer applies `pam_regions` to its physical memory model.
- The datasheet notes the attributes "apply to both CPU accesses and PCI
  initiator accesses"; nothing here distinguishes an initiator.
- FDHC (`0x68`, the `080000-09FFFFh` DRAM hole) and SMRAM (`0x72`) are plain
  read/write configuration bytes with no decode, so the rest of the PMC legacy
  memory map is still unmodelled.
- Cacheability is an MTRR concern on this chipset and is out of scope, which is
  why the 4-bit fields have no cache-enable bit.
