//! 32-bit paging translation engine.
//!
//! Spec: Intel SDM Vol. 3 Chapter 4 "Paging" — §4.1.1 (paging-mode selection),
//! §4.1.3 (paging-mode modifiers), §4.3 (32-bit paging), Table 4-3 (use of
//! CR3).
//!
//! # Scope
//!
//! This is a **standalone engine**. Nothing in the interpreter or the machine
//! calls it: `CR0.PG`, `CR3`, `CR4` and `#PF` delivery are not wired to it, so
//! no guest can use it yet. It reads and writes guest physical memory only
//! through a caller-supplied [`PageTableMemory`], the same shape the `devices`
//! crate uses for bus-master transfers, so it has no dependency on
//! `machine-pc`.
//!
//! Unsupported and reported rather than guessed: PAE paging (§4.4), 4-level and
//! 5-level paging (§4.5), long mode, `CR4.SMEP` / `CR4.SMAP` / `CR4.PKE`
//! (§4.6.1, §4.6.2), `IA32_EFER.NXE` execute-disable, PCIDs and the
//! paging-structure caches (§4.10.1, §4.10.3), memory typing from PWT/PCD/PAT
//! (§4.9), and shadow or nested paging.

pub mod entry;

pub use entry::{PagingProfile, Pde, Pte};

/// `CR0.PG` — paging enable, bit 31 (SDM §4.1.1).
pub const CR0_PG: u64 = 1 << 31;
/// `CR0.WP` — write protect, bit 16 (SDM §4.1.3).
pub const CR0_WP: u64 = 1 << 16;
/// `CR4.PSE` — page size extensions, bit 4 (SDM §4.1.3).
pub const CR4_PSE: u64 = 1 << 4;
/// `CR4.PAE` — physical address extension, bit 5 (SDM §4.1.1).
pub const CR4_PAE: u64 = 1 << 5;
/// `CR4.PGE` — page global enable, bit 7 (SDM §4.1.3).
pub const CR4_PGE: u64 = 1 << 7;

/// `CR3.PWT`, bit 3 (SDM Table 4-3).
pub const CR3_PWT: u64 = 1 << 3;
/// `CR3.PCD`, bit 4 (SDM Table 4-3).
pub const CR3_PCD: u64 = 1 << 4;
/// `CR3` bits 31:12: the physical address of the page directory (SDM
/// Table 4-3).
pub const CR3_PD_BASE: u64 = 0xFFFF_F000;

/// Guest physical memory as the page walker needs it.
///
/// A 32-bit paging-structure entry is a naturally aligned little-endian
/// doubleword; an implementation over a byte-addressed bus must compose and
/// decompose it little-endian. Reads take `&mut self` because a bus read may
/// have side effects in general, even though page tables must live in RAM.
pub trait PageTableMemory {
    /// Read the 32-bit paging-structure entry at `phys_addr`.
    fn read_entry_u32(&mut self, phys_addr: u64) -> u32;
    /// Write the 32-bit paging-structure entry at `phys_addr`. Used only for
    /// accessed/dirty updates (SDM §4.8).
    fn write_entry_u32(&mut self, phys_addr: u64, value: u32);
}

impl<T: PageTableMemory + ?Sized> PageTableMemory for &mut T {
    fn read_entry_u32(&mut self, phys_addr: u64) -> u32 {
        (**self).read_entry_u32(phys_addr)
    }

    fn write_entry_u32(&mut self, phys_addr: u64, value: u32) {
        (**self).write_entry_u32(phys_addr, value)
    }
}

/// A [`PageTableMemory`] built from a pair of closures, for callers that do not
/// want to name a type.
pub struct ClosureMemory<R, W> {
    read: R,
    write: W,
}

impl<R, W> ClosureMemory<R, W>
where
    R: FnMut(u64) -> u32,
    W: FnMut(u64, u32),
{
    pub fn new(read: R, write: W) -> Self {
        Self { read, write }
    }
}

impl<R, W> PageTableMemory for ClosureMemory<R, W>
where
    R: FnMut(u64) -> u32,
    W: FnMut(u64, u32),
{
    fn read_entry_u32(&mut self, phys_addr: u64) -> u32 {
        (self.read)(phys_addr)
    }

    fn write_entry_u32(&mut self, phys_addr: u64, value: u32) {
        (self.write)(phys_addr, value)
    }
}

