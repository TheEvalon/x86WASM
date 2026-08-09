//! 32-bit paging-structure entries.
//!
//! Spec: Intel SDM Vol. 3 §4.3 "32-Bit Paging", Figure 4-4 (formats of CR3 and
//! the paging-structure entries), Table 4-3 (use of CR3), Table 4-4 (PDE that
//! maps a 4-MByte page), Table 4-5 (PDE that references a page table), and
//! Table 4-6 (PTE that maps a 4-KByte page).

/// Present. SDM Tables 4-4/4-5/4-6, bit 0.
pub const ENTRY_P: u32 = 1 << 0;
/// Read/write. SDM Tables 4-4/4-5/4-6, bit 1.
pub const ENTRY_RW: u32 = 1 << 1;
/// User/supervisor. SDM Tables 4-4/4-5/4-6, bit 2.
pub const ENTRY_US: u32 = 1 << 2;
/// Page-level write-through. SDM Tables 4-4/4-5/4-6, bit 3.
pub const ENTRY_PWT: u32 = 1 << 3;
/// Page-level cache disable. SDM Tables 4-4/4-5/4-6, bit 4.
pub const ENTRY_PCD: u32 = 1 << 4;
/// Accessed. SDM Tables 4-4/4-5/4-6, bit 5; semantics in §4.8.
pub const ENTRY_A: u32 = 1 << 5;
/// Dirty. SDM Tables 4-4/4-6, bit 6; ignored in a PDE that references a page
/// table (Table 4-5 marks bit 6 "Ignored").
pub const ENTRY_D: u32 = 1 << 6;
/// Page size, PDE only. SDM Table 4-4 bit 7 ("must be 1 to map a 4-MByte
/// page"); Table 4-5 bit 7 ("if CR4.PSE = 1, must be 0 ...; otherwise,
/// ignored").
pub const PDE_PS: u32 = 1 << 7;
/// Page-attribute table, PTE only. SDM Table 4-6 bit 7.
pub const PTE_PAT: u32 = 1 << 7;
/// Page-attribute table, 4-MByte PDE only. SDM Table 4-4 bit 12.
pub const PDE_LARGE_PAT: u32 = 1 << 12;
/// Global. SDM Table 4-4 bit 8 / Table 4-6 bit 8; used only if CR4.PGE = 1
/// (§4.10.2.4).
pub const ENTRY_G: u32 = 1 << 8;

/// Bits 31:12 of an entry: the 4-KiB aligned physical address of the next
/// paging structure or of a 4-KiB page frame.
pub const ENTRY_ADDR_4KIB: u32 = 0xFFFF_F000;
/// Bits 31:22 of a 4-MiB PDE: bits 31:22 of the mapped page frame.
pub const PDE_ADDR_4MIB: u32 = 0xFFC0_0000;
/// Bits 20:13 of a 4-MiB PDE: bits 39:32 of the mapped page frame when the
/// PSE-36 mechanism is supported (SDM Table 4-4, row `(M–20):13`).
pub const PDE_ADDR_4MIB_HIGH: u32 = 0x001F_E000;

/// Processor-model inputs that decide which entry bits are reserved and how
/// wide a 4-MiB page frame may be.
///
/// Spec: SDM §4.1.4 "Enumeration of Paging Features by CPUID" — the PAT
/// (`CPUID.01H:EDX.PAT[16]`) and PSE-36 (`CPUID.01H:EDX.PSE-36[17]`) bits, and
/// MAXPHYADDR.
///
/// The default is the profile this emulator's CPUID actually reports: no PAT,
/// no PSE-36, 32 physical address bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PagingProfile {
    /// `CPUID.01H:EDX.PAT[bit 16]`. When false, PTE bit 7 and 4-MiB PDE bit 12
    /// are reserved rather than PAT selectors (SDM §4.3).
    pub pat_supported: bool,
    /// `CPUID.01H:EDX.PSE-36[bit 17]`. When false, a 4-MiB page frame is
    /// limited to 32 physical address bits and PDE bits 21:13 are reserved.
    pub pse36_supported: bool,
    /// MAXPHYADDR (SDM §4.1.4). Only consulted when `pse36_supported`.
    pub maxphyaddr: u8,
}

