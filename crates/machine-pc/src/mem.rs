//! Physical RAM + ROM window (+ A20 gate mask).

use devices::{FdhcHole, SmramRegion};

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
    /// PIIX XBCS bit2 inverted: when true, BIOSCS# is not asserted for writes
    /// (Intel 82371AB §4.1.9). ROM content is still never stored.
    bios_write_protect: bool,
    /// Compatible SMRAM window steering (440FX §3.2.23 Table 4).
    smram: Option<SmramSteer>,
    /// Shadow store for SMRAM when the configured RAM ends below `A0000h`.
    smram_shadow: Vec<u8>,
    /// Fixed DRAM hole inclusive range when FDHC HEN enables one (440FX §3.2.20).
    fdhc_hole: Option<(u32, u32)>,
}

/// Cached SMRAM window decode for PhysMem / MachineBus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SmramSteer {
    start: u32,
    end: u32,
    code_to_dram: bool,
    data_to_dram: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemError {
    OutOfRange,
    /// A write resolved to a ROM window. **No longer returned by
    /// [`PhysMem::write_u8`]** — see [`WriteDisposition::DroppedRom`] and
    /// `docs/machine-r4-write-semantics.md`. Retained so a host that wants the
    /// old diagnostic distinction has a name for it.
    RomWrite,
}

/// What the memory model did with a write. Diagnostics only: every `Dropped*`
/// variant is architecturally identical to the processor, which sees a normal
/// bus completion in all three cases.
///
/// Spec: PCI Local Bus Specification Revision 3.0 §3.2.2.3.4 (Master-Abort:
/// write data discarded, reads all ones, no error to the initiator unless
/// error reporting is enabled); Intel 440FX 82441FX (PMC) §3.2.18 (PAM WE);
/// Intel SDM Vol. 3 §6.15 (`#GP` sources for a data write — none of them is a
/// bus response).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteDisposition {
    /// Landed in DRAM, or in the shadow DRAM behind a PAM region with WE set.
    Accepted,
    /// The address decoded to a ROM window. The ROM does not accept write
    /// cycles, so the data is discarded.
    DroppedRom,
    /// Inside the PAM window with WE clear: the DRAM controller forwards the
    /// write to PCI, where nothing claims it.
    DroppedPamWriteDisabled,
    /// No RAM, no ROM, no device: PCI Master-Abort, data discarded.
    DroppedUnclaimed,
}

impl WriteDisposition {
    /// Whether the write reached storage.
    pub fn accepted(self) -> bool {
        self == Self::Accepted
    }
}

impl PhysMem {
    pub fn new(ram_size: usize) -> Self {
        Self {
            ram: vec![0; ram_size],
            roms: Vec::new(),
            a20_enabled: true,
            pam: [PamAttributes::default(); PAM_REGION_COUNT],
            legacy_shadow: Vec::new(),
            // Spec: Intel 82371AB XBCS default `03h` — bit2 clear → write protect.
            bios_write_protect: true,
            smram: None,
            smram_shadow: Vec::new(),
            fdhc_hole: None,
        }
    }

    pub fn ram_len(&self) -> usize {
        self.ram.len()
    }

    /// Whether BIOSCS# write-protect is in force (XBCS bit2 clear).
    pub fn bios_write_protect(&self) -> bool {
        self.bios_write_protect
    }

    /// Mirror PIIX XBCS bit2 into the memory model (true = protect / no write CS#).
    pub fn set_bios_write_protect(&mut self, enabled: bool) {
        self.bios_write_protect = enabled;
    }

    /// Apply i440FX SMRAM Table 4 steering for the compatible `A0000h`–`BFFFFh` window.
    ///
    /// Spec: Intel 440FX 82441FX (PMC) §3.2.23. `in_smm` is already baked into
    /// `region.code_to_dram` / `region.data_to_dram` by [`PciConfig::smram_region`].
    /// When both are false the window returns to VGA/PCI overlay ownership.
    pub fn apply_smram(&mut self, region: SmramRegion, _in_smm: bool) {
        if region.code_to_dram || region.data_to_dram {
            self.smram = Some(SmramSteer {
                start: region.start,
                end: region.end,
                code_to_dram: region.code_to_dram,
                data_to_dram: region.data_to_dram,
            });
        } else {
            self.smram = None;
        }
    }

    /// Apply i440FX Fixed DRAM Hole Control.
    ///
    /// Spec: Intel 440FX 82441FX (PMC) §3.2.20 — CPU cycles matching an enabled
    /// hole forward to PCI (open bus here); `None` restores normal DRAM decode.
    pub fn apply_fdhc_hole(&mut self, hole: Option<FdhcHole>) {
        self.fdhc_hole = hole.map(|h| (h.start, h.end));
    }

