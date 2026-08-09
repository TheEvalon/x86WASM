# i440FX PMC Fixed DRAM Hole Control (`0x68`)

Milestone 2 round 5. Decodes FDHC on the host bridge and exposes a host
accessor. No `PhysMem` wiring lives in this crate.

## Spec

Intel 440FX PCIset (82441FX PMC) datasheet, order 290549-001, May 1996,
§3.2.20 "FDHC — Fixed DRAM Hole Control Register" — offset `68h`, default
`00h`, attribute R/W.

| Bits | Name | Encoding |
|---|---|---|
| 7:6 | HEN | `00` none; `01` 512 KB–640 KB (`080000h`–`09FFFFh`); `10` 15 MB–16 MB; `11` reserved |
| 5:0 | Reserved | Reads 0 |

CPU cycles matching an enabled hole are forwarded to PCI; the hole is not
remapped. PCI cycles matching the hole are ignored by the PMC.

## Host accessor

```rust
pub struct FdhcHole {
    pub start: u32,
    pub end: u32,
    pub hen: u8,
}

impl PciConfig {
    pub fn fdhc_register(&self) -> u8;
    pub fn set_fdhc_register(&mut self, value: u8) -> u8;
    pub fn fdhc_hen(&self) -> u8;
    pub fn fdhc_hole(&self) -> Option<FdhcHole>;
    pub fn fdhc_config_write_overlaps(&self, port: u16, size: u8) -> bool;
}
```

## Exact PhysMem / Machine APIs needed to wire

```rust
impl PhysMem {
    /// When `Some(hole)`, CPU accesses in [start,end] must not hit DRAM
    /// (forward to PCI / open bus). When `None`, restore normal DRAM decode
    /// for both possible hole ranges.
    pub fn apply_fdhc_hole(&mut self, hole: Option<FdhcHole>);
}

impl Machine {
    pub fn sync_fdhc_to_memory(&mut self) {
        self.mem.apply_fdhc_hole(self.pci.fdhc_hole());
    }
}

// MachineBus CONFIG_DATA write path:
if self.pci.fdhc_config_write_overlaps(port, size) {
    self.sync_fdhc_to_memory();
}
```

## Not implemented

- Actual hole effect on physical memory (accessor only).
- Remapping of the hole elsewhere (datasheet: not remapped).