/// Which paging mode the control registers select.
///
/// Spec: SDM §4.1.1 "Three Paging Modes".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PagingMode {
    /// `CR0.PG = 0`: linear addresses are physical addresses.
    Disabled,
    /// `CR0.PG = 1`, `CR4.PAE = 0`: 32-bit paging (§4.3). The only mode this
    /// engine implements.
    Bits32,
    /// `CR0.PG = 1`, `CR4.PAE = 1`: PAE (§4.4) or IA-32e (§4.5) paging.
    /// Unsupported here.
    PaeOrLongMode,
}

/// The control-register state a 32-bit page walk depends on, plus the
/// processor-model profile that decides reserved bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PagingContext {
    /// `CR0`; only PG (bit 31) and WP (bit 16) are consulted.
    pub cr0: u64,
    /// `CR3`; bits 31:12 locate the page directory, PWT/PCD are stored but
    /// inert here (see [`PagingContext::cr3_write_through`]).
    pub cr3: u64,
    /// `CR4`; only PSE (bit 4), PAE (bit 5) and PGE (bit 7) are consulted.
    pub cr4: u64,
    /// Reserved-bit and physical-width profile.
    pub profile: PagingProfile,
}

impl PagingContext {
    /// Build a context from raw control-register values with the default
    /// profile (no PAT, no PSE-36, MAXPHYADDR 32).
    pub fn new(cr0: u64, cr3: u64, cr4: u64) -> Self {
        Self {
            cr0,
            cr3,
            cr4,
            profile: PagingProfile::default(),
        }
    }

    /// Same, with an explicit profile.
    pub fn with_profile(cr0: u64, cr3: u64, cr4: u64, profile: PagingProfile) -> Self {
        Self {
            cr0,
            cr3,
            cr4,
            profile,
        }
    }

    /// SDM §4.1.1.
    pub fn mode(&self) -> PagingMode {
        if self.cr0 & CR0_PG == 0 {
            PagingMode::Disabled
        } else if self.cr4 & CR4_PAE != 0 {
            PagingMode::PaeOrLongMode
        } else {
            PagingMode::Bits32
        }
    }

    /// `CR0.PG`.
    pub fn paging_enabled(&self) -> bool {
        self.cr0 & CR0_PG != 0
    }

    /// `CR0.WP` (SDM §4.1.3, §4.6.1).
    pub fn write_protect(&self) -> bool {
        self.cr0 & CR0_WP != 0
    }

    /// `CR4.PSE` (SDM §4.1.3).
    pub fn pse(&self) -> bool {
        self.cr4 & CR4_PSE != 0
    }

    /// `CR4.PAE` (SDM §4.1.1).
    pub fn pae(&self) -> bool {
        self.cr4 & CR4_PAE != 0
    }

    /// `CR4.PGE` (SDM §4.1.3, §4.10.2.4).
    pub fn pge(&self) -> bool {
        self.cr4 & CR4_PGE != 0
    }

    /// Physical address of the page directory: `CR3` bits 31:12 (SDM
    /// Table 4-3). Bits 2:0 and 11:5 are ignored, and bits 63:32 are ignored
    /// with 32-bit paging even on an Intel 64 processor.
    pub fn page_directory_base(&self) -> u64 {
        self.cr3 & CR3_PD_BASE
    }

    /// `CR3.PWT` (SDM Table 4-3). **Stored but inert**: it only selects the
    /// memory type used to access the page directory (§4.9), and this engine
    /// models no caches or memory types.
    pub fn cr3_write_through(&self) -> bool {
        self.cr3 & CR3_PWT != 0
    }

    /// `CR3.PCD` (SDM Table 4-3). Stored but inert, exactly as
    /// [`PagingContext::cr3_write_through`].
    pub fn cr3_cache_disable(&self) -> bool {
        self.cr3 & CR3_PCD != 0
    }
}

/// Page size produced by a translation (SDM §4.10.2.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageSize {
    /// 4 KiB; the translation used a PTE.
    Size4KiB,
    /// 4 MiB; `CR4.PSE = 1` and the PDE's PS flag was 1.
    Size4MiB,
}