impl Default for PagingProfile {
    fn default() -> Self {
        Self {
            pat_supported: false,
            pse36_supported: false,
            maxphyaddr: 32,
        }
    }
}

impl PagingProfile {
    /// A profile with the PSE-36 mechanism supported at `maxphyaddr` bits.
    ///
    /// Spec: SDM §4.3 — "If the PSE-36 mechanism is supported, M is the minimum
    /// of 40 and MAXPHYADDR".
    pub fn with_pse36(maxphyaddr: u8) -> Self {
        Self {
            pat_supported: false,
            pse36_supported: true,
            maxphyaddr,
        }
    }

    /// `M` from SDM Table 4-4: the minimum of 40 and MAXPHYADDR when PSE-36 is
    /// supported, and 32 otherwise.
    pub fn large_page_addr_width(&self) -> u32 {
        if self.pse36_supported {
            u32::from(self.maxphyaddr).min(40)
        } else {
            32
        }
    }
}

/// A 32-bit page-directory entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pde(pub u32);

/// A 32-bit page-table entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pte(pub u32);

macro_rules! common_entry_bits {
    ($ty:ty) => {
        impl $ty {
            /// Raw entry value.
            pub fn bits(self) -> u32 {
                self.0
            }

            /// P (bit 0). An entry with P = 0 is used neither to reference
            /// another paging structure nor to map a page (SDM §4.3).
            pub fn present(self) -> bool {
                self.0 & ENTRY_P != 0
            }

            /// R/W (bit 1). SDM §4.6.
            pub fn read_write(self) -> bool {
                self.0 & ENTRY_RW != 0
            }

            /// U/S (bit 2). SDM §4.6.
            pub fn user_supervisor(self) -> bool {
                self.0 & ENTRY_US != 0
            }

            /// PWT (bit 3). Memory typing only (SDM §4.9); inert here.
            pub fn write_through(self) -> bool {
                self.0 & ENTRY_PWT != 0
            }

            /// PCD (bit 4). Memory typing only (SDM §4.9); inert here.
            pub fn cache_disable(self) -> bool {
                self.0 & ENTRY_PCD != 0
            }

            /// A (bit 5). SDM §4.8.
            pub fn accessed(self) -> bool {
                self.0 & ENTRY_A != 0
            }
        }
    };
}

common_entry_bits!(Pde);
common_entry_bits!(Pte);

impl Pde {
    /// PS (bit 7). Only meaningful when `CR4.PSE = 1`; SDM Table 4-5 says the
    /// bit is ignored otherwise, so callers must pass `cr4_pse`.
    pub fn maps_large_page(self, cr4_pse: bool) -> bool {
        cr4_pse && (self.0 & PDE_PS != 0)
    }

    /// D (bit 6) of a PDE that maps a 4-MiB page (SDM Table 4-4). Bit 6 is
    /// ignored in a PDE that references a page table (Table 4-5).
    pub fn dirty(self) -> bool {
        self.0 & ENTRY_D != 0
    }

    /// G (bit 8) of a PDE that maps a 4-MiB page (SDM Table 4-4).
    pub fn global(self) -> bool {
        self.0 & ENTRY_G != 0
    }

    /// Physical address of the referenced 4-KiB aligned page table.
    ///
    /// Spec: SDM §4.3 — "Bits 39:32 are all 0. Bits 31:12 are from the PDE."
    pub fn page_table_base(self) -> u64 {
        u64::from(self.0 & ENTRY_ADDR_4KIB)
    }

    /// Physical base of the mapped 4-MiB page frame.
    ///
    /// Spec: SDM §4.3 / Table 4-4 — bits 39:32 come from PDE bits 20:13 and
    /// bits 31:22 from PDE bits 31:22. The physical-address bits in the PDE are
    /// deliberately not contiguous.
    pub fn large_page_base(self, profile: &PagingProfile) -> u64 {
        let low = u64::from(self.0 & PDE_ADDR_4MIB);
        if profile.pse36_supported {
            let high = u64::from((self.0 & PDE_ADDR_4MIB_HIGH) >> 13);
            (high << 32) | low
        } else {
            low
        }
    }