    /// Whether a CPU data reference into the SMRAM window maps to DRAM.
    pub fn smram_steers_read_to_dram(&self, addr: u64) -> bool {
        self.smram_contains(addr) && self.smram.is_some_and(|s| s.data_to_dram || s.code_to_dram)
    }

    /// Whether a CPU data write into the SMRAM window maps to DRAM.
    pub fn smram_steers_write_to_dram(&self, addr: u64) -> bool {
        self.smram_contains(addr) && self.smram.is_some_and(|s| s.data_to_dram)
    }

    fn smram_contains(&self, addr: u64) -> bool {
        self.smram
            .is_some_and(|s| addr as u32 >= s.start && addr as u32 <= s.end)
    }

    fn in_fdhc_hole(&self, addr: u64) -> bool {
        self.fdhc_hole
            .is_some_and(|(start, end)| addr as u32 >= start && addr as u32 <= end)
    }

    fn smram_read(&self, addr: u64) -> u8 {
        let i = addr as usize;
        if i < self.ram.len() {
            return self.ram[i];
        }
        let Some(s) = self.smram else {
            return 0xFF;
        };
        let off = (addr as u32).wrapping_sub(s.start) as usize;
        self.smram_shadow.get(off).copied().unwrap_or(0)
    }

    fn smram_write(&mut self, addr: u64, val: u8) {
        let i = addr as usize;
        if i < self.ram.len() {
            self.ram[i] = val;
            return;
        }
        let Some(s) = self.smram else {
            return;
        };
        let len = (s.end - s.start + 1) as usize;
        if self.smram_shadow.is_empty() {
            self.smram_shadow = vec![0; len];
        }
        let off = (addr as u32).wrapping_sub(s.start) as usize;
        if let Some(slot) = self.smram_shadow.get_mut(off) {
            *slot = val;
        }
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
        if self.in_fdhc_hole(addr) {
            return false;
        }
        if self.smram_steers_read_to_dram(addr) {
            return true;
        }
        if let Some(region) = Self::pam_region_index(addr) {
            if self.pam[region].read == PamRead::ShadowRam {
                return true;
            }
        }
        self.rom_read(addr).is_some() || (addr as usize) < self.ram.len()
    }

