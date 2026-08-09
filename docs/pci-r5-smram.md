# i440FX PMC SMRAM Control (`0x72`)

Milestone 2 round 5. Decodes the System Management RAM Control register on the
host bridge at `00:00.0` and exposes a PAM-style host accessor. No `PhysMem`
wiring lives in this crate.

## Spec

Intel 440FX PCIset (82441FX PMC) datasheet, order 290549-001, May 1996:

- §3.2.23 "SMRAM — System Management RAM Control Register" — offset `72h`,
  default `02h`, attribute R/W.
- Table 4 "SMRAM Space Cycles" — which CPU code/data references hit DRAM vs PCI.

Bit names below use the modern `D_OPEN` / `D_CLS` / `D_LCK` / `G_SMRAME` /
`C_BASE_SEG` spellings; the datasheet labels the same fields DOPEN / DCLS /
DLCK / SMRAME / DBASESEG.

| Bit | Name | Meaning |
|---|---|---|
| 7 | Reserved | Reads 0 |
| 6 | D_OPEN | SMM DRAM visible outside SMM (cleared when D_LCK sets) |
| 5 | D_CLS | Data references closed; code may still hit DRAM in SMM |
| 4 | D_LCK | Sticky until power-on reset; forces D_OPEN=0 |
| 3 | G_SMRAME | Global SMRAM enable (128 KiB at `A0000h` when base-seg compatible) |
| 2:0 | C_BASE_SEG | `010b` → `A0000h`–`BFFFFh`; other encodings reserved |

## Host accessor

```rust
pub struct SmramRegion {
    pub start: u32,          // 0xA0000
    pub end: u32,            // 0xBFFFF
    pub g_smrame: bool,
    pub d_open: bool,
    pub d_cls: bool,
    pub d_lck: bool,
    pub c_base_seg: u8,
    pub code_to_dram: bool,  // Table 4 for the supplied in_smm
    pub data_to_dram: bool,
}

impl PciConfig {
    pub fn smram_register(&self) -> u8;
    pub fn set_smram_register(&mut self, value: u8) -> u8;
    pub fn smram_region(&self, in_smm: bool) -> SmramRegion;
    pub fn smram_config_write_overlaps(&self, port: u16, size: u8) -> bool;
}
```

`smram_region` recomputes from the register file. `D_OPEN ∧ D_CLS` is Table 4
INVALID and is reported as PCI for both code and data.

## Exact PhysMem / Machine APIs (wired at integration)

```rust
impl PhysMem {
    pub fn apply_smram(&mut self, region: SmramRegion, in_smm: bool);
}
impl Machine {
    pub fn sync_smram_to_memory(&mut self, in_smm: bool);
}
// MachineBus CONFIG_DATA write path (alongside sync_pam_registers_to_memory):
if self.pci.smram_config_write_overlaps(port, size) {
    self.sync_smram_to_memory(/* current CPU SMM state; false until SMM */);
}
```

SMM mode itself remains unimplemented (no real SMM entry); config writes sync
with `in_smm = false`.