    /// Bits that are reserved in this PDE, or 0 if none are.
    ///
    /// Spec: SDM §4.3 — "With 32-bit paging, there are reserved bits only if
    /// CR4.PSE = 1", and then only in an entry whose P flag is 1:
    ///
    /// * If the P flag and the PS flag of a PDE are both 1, the bits reserved
    ///   depend on MAXPHYADDR and whether the PSE-36 mechanism is supported: if
    ///   PSE-36 is not supported, bits 21:13 are reserved; if it is supported,
    ///   bits 21:(M–19) are reserved, where M is the minimum of 40 and
    ///   MAXPHYADDR.
    /// * If the PAT is not supported and the P and PS flags of a PDE are both
    ///   1, bit 12 is reserved.
    ///
    /// A PDE that references a page table has no reserved bits (Table 4-5).
    pub fn reserved_bits(self, cr4_pse: bool, profile: &PagingProfile) -> u32 {
        if !self.present() || !self.maps_large_page(cr4_pse) {
            return 0;
        }
        let mut mask = 0u32;
        if !profile.pat_supported {
            mask |= PDE_LARGE_PAT;
        }
        mask |= large_page_reserved_addr_mask(profile);
        self.0 & mask
    }

    /// Convenience: does this PDE set any reserved bit?
    pub fn sets_reserved_bit(self, cr4_pse: bool, profile: &PagingProfile) -> bool {
        self.reserved_bits(cr4_pse, profile) != 0
    }
}

impl Pte {
    /// D (bit 6). SDM Table 4-6.
    pub fn dirty(self) -> bool {
        self.0 & ENTRY_D != 0
    }

    /// G (bit 8). SDM Table 4-6.
    pub fn global(self) -> bool {
        self.0 & ENTRY_G != 0
    }

    /// Physical base of the mapped 4-KiB page frame.
    ///
    /// Spec: SDM §4.3 — "Bits 39:32 are all 0. Bits 31:12 are from the PTE."
    pub fn page_base(self) -> u64 {
        u64::from(self.0 & ENTRY_ADDR_4KIB)
    }

    /// Bits that are reserved in this PTE, or 0 if none are.
    ///
    /// Spec: SDM §4.3 — reserved bits exist only if CR4.PSE = 1; if the PAT is
    /// not supported and the P flag of a PTE is 1, bit 7 is reserved.
    pub fn reserved_bits(self, cr4_pse: bool, profile: &PagingProfile) -> u32 {
        if !self.present() || !cr4_pse || profile.pat_supported {
            return 0;
        }
        self.0 & PTE_PAT
    }

    /// Convenience: does this PTE set any reserved bit?
    pub fn sets_reserved_bit(self, cr4_pse: bool, profile: &PagingProfile) -> bool {
        self.reserved_bits(cr4_pse, profile) != 0
    }
}