    pub fn read_u8(&self, addr: u64) -> Result<u8, MemError> {
        let addr = self.apply_a20(addr);
        // Spec: 440FX §3.2.20 — hole cycles forward to PCI (open bus = 0xFF).
        if self.in_fdhc_hole(addr) {
            return Ok(0xFF);
        }
        // Spec: 440FX §3.2.23 Table 4 — SMRAM DRAM path (shadow when no RAM).
        if self.smram_steers_read_to_dram(addr) {
            return Ok(self.smram_read(addr));
        }
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

    /// Write a byte, never failing, and report where it went.
    ///
    /// A write this platform cannot store is **dropped**, not faulted, in all
    /// three cases: a ROM window, a PAM region with WE clear, and unclaimed
    /// physical space. The returned [`WriteDisposition`] is a diagnostic for
    /// the host; the guest cannot tell the cases apart, which is the point.
    ///
    /// Spec: PCI Local Bus Specification Revision 3.0 §3.2.2.3.4 (Master-Abort
    /// discards write data and reports nothing to the processor); Intel 440FX
    /// 82441FX (PMC) §3.2.18 (PAM WE forwards the write off DRAM); Intel
    /// 82371SB (PIIX3) / 82371AB §4.1.9 XBCS (BIOSCS# is not asserted for a
    /// write unless BIOS write-protect enable bit2 is set, so the ROM never
    /// claims one when protect is in force; with protect lifted the cycle
    /// reaches a mask ROM / flash that still stores nothing); Intel SDM
    /// Vol. 3 §6.15 (the processor's `#GP` sources for a store are
    /// segmentation and paging, not a bus response). See
    /// `docs/machine-r4-write-semantics.md` and `docs/machine-r5-xbcs.md`.
    pub fn write_u8_classified(&mut self, addr: u64, val: u8) -> WriteDisposition {
        let addr = self.apply_a20(addr);
        // Spec: 440FX §3.2.20 — hole cycles forward to PCI (dropped write).
        if self.in_fdhc_hole(addr) {
            return WriteDisposition::DroppedUnclaimed;
        }
        // Spec: 440FX §3.2.23 Table 4 — SMRAM DRAM path.
        if self.smram_steers_write_to_dram(addr) {
            self.smram_write(addr, val);
            return WriteDisposition::Accepted;
        }
        if let Some(region) = Self::pam_region_index(addr) {
            if self.pam[region].write == PamWrite::ShadowRam {
                self.shadow_write(addr, val);
                return WriteDisposition::Accepted;
            }
            return WriteDisposition::DroppedPamWriteDisabled;
        }
        if self.rom_read(addr).is_some() {
            // Spec: Intel 82371AB §4.1.9 — XBCS bit2 gates BIOSCS# on writes.
            // [`Self::bios_write_protect`] mirrors that bit; ROM image bytes are
            // never mutated in either setting (see `docs/machine-r5-xbcs.md`).
            return WriteDisposition::DroppedRom;
        }
        let i = addr as usize;
        if i < self.ram.len() {
            self.ram[i] = val;
            WriteDisposition::Accepted
        } else {
            WriteDisposition::DroppedUnclaimed
        }
    }

    /// [`Self::write_u8_classified`] for callers that do not want the
    /// disposition. Always `Ok(())`: no write raises a bus error on this
    /// platform.
    pub fn write_u8(&mut self, addr: u64, val: u8) -> Result<(), MemError> {
        self.write_u8_classified(addr, val);
        Ok(())
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
        // Spec: the ROM does not accept the cycle and nothing else claims it,
        // so the data is discarded and the processor sees a normal completion.
        assert_eq!(m.write_u8(0, 0x00), Ok(()));
        assert_eq!(m.read_u8(0).unwrap(), 0xF4);
    }

    /// The whole point of the round-4 write-semantics slice: **all three** ways
    /// a write can fail to reach storage behave identically to the processor.
    ///
    /// Spec: PCI Local Bus Specification Revision 3.0 §3.2.2.3.4 — an
    /// unclaimed cycle terminates with Master-Abort, write data discarded, no
    /// error signalled to the initiator unless SERR#/PERR# reporting is armed
    /// (it is not, at reset). Intel 440FX 82441FX (PMC) §3.2.18 — a PAM region
    /// with WE clear forwards the write to PCI. Intel SDM Vol. 3 §6.15 — the
    /// processor has no exception for a store the platform declines; `#GP` for
    /// a data write comes from segmentation, not from the bus.
    #[test]
    fn every_dropped_write_case_completes_and_they_are_distinguishable() {
        let mut m = PhysMem::new(1024 * 1024);
        m.add_rom(0xFFFF_0000, vec![0xAA; 64 * 1024]);
        m.add_rom(0x000F_0000, vec![0xAA; 64 * 1024]);

        // 1. Mapped ROM outside the PAM window (top-of-4 GiB reset alias).
        assert_eq!(m.write_u8(0xFFFF_0000, 0x5A), Ok(()));
        assert_eq!(
            m.write_u8_classified(0xFFFF_0000, 0x5A),
            WriteDisposition::DroppedRom
        );
        assert_eq!(m.read_u8(0xFFFF_0000).unwrap(), 0xAA);

        // 2. PAM region with WE clear (the reset attribute).
        assert_eq!(m.write_u8(0x000F_0000, 0x5A), Ok(()));
        assert_eq!(
            m.write_u8_classified(0x000F_0000, 0x5A),
            WriteDisposition::DroppedPamWriteDisabled
        );
        assert_eq!(m.read_u8(0x000F_0000).unwrap(), 0xAA);

        // 3. Unclaimed physical space (no RAM, no ROM, no device).
        assert_eq!(m.write_u8(0xF000_0000, 0x5A), Ok(()));
        assert_eq!(
            m.write_u8_classified(0xF000_0000, 0x5A),
            WriteDisposition::DroppedUnclaimed
        );
        assert_eq!(m.read_u8(0xF000_0000).unwrap(), 0xFF);

        // And a write that does land is reported as such.
        assert_eq!(
            m.write_u8_classified(0x0000_1000, 0x5A),
            WriteDisposition::Accepted
        );
        assert_eq!(m.read_u8(0x0000_1000).unwrap(), 0x5A);
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
        // Outside the PAM window the write is declined by the ROM rather than
        // by the DRAM controller, but the processor sees the same thing.
        assert_eq!(m.write_u8(0xFFFF_0000, 0x00), Ok(()));
        assert_eq!(m.read_u8(0xFFFF_0000).unwrap(), 0xAA);
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

    /// The PAM window is below 1 MiB only: re-attributing the `0xF0000` alias
    /// does not make the top-of-4 GiB window writable, and a write there is
    /// dropped by the ROM rather than shadowed.
    #[test]
    fn pam_does_not_touch_high_rom_window() {
        let mut m = PhysMem::new(1024 * 1024);
        m.add_rom(0xFFFF_0000, vec![0xAA; 64 * 1024]);
        m.set_region_attributes(PAM_BIOS_REGION, PamRead::ShadowRam, PamWrite::ShadowRam);
        assert_eq!(m.read_u8(0xFFFF_0000).unwrap(), 0xAA);
        assert_eq!(
            m.write_u8_classified(0xFFFF_0000, 0x00),
            WriteDisposition::DroppedRom
        );
        assert_eq!(m.read_u8(0xFFFF_0000).unwrap(), 0xAA);
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
