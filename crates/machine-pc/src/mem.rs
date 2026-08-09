//! Physical RAM + ROM window (+ A20 gate mask).

/// Physical address bit 20 — cleared when the A20 gate is disabled (IBM PC AT).
const A20_ADDR_BIT: u64 = 1 << 20;

/// Base of the PAM-attributed legacy region (`0xC0000`).
///
/// Spec: Intel 440FX PMC datasheet, Programmable Attribute Map (PAM0-PAM6).
pub const PAM_WINDOW_BASE: u64 = 0x000C_0000;

/// End (exclusive) of the PAM-attributed legacy region (`0x100000`).
pub const PAM_WINDOW_END: u64 = 0x0010_0000;

/// Independently attributed PAM regions: twelve 16 KiB + one 64 KiB.
pub const PAM_REGION_COUNT: usize = 13;

/// Index of the `0xF0000`-`0xFFFFF` BIOS area region (PAM0 bits 7:4).
pub const PAM_BIOS_REGION: usize = 12;

/// First PAM configuration register offset in PCI config space (PAM0).
pub const PAM_REGISTER_FIRST: u8 = 0x59;

/// Last PAM configuration register offset in PCI config space (PAM6).
pub const PAM_REGISTER_LAST: u8 = 0x5F;

/// Attribute-field bit 0 - RE: reads are directed to DRAM when set.
pub const PAM_FIELD_RE: u8 = 1 << 0;

/// Attribute-field bit 1 - WE: writes are directed to DRAM when set.
pub const PAM_FIELD_WE: u8 = 1 << 1;

/// Attribute-field bits decoded (bits 3:2 of each nibble are reserved).
pub const PAM_FIELD_MASK: u8 = PAM_FIELD_RE | PAM_FIELD_WE;

/// `(base, length)` of each PAM region, in ascending address order.
///
/// Spec: Intel 440FX PMC - PAM1-PAM6 attribute twelve 16 KiB regions from
/// `0xC0000` to `0xEFFFF`; PAM0 bits 7:4 attribute the 64 KiB BIOS area.
pub const PAM_REGIONS: [(u64, u64); PAM_REGION_COUNT] = [
    (0x000C_0000, 16 * 1024),
    (0x000C_4000, 16 * 1024),
    (0x000C_8000, 16 * 1024),
    (0x000C_C000, 16 * 1024),
    (0x000D_0000, 16 * 1024),
    (0x000D_4000, 16 * 1024),
    (0x000D_8000, 16 * 1024),
    (0x000D_C000, 16 * 1024),
    (0x000E_0000, 16 * 1024),
    (0x000E_4000, 16 * 1024),
    (0x000E_8000, 16 * 1024),
    (0x000E_C000, 16 * 1024),
    (0x000F_0000, 64 * 1024),
];

/// Where reads of a PAM region are satisfied from.
///
/// Spec: Intel 440FX PMC - RE clear forwards the read to PCI (the ROM window);
/// RE set directs it to shadow DRAM.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PamRead {
    /// Read the ROM window covering the address. Falls through to the normal
    /// RAM / open-bus decode when no ROM window covers it - a model choice, not
    /// 440FX behavior; see `docs/machine-r2-pam-memory.md`.
    #[default]
    Rom,
    /// Read the shadow DRAM backing store.
    ShadowRam,
}

/// Where writes to a PAM region go.
///
/// Spec: Intel 440FX PMC - WE clear forwards the write to PCI (where nothing
/// claims it, so it is dropped); WE set directs it to shadow DRAM.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PamWrite {
    /// Forwarded off the DRAM controller and dropped (no fault).
    #[default]
    Ignored,
    /// Written to the shadow DRAM backing store.
    ShadowRam,
}

/// One region's attribute pair.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PamAttributes {
    pub read: PamRead,
    pub write: PamWrite,
}

impl PamAttributes {
    /// Decode one 4-bit PAM attribute field (bits 3:2 reserved).
    pub fn from_field(field: u8) -> Self {
        Self {
            read: if field & PAM_FIELD_RE != 0 {
                PamRead::ShadowRam
            } else {
                PamRead::Rom
            },
            write: if field & PAM_FIELD_WE != 0 {
                PamWrite::ShadowRam
            } else {
                PamWrite::Ignored
            },
        }
    }