impl PageSize {
    /// Size of the page in bytes.
    pub fn bytes(self) -> u64 {
        match self {
            PageSize::Size4KiB => 4 * 1024,
            PageSize::Size4MiB => 4 * 1024 * 1024,
        }
    }

    /// Mask selecting the page offset within the page.
    pub fn offset_mask(self) -> u64 {
        self.bytes() - 1
    }
}

/// Which paging structure a walk was in when it failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PagingLevel {
    /// The page directory located by `CR3`.
    PageDirectory,
    /// The page table located by a PDE.
    PageTable,
}

/// Why a linear address has no usable translation, or why an existing
/// translation refused the access.
///
/// Spec: SDM §4.7 — "there is no translation for a linear address if the
/// translation process for that address would use a paging-structure entry in
/// which the P flag (bit 0) is 0 or one that sets a reserved bit".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultReason {
    /// The P flag was 0 in the entry at this level.
    NotPresent(PagingLevel),
    /// The entry at this level set a reserved bit.
    ReservedBit(PagingLevel),
}

/// A structured page fault. Delivery — vector 14, the error code, and `CR2` —
/// is the interpreter's job; this engine only reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageFault {
    /// The linear address whose use caused the fault. This is the value that
    /// belongs in `CR2` (SDM §4.7 / Vol. 3 "Interrupt 14—Page-Fault
    /// Exception").
    pub linear_address: u32,
    /// What went wrong.
    pub reason: FaultReason,
}

impl PageFault {
    /// The value the interpreter must load into `CR2` when it delivers this
    /// fault.
    pub fn cr2(&self) -> u64 {
        u64::from(self.linear_address)
    }
}

/// A condition this engine deliberately does not model, reported instead of
/// guessed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnsupportedPaging {
    /// `CR0.PG = 0`. There is nothing to translate; the caller must use the
    /// linear address as the physical address itself (SDM §4.1.1).
    PagingDisabled,
    /// `CR4.PAE = 1`: PAE paging (§4.4) or IA-32e paging (§4.5).
    PaeOrLongMode,
    /// `CR4.PSE = 1` and the PDE's PS flag is 1. 4-MiB pages arrive in a later
    /// slice; until then this engine refuses rather than translating the
    /// address as if the PDE referenced a page table.
    LargePage,
}

/// Failure of a translation attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranslateError {
    /// An architectural page fault. Deliver `#PF`.
    Fault(PageFault),
    /// Outside what this engine models.
    Unsupported(UnsupportedPaging),
}

impl TranslateError {
    /// The page fault, if this failure is one.
    pub fn as_fault(&self) -> Option<PageFault> {
        match self {
            TranslateError::Fault(fault) => Some(*fault),
            TranslateError::Unsupported(_) => None,
        }
    }
}

/// The result of a structural page walk: every paging-structure entry the
/// translation used, and the physical address it produced.
///
/// A walk performs no access-rights check and has **no side effects** — in
/// particular it does not set accessed or dirty flags. That keeps
/// fault detection strictly ahead of any paging-structure write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Walk {
    /// The linear address that was walked.
    pub linear_address: u32,
    /// 4 KiB or 4 MiB.
    pub page_size: PageSize,
    /// Physical base of the mapped page frame.
    pub frame_base: u64,
    /// Translated physical address: `frame_base` plus the page offset.
    pub phys_addr: u64,
    /// Physical address of the PDE that was used.
    pub pde_addr: u64,
    /// The PDE that was used.
    pub pde: Pde,
    /// The PTE that was used and its physical address, when the translation
    /// used a page table.
    pub pte: Option<(u64, Pte)>,
}