/// Reserved address bits of a 4-MiB PDE, from SDM Table 4-4 row
/// "21:(M–19) Reserved (must be 0)".
fn large_page_reserved_addr_mask(profile: &PagingProfile) -> u32 {
    let m = profile.large_page_addr_width();
    // Bits 21:(M-19). With M = 32 (no PSE-36) that is bits 21:13; with M = 40
    // it is bit 21 alone.
    let low = m - 19;
    let mut mask = 0u32;
    for bit in low..=21 {
        mask |= 1 << bit;
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SDM Table 4-6: bits 31:12 are the frame, and PWT/PCD/A/D/G decode where
    /// the table says they do.
    #[test]
    fn pte_field_decode() {
        let pte = Pte(0x0012_3000 | ENTRY_P | ENTRY_RW | ENTRY_US | ENTRY_A | ENTRY_D | ENTRY_G);
        assert!(pte.present());
        assert!(pte.read_write());
        assert!(pte.user_supervisor());
        assert!(pte.accessed());
        assert!(pte.dirty());
        assert!(pte.global());
        assert_eq!(pte.page_base(), 0x0012_3000);
        assert!(!pte.write_through());
        assert!(!pte.cache_disable());
    }

    /// SDM Table 4-5: bit 7 of a PDE is ignored when CR4.PSE = 0, so the same
    /// entry references a page table in one mode and maps a page in the other.
    #[test]
    fn pde_ps_ignored_without_cr4_pse() {
        let pde = Pde(0x0040_0000 | ENTRY_P | PDE_PS);
        assert!(!pde.maps_large_page(false));
        assert!(pde.maps_large_page(true));
        assert_eq!(pde.page_table_base(), 0x0040_0000);
    }

    /// SDM §4.3: "If CR4.PSE = 0, no bits are reserved with 32-bit paging."
    #[test]
    fn no_reserved_bits_when_pse_clear() {
        let profile = PagingProfile::default();
        let pte = Pte(0xFFFF_FFFF);
        let pde = Pde(0xFFFF_FFFF);
        assert_eq!(pte.reserved_bits(false, &profile), 0);
        assert_eq!(pde.reserved_bits(false, &profile), 0);
    }

    /// SDM §4.3: with CR4.PSE = 1 and no PAT, PTE bit 7 is reserved when P = 1.
    #[test]
    fn pte_bit7_reserved_with_pse_and_no_pat() {
        let profile = PagingProfile::default();
        let pte = Pte(0x0000_1000 | ENTRY_P | PTE_PAT);
        assert_eq!(pte.reserved_bits(true, &profile), PTE_PAT);

        // Reserved bits are not checked in an entry whose P flag is 0
        // (SDM §4.7, RSVD flag note).
        let absent = Pte(PTE_PAT);
        assert_eq!(absent.reserved_bits(true, &profile), 0);

        // With the PAT supported, bit 7 is the PAT selector, not reserved.
        let pat_profile = PagingProfile {
            pat_supported: true,
            ..PagingProfile::default()
        };
        assert_eq!(pte.reserved_bits(true, &pat_profile), 0);
    }

    /// SDM Table 4-4: with PSE-36 unsupported, bits 21:13 are reserved in a
    /// 4-MiB PDE, and bit 12 as well while the PAT is unsupported.
    #[test]
    fn large_pde_reserved_bits_without_pse36() {
        let profile = PagingProfile::default();
        assert_eq!(large_page_reserved_addr_mask(&profile), 0x003F_E000);

        let pde = Pde(0x0080_0000 | ENTRY_P | PDE_PS);
        assert_eq!(pde.reserved_bits(true, &profile), 0);
        assert_eq!(pde.large_page_base(&profile), 0x0080_0000);

        let with_high_bits = Pde(pde.0 | 0x0000_2000);
        assert_eq!(with_high_bits.reserved_bits(true, &profile), 0x0000_2000);

        let with_pat = Pde(pde.0 | PDE_LARGE_PAT);
        assert_eq!(with_pat.reserved_bits(true, &profile), PDE_LARGE_PAT);
    }

    /// SDM Table 4-4: with PSE-36 at MAXPHYADDR 40, only bit 21 is reserved and
    /// PDE bits 20:13 supply physical-address bits 39:32.
    #[test]
    fn large_pde_pse36_high_address_bits() {
        let profile = PagingProfile::with_pse36(40);
        assert_eq!(large_page_reserved_addr_mask(&profile), 1 << 21);

        // PDE bits 20:13 = 0x0F -> physical bits 39:32 = 0x0F.
        let pde = Pde(0x0080_0000 | (0x0F << 13) | ENTRY_P | PDE_PS);
        assert_eq!(pde.reserved_bits(true, &profile), 0);
        assert_eq!(pde.large_page_base(&profile), 0x0000_000F_0080_0000);

        // Bit 21 set is a reserved-bit violation even with PSE-36.
        let bad = Pde(pde.0 | (1 << 21));
        assert_eq!(bad.reserved_bits(true, &profile), 1 << 21);
    }

    /// SDM Table 4-4 note 2: with PSE-36 supported and MAXPHYADDR 36, M is 36,
    /// so bits 21:17 are reserved and bits 16:13 carry physical bits 35:32.
    #[test]
    fn large_pde_pse36_maxphyaddr_36() {
        let profile = PagingProfile::with_pse36(36);
        assert_eq!(profile.large_page_addr_width(), 36);
        assert_eq!(large_page_reserved_addr_mask(&profile), 0x003E_0000);

        let pde = Pde(0x0080_0000 | (0x0F << 13) | ENTRY_P | PDE_PS);
        assert_eq!(pde.reserved_bits(true, &profile), 0);
        assert_eq!(pde.large_page_base(&profile), 0x0000_000F_0080_0000);
    }
}