    /// Encode back into a 4-bit PAM attribute field.
    pub fn to_field(self) -> u8 {
        let mut field = 0;
        if self.read == PamRead::ShadowRam {
            field |= PAM_FIELD_RE;
        }
        if self.write == PamWrite::ShadowRam {
            field |= PAM_FIELD_WE;
        }
        field
    }
}

#[derive(Clone, Debug)]
pub struct RomWindow {
    pub base: u64,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct PhysMem {
    ram: Vec<u8>,
    /// One or more ROM windows (e.g. BIOS high map + below-1 MiB alias).
    roms: Vec<RomWindow>,
    /// A20 gate: when false, physical bit 20 is forced clear (IBM PC AT).
    /// Power-on / reset default is enabled (open gate).
    a20_enabled: bool,
    /// Per-region legacy-window attributes (i440FX PAM).
    pam: [PamAttributes; PAM_REGION_COUNT],
    /// Shadow store for legacy-window addresses with no DRAM behind them.
    /// Allocated on first use; addresses inside `ram` shadow into `ram`.
    legacy_shadow: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemError {
    OutOfRange,
    RomWrite,
}

impl PhysMem {
    pub fn new(ram_size: usize) -> Self {
        Self {
            ram: vec![0; ram_size],
            roms: Vec::new(),
            a20_enabled: true,
            pam: [PamAttributes::default(); PAM_REGION_COUNT],
            legacy_shadow: Vec::new(),
        }
    }

    pub fn ram_len(&self) -> usize {
        self.ram.len()
    }

    pub fn a20_enabled(&self) -> bool {
        self.a20_enabled
    }

    /// Set A20 gate. Spec: IBM PC AT — gate disabled masks physical A20.
    pub fn set_a20_enabled(&mut self, enabled: bool) {
        self.a20_enabled = enabled;
    }

    /// Apply A20 mask to a physical address before RAM/ROM decode.
    fn apply_a20(&self, addr: u64) -> u64 {
        if self.a20_enabled {
            addr
        } else {
            addr & !A20_ADDR_BIT
        }
    }

    /// Replace all ROM windows with a single mapping (HELLO / lab ROMs).
    pub fn map_rom(&mut self, base: u64, data: Vec<u8>) {
        self.roms.clear();
        self.roms.push(RomWindow { base, data });
    }

    /// Clear every ROM window.
    pub fn clear_roms(&mut self) {
        self.roms.clear();
    }

    /// Append a ROM window without clearing existing ones (BIOS dual-map).
    pub fn add_rom(&mut self, base: u64, data: Vec<u8>) {
        self.roms.push(RomWindow { base, data });
    }

    fn rom_read(&self, addr: u64) -> Option<u8> {
        for rom in &self.roms {
            if addr < rom.base {
                continue;
            }
            let off = (addr - rom.base) as usize;
            if off < rom.data.len() {
                return Some(rom.data[off]);
            }
        }
        None
    }

    /// PAM region containing an already-A20-masked physical address.
    pub fn pam_region_index(addr: u64) -> Option<usize> {
        if !(PAM_WINDOW_BASE..PAM_WINDOW_END).contains(&addr) {
            return None;
        }
        PAM_REGIONS
            .iter()
            .position(|(base, len)| addr >= *base && addr < base + len)
    }

    /// `(base, length)` of a PAM region.
    pub fn pam_region_range(region: usize) -> Option<(u64, u64)> {
        PAM_REGIONS.get(region).copied()
    }

    /// Region attributed by one nibble of a PAM configuration register.
    ///
    /// Spec: Intel 440FX PMC - PAM0 (`0x59`) low nibble is reserved and its
    /// high nibble owns the BIOS area; PAM1-PAM6 (`0x5A`-`0x5F`) own the twelve
    /// 16 KiB regions in ascending address order, low nibble first.
    pub fn pam_region_for_register(offset: u8, high_nibble: bool) -> Option<usize> {
        if !(PAM_REGISTER_FIRST..=PAM_REGISTER_LAST).contains(&offset) {
            return None;
        }
        if offset == PAM_REGISTER_FIRST {
            return high_nibble.then_some(PAM_BIOS_REGION);
        }
        let pair = usize::from(offset - PAM_REGISTER_FIRST - 1);
        Some(pair * 2 + usize::from(high_nibble))
    }

    /// Current attributes of a PAM region.
    pub fn region_attributes(&self, region: usize) -> Option<PamAttributes> {
        self.pam.get(region).copied()
    }

    /// Host entry point for a PCI-side PAM caller: set one region's attributes.
    ///
    /// Returns `false` for an out-of-range region index. Spec: Intel 440FX PMC
    /// Programmable Attribute Map (RE / WE per region).
    pub fn set_region_attributes(
        &mut self,
        region: usize,
        readable_from: PamRead,
        writable_to: PamWrite,
    ) -> bool {
        match self.pam.get_mut(region) {
            Some(slot) => {
                *slot = PamAttributes {
                    read: readable_from,
                    write: writable_to,
                };
                true
            }
            None => false,
        }
    }

    /// Apply one i440FX PAM configuration register byte (`0x59`-`0x5F`).
    ///
    /// The low nibble drives the lower-addressed region of the pair and the
    /// high nibble the higher-addressed one; PAM0's low nibble is reserved and
    /// ignored. Returns `false` when `offset` is not a PAM register.
    pub fn apply_pam_register(&mut self, offset: u8, value: u8) -> bool {
        if !(PAM_REGISTER_FIRST..=PAM_REGISTER_LAST).contains(&offset) {
            return false;
        }
        for high in [false, true] {
            let Some(region) = Self::pam_region_for_register(offset, high) else {
                continue;
            };
            let field = if high { value >> 4 } else { value } & PAM_FIELD_MASK;
            self.pam[region] = PamAttributes::from_field(field);
        }
        true
    }

    /// Re-encode the attributes a PAM configuration register currently holds.
    ///
    /// Reserved bits read back as zero, so this is the decoded view rather than
    /// a byte-exact register file (the PCI side owns the stored byte).
    pub fn pam_register_value(&self, offset: u8) -> Option<u8> {
        if !(PAM_REGISTER_FIRST..=PAM_REGISTER_LAST).contains(&offset) {
            return None;
        }
        let mut value = 0u8;
        for high in [false, true] {
            if let Some(region) = Self::pam_region_for_register(offset, high) {
                let field = self.pam[region].to_field();
                value |= if high { field << 4 } else { field };
            }
        }
        Some(value)
    }

    /// Restore the PAM reset defaults: every region reads ROM, writes dropped.
    ///
    /// Spec: Intel 440FX PMC - PAM0-PAM6 reset to `0x00`. Does not clear the
    /// shadow contents (DRAM keeps its bits across a chipset attribute reset).
    pub fn reset_pam(&mut self) {
        self.pam = [PamAttributes::default(); PAM_REGION_COUNT];
    }

    /// Shadow-DRAM read for a legacy-window address.
    fn shadow_read(&self, addr: u64) -> u8 {
        let i = addr as usize;
        if i < self.ram.len() {
            return self.ram[i];
        }
        self.legacy_shadow
            .get((addr - PAM_WINDOW_BASE) as usize)
            .copied()
            .unwrap_or(0)
    }

    /// Shadow-DRAM write for a legacy-window address.
    fn shadow_write(&mut self, addr: u64, val: u8) {
        let i = addr as usize;
        if i < self.ram.len() {
            self.ram[i] = val;
            return;
        }
        if self.legacy_shadow.is_empty() {
            self.legacy_shadow = vec![0; (PAM_WINDOW_END - PAM_WINDOW_BASE) as usize];
        }
        self.legacy_shadow[(addr - PAM_WINDOW_BASE) as usize] = val;
    }

    /// Whether an A20-masked physical address decodes to RAM or a ROM window.
    ///
    /// Everything else is open bus (reads return `0xFF`, writes are dropped);
    /// the POST probe uses this to report unimplemented MMIO regions.
    pub fn is_mapped(&self, addr: u64) -> bool {
        let addr = self.apply_a20(addr);
        if let Some(region) = Self::pam_region_index(addr) {
            if self.pam[region].read == PamRead::ShadowRam {
                return true;
            }
        }
        self.rom_read(addr).is_some() || (addr as usize) < self.ram.len()
    }

    pub fn read_u8(&self, addr: u64) -> Result<u8, MemError> {
        let addr = self.apply_a20(addr);
        if let Some(region) = Self::pam_region_index(addr) {
            if self.pam[region].read == PamRead::ShadowRam {
                return Ok(self.shadow_read(addr));
            }
        }
        if let Some(b) = self.rom_read(addr) {
            return Ok(b);
        }
        let i = addr as usize;
        if i < self.ram.len() {
            Ok(self.ram[i])
        } else {
            // Open bus for unmapped high addresses outside ROM: return 0xFF
            Ok(0xFF)
        }
    }

    pub fn write_u8(&mut self, addr: u64, val: u8) -> Result<(), MemError> {
        let addr = self.apply_a20(addr);
        // Spec: Intel 440FX PMC - inside the PAM window the DRAM controller
        // either accepts the write (WE set) or forwards it to PCI, where
        // nothing claims it. Neither case is an error to the CPU, so the
        // `RomWrite` diagnostic applies only outside this window.
        if let Some(region) = Self::pam_region_index(addr) {
            if self.pam[region].write == PamWrite::ShadowRam {
                self.shadow_write(addr, val);
            }
            return Ok(());
        }
        if self.rom_read(addr).is_some() {
            return Err(MemError::RomWrite);
        }
        let i = addr as usize;
        if i < self.ram.len() {
            self.ram[i] = val;
            Ok(())
        } else {
            // Ignore writes to unmapped space (MMIO stub).
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rom_overrides_ram() {
        let mut m = PhysMem::new(1024);
        m.ram[0] = 0x11;
        m.map_rom(0, vec![0xF4]);
        assert_eq!(m.read_u8(0).unwrap(), 0xF4);
        assert_eq!(m.write_u8(0, 0x00), Err(MemError::RomWrite));
    }

    /// Spec: machine-model — BIOS may appear at high map and `0xF0000` alias.
    #[test]
    fn dual_rom_windows_high_and_low_alias() {
        let mut m = PhysMem::new(1024 * 1024);
        m.add_rom(0xFFFF_0000, vec![0xAA; 4]);
        m.add_rom(0x000F_0000, vec![0xBB; 4]);
        assert_eq!(m.read_u8(0xFFFF_0000).unwrap(), 0xAA);
        assert_eq!(m.read_u8(0x000F_0000).unwrap(), 0xBB);
        // Spec: Intel 440FX PMC - the low alias sits in PAM region
        // `PAM_BIOS_REGION`, whose reset attributes drop writes on the PCI bus
        // instead of faulting; the ROM contents stay visible.
        assert_eq!(m.write_u8(0x000F_0001, 0x00), Ok(()));
        assert_eq!(m.read_u8(0x000F_0001).unwrap(), 0xBB);
        // Outside the PAM window a ROM write still reports the diagnostic.
        assert_eq!(m.write_u8(0xFFFF_0000, 0x00), Err(MemError::RomWrite));
    }

    /// Spec: Intel 440FX PMC §3.2.19 (PAM0–PAM6, config `0x59`–`0x5F`) —
    /// thirteen attribute regions: `0xF0000`–`0xFFFFF` (PAM0 bits 7:4) and
    /// twelve 16 KiB regions covering `0xC0000`–`0xEFFFF`.
    #[test]
    fn pam_region_table_covers_c0000_to_fffff() {
        assert_eq!(PAM_REGIONS.len(), PAM_REGION_COUNT);
        let mut expect = PAM_WINDOW_BASE;
        for (index, (base, len)) in PAM_REGIONS.iter().enumerate() {
            assert_eq!(*base, expect, "region {index} base");
            let expected_len = if index == PAM_BIOS_REGION {
                64 * 1024
            } else {
                16 * 1024
            };
            assert_eq!(*len, expected_len, "region {index} length");
            expect += *len;
        }
        assert_eq!(expect, PAM_WINDOW_END);

        assert_eq!(PhysMem::pam_region_index(0x000B_FFFF), None);
        assert_eq!(PhysMem::pam_region_index(0x000C_0000), Some(0));
        assert_eq!(PhysMem::pam_region_index(0x000C_3FFF), Some(0));
        assert_eq!(PhysMem::pam_region_index(0x000C_4000), Some(1));
        assert_eq!(PhysMem::pam_region_index(0x000E_C000), Some(11));
        assert_eq!(
            PhysMem::pam_region_index(0x000F_0000),
            Some(PAM_BIOS_REGION)
        );
        assert_eq!(
            PhysMem::pam_region_index(0x000F_FFFF),
            Some(PAM_BIOS_REGION)
        );
        assert_eq!(PhysMem::pam_region_index(0x0010_0000), None);
    }

    /// Spec: Intel 440FX PMC — PAM0 low nibble is reserved; each remaining
    /// nibble owns one region, low nibble first, ascending from `0xC0000`.
    #[test]
    fn pam_register_nibbles_map_to_regions() {
        assert_eq!(PhysMem::pam_region_for_register(0x59, false), None);
        assert_eq!(
            PhysMem::pam_region_for_register(0x59, true),
            Some(PAM_BIOS_REGION)
        );
        assert_eq!(PhysMem::pam_region_for_register(0x5A, false), Some(0));
        assert_eq!(PhysMem::pam_region_for_register(0x5A, true), Some(1));
        assert_eq!(PhysMem::pam_region_for_register(0x5F, false), Some(10));
        assert_eq!(PhysMem::pam_region_for_register(0x5F, true), Some(11));
        assert_eq!(PhysMem::pam_region_for_register(0x58, false), None);
        assert_eq!(PhysMem::pam_region_for_register(0x60, true), None);
    }

    /// Spec: Intel 440FX PMC — PAM reset value is `0x00`: every region reads
    /// from PCI (the ROM window) and writes are forwarded away from DRAM.
    #[test]
    fn pam_reset_defaults_read_rom_and_disable_writes() {
        let m = PhysMem::new(1024 * 1024);
        for region in 0..PAM_REGION_COUNT {
            assert_eq!(
                m.region_attributes(region),
                Some(PamAttributes {
                    read: PamRead::Rom,
                    write: PamWrite::Ignored,
                }),
                "region {region}"
            );
        }
        for offset in PAM_REGISTER_FIRST..=PAM_REGISTER_LAST {
            assert_eq!(m.pam_register_value(offset), Some(0x00));
        }
    }

    /// Spec: Intel 440FX PMC — attribute field bit0 RE (read from DRAM) and
    /// bit1 WE (write to DRAM), low nibble = lower region of the pair.
    #[test]
    fn pam_register_write_sets_both_nibble_regions() {
        let mut m = PhysMem::new(1024 * 1024);
        // PAM1 (0x5A): low nibble region 0 read-only-from-DRAM, high nibble
        // region 1 write-only-to-DRAM.
        assert!(m.apply_pam_register(0x5A, PAM_FIELD_RE | (PAM_FIELD_WE << 4)));
        assert_eq!(
            m.region_attributes(0),
            Some(PamAttributes {
                read: PamRead::ShadowRam,
                write: PamWrite::Ignored,
            })
        );
        assert_eq!(
            m.region_attributes(1),
            Some(PamAttributes {
                read: PamRead::Rom,
                write: PamWrite::ShadowRam,
            })
        );
        assert_eq!(m.pam_register_value(0x5A), Some(0x21));
        assert!(!m.apply_pam_register(0x58, 0xFF));
    }

    /// SeaBIOS `make_bios_writable()` shape: read ROM + write shadow, copy,
    /// then read shadow + drop writes. Spec: Intel 440FX PMC PAM0 bits 7:4.
    #[test]
    fn pam_bios_region_shadow_copy_then_lock() {
        let mut m = PhysMem::new(1024 * 1024);
        m.add_rom(0x000F_0000, vec![0xAA; 64 * 1024]);

        // Default: ROM reads, writes dropped on the PCI bus (no fault).
        assert_eq!(m.read_u8(0x000F_0000).unwrap(), 0xAA);
        assert_eq!(m.write_u8(0x000F_0000, 0x11), Ok(()));
        assert_eq!(m.read_u8(0x000F_0000).unwrap(), 0xAA);

        // Read ROM / write DRAM: the copy reads the ROM and lands in shadow.
        m.set_region_attributes(PAM_BIOS_REGION, PamRead::Rom, PamWrite::ShadowRam);
        for off in 0..64u64 * 1024 {
            let b = m.read_u8(0x000F_0000 + off).unwrap();
            m.write_u8(0x000F_0000 + off, b).unwrap();
        }
        m.write_u8(0x000F_0010, 0x5A).unwrap();
        // Reads still come from ROM while RE is clear.
        assert_eq!(m.read_u8(0x000F_0010).unwrap(), 0xAA);

        // Read DRAM / writes dropped: the shadow copy is now what executes.
        m.set_region_attributes(PAM_BIOS_REGION, PamRead::ShadowRam, PamWrite::Ignored);
        assert_eq!(m.read_u8(0x000F_0000).unwrap(), 0xAA);
        assert_eq!(m.read_u8(0x000F_0010).unwrap(), 0x5A);
        assert_eq!(m.write_u8(0x000F_0010, 0x99), Ok(()));
        assert_eq!(m.read_u8(0x000F_0010).unwrap(), 0x5A);
    }

    /// Each region is attributed independently; a neighbour keeps reading ROM.
    #[test]
    fn pam_regions_are_independent() {
        let mut m = PhysMem::new(1024 * 1024);
        m.add_rom(0x000C_0000, vec![0x77; 32 * 1024]);
        m.set_region_attributes(0, PamRead::Rom, PamWrite::ShadowRam);
        m.write_u8(0x000C_0000, 0x01).unwrap();
        m.write_u8(0x000C_4000, 0x02).unwrap(); // region 1: dropped
        m.set_region_attributes(0, PamRead::ShadowRam, PamWrite::Ignored);
        m.set_region_attributes(1, PamRead::ShadowRam, PamWrite::Ignored);
        assert_eq!(m.read_u8(0x000C_0000).unwrap(), 0x01);
        assert_eq!(m.read_u8(0x000C_4000).unwrap(), 0x00);
    }

    /// The PAM window is below 1 MiB only: the top-of-4 GiB BIOS window keeps
    /// its ROM semantics, including the `RomWrite` diagnostic.
    #[test]
    fn pam_does_not_touch_high_rom_window() {
        let mut m = PhysMem::new(1024 * 1024);
        m.add_rom(0xFFFF_0000, vec![0xAA; 64 * 1024]);
        m.set_region_attributes(PAM_BIOS_REGION, PamRead::ShadowRam, PamWrite::ShadowRam);
        assert_eq!(m.read_u8(0xFFFF_0000).unwrap(), 0xAA);
        assert_eq!(m.write_u8(0xFFFF_0000, 0x00), Err(MemError::RomWrite));
    }

    /// A machine without DRAM behind the legacy window still shadows, using the
    /// lazily allocated legacy buffer (model choice, documented).
    #[test]
    fn pam_shadow_works_without_dram_backing() {
        let mut m = PhysMem::new(64 * 1024);
        m.set_region_attributes(5, PamRead::ShadowRam, PamWrite::ShadowRam);
        assert_eq!(m.read_u8(0x000D_4000).unwrap(), 0x00);
        m.write_u8(0x000D_4000, 0xC3).unwrap();
        assert_eq!(m.read_u8(0x000D_4000).unwrap(), 0xC3);
        assert!(m.is_mapped(0x000D_4000));
    }

    /// Spec: IBM PC AT A20 gate — masking happens before the PAM decode.
    #[test]
    fn pam_decode_follows_a20_mask() {
        let mut m = PhysMem::new(1024 * 1024);
        m.set_region_attributes(PAM_BIOS_REGION, PamRead::ShadowRam, PamWrite::ShadowRam);
        m.set_a20_enabled(false);
        m.write_u8(0x001F_0000, 0x3C).unwrap();
        m.set_a20_enabled(true);
        assert_eq!(m.read_u8(0x000F_0000).unwrap(), 0x3C);
    }

    /// Spec: IBM PC AT A20 gate — when disabled, phys bit 20 is forced clear.
    #[test]
    fn a20_disabled_aliases_bit20() {
        let mut m = PhysMem::new(2 * 1024 * 1024);
        m.write_u8(0, 0x11).unwrap();
        m.write_u8(A20_ADDR_BIT, 0x22).unwrap();
        assert_eq!(m.read_u8(0).unwrap(), 0x11);
        assert_eq!(m.read_u8(A20_ADDR_BIT).unwrap(), 0x22);

        m.set_a20_enabled(false);
        assert!(!m.a20_enabled());
        // Access at 1 MiB aliases to address 0.
        assert_eq!(m.read_u8(A20_ADDR_BIT).unwrap(), 0x11);
        m.write_u8(A20_ADDR_BIT, 0x33).unwrap();
        assert_eq!(m.read_u8(0).unwrap(), 0x33);

        m.set_a20_enabled(true);
        assert_eq!(m.read_u8(A20_ADDR_BIT).unwrap(), 0x22);
    }
}