/// Walk the 32-bit paging structures for `linear`, with no access-rights check
/// and no accessed/dirty update.
///
/// Spec: SDM §4.3. The PDE is selected with linear bits 31:22 and the PTE with
/// linear bits 21:12; an entry whose P flag is 0, or which sets a reserved bit,
/// yields no translation (§4.3, §4.7).
pub fn walk<M: PageTableMemory>(
    ctx: &PagingContext,
    mem: &mut M,
    linear: u32,
) -> Result<Walk, TranslateError> {
    match ctx.mode() {
        PagingMode::Disabled => {
            return Err(TranslateError::Unsupported(
                UnsupportedPaging::PagingDisabled,
            ))
        }
        PagingMode::PaeOrLongMode => {
            return Err(TranslateError::Unsupported(
                UnsupportedPaging::PaeOrLongMode,
            ))
        }
        PagingMode::Bits32 => {}
    }

    let cr4_pse = ctx.pse();
    let profile = ctx.profile;

    // SDM §4.3: "Bits 31:12 are from CR3. Bits 11:2 are bits 31:22 of the
    // linear address. Bits 1:0 are 0."
    let pde_addr = ctx.page_directory_base() + 4 * u64::from(linear >> 22);
    let pde = Pde(mem.read_entry_u32(pde_addr));

    if !pde.present() {
        return Err(fault(
            linear,
            FaultReason::NotPresent(PagingLevel::PageDirectory),
        ));
    }
    if pde.sets_reserved_bit(cr4_pse, &profile) {
        return Err(fault(
            linear,
            FaultReason::ReservedBit(PagingLevel::PageDirectory),
        ));
    }

    if pde.maps_large_page(cr4_pse) {
        return Err(TranslateError::Unsupported(UnsupportedPaging::LargePage));
    }

    // SDM §4.3: "Bits 31:12 are from the PDE. Bits 11:2 are bits 21:12 of the
    // linear address. Bits 1:0 are 0."
    let pte_addr = pde.page_table_base() + 4 * u64::from((linear >> 12) & 0x3FF);
    let pte = Pte(mem.read_entry_u32(pte_addr));

    if !pte.present() {
        return Err(fault(
            linear,
            FaultReason::NotPresent(PagingLevel::PageTable),
        ));
    }
    if pte.sets_reserved_bit(cr4_pse, &profile) {
        return Err(fault(
            linear,
            FaultReason::ReservedBit(PagingLevel::PageTable),
        ));
    }

    let frame_base = pte.page_base();
    let offset = u64::from(linear) & PageSize::Size4KiB.offset_mask();

    Ok(Walk {
        linear_address: linear,
        page_size: PageSize::Size4KiB,
        frame_base,
        phys_addr: frame_base + offset,
        pde_addr,
        pde,
        pte: Some((pte_addr, pte)),
    })
}

fn fault(linear: u32, reason: FaultReason) -> TranslateError {
    TranslateError::Fault(PageFault {
        linear_address: linear,
        reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SDM §4.1.1: the three paging modes, keyed on CR0.PG and CR4.PAE.
    #[test]
    fn mode_selection() {
        assert_eq!(PagingContext::new(0, 0, 0).mode(), PagingMode::Disabled);
        assert_eq!(PagingContext::new(CR0_PG, 0, 0).mode(), PagingMode::Bits32);
        assert_eq!(
            PagingContext::new(CR0_PG, 0, CR4_PAE).mode(),
            PagingMode::PaeOrLongMode
        );
        // CR4.PAE without CR0.PG still means paging is off.
        assert_eq!(
            PagingContext::new(0, 0, CR4_PAE).mode(),
            PagingMode::Disabled
        );
    }

    /// SDM Table 4-3: CR3 bits 2:0 and 11:5 are ignored, bits 31:12 are the
    /// page-directory address, and bits 63:32 are ignored with 32-bit paging.
    #[test]
    fn cr3_decode_ignores_the_bits_the_table_says_it_does() {
        let ctx = PagingContext::new(CR0_PG, 0xDEAD_BEEF_0012_3FFF, 0);
        assert_eq!(ctx.page_directory_base(), 0x0012_3000);
        assert!(ctx.cr3_write_through());
        assert!(ctx.cr3_cache_disable());
    }

    /// PageSize arithmetic used by the walker and the TLB.
    #[test]
    fn page_size_geometry() {
        assert_eq!(PageSize::Size4KiB.bytes(), 0x1000);
        assert_eq!(PageSize::Size4KiB.offset_mask(), 0xFFF);
        assert_eq!(PageSize::Size4MiB.bytes(), 0x40_0000);
        assert_eq!(PageSize::Size4MiB.offset_mask(), 0x3F_FFFF);
    }
}
