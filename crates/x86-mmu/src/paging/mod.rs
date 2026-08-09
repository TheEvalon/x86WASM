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
pub mod tlb;

pub use entry::{PagingProfile, Pde, Pte};
pub use tlb::{Mmu, Tlb, TlbEntry};

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
/// Spec: SDM §4.7 — an access "may cause a page-fault exception for either of
/// two reasons: (1) there is no translation for the linear address; or (2)
/// there is a translation for the linear address, but its access rights do not
/// permit the access". There is no translation if the process "would use a
/// paging-structure entry in which the P flag (bit 0) is 0 or one that sets a
/// reserved bit".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultReason {
    /// The P flag was 0 in the entry at this level.
    NotPresent(PagingLevel),
    /// The entry at this level set a reserved bit.
    ReservedBit(PagingLevel),
    /// The translation exists but its access rights do not permit the access
    /// (§4.6.1).
    Protection,
}

/// What kind of access is being attempted.
///
/// Spec: SDM §4.6.1 and the §4.7 error-code definitions, which describe the
/// access rather than the access rights.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessKind {
    /// A data read.
    Read,
    /// A data write.
    Write,
    /// An instruction fetch.
    InstructionFetch,
}

/// Whether the access is a supervisor-mode or user-mode access.
///
/// Spec: SDM §4.6.1 — "accesses made while CPL < 3 are supervisor-mode
/// accesses, while accesses made while CPL = 3 are user-mode accesses".
///
/// The SDM further splits supervisor-mode accesses into *implicit* (accesses to
/// the GDT/LDT/IDT/TSS) and *explicit* ones. That distinction only ever
/// modifies `CR4.SMAP` behavior, and SMAP is not modeled here, so this engine
/// does not carry it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessMode {
    /// CPL < 3, or an implicit access to a system data structure.
    Supervisor,
    /// CPL = 3.
    User,
}

/// The access an interpreter is asking permission for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Access {
    pub kind: AccessKind,
    pub mode: AccessMode,
}

impl Access {
    pub fn new(kind: AccessKind, mode: AccessMode) -> Self {
        Self { kind, mode }
    }

    /// Build an access from the current privilege level (SDM §4.6.1: CPL = 3 is
    /// a user-mode access, CPL < 3 a supervisor-mode one).
    pub fn from_cpl(kind: AccessKind, cpl: u8) -> Self {
        let mode = if cpl == 3 {
            AccessMode::User
        } else {
            AccessMode::Supervisor
        };
        Self { kind, mode }
    }

    pub fn is_write(&self) -> bool {
        matches!(self.kind, AccessKind::Write)
    }

    pub fn is_user(&self) -> bool {
        matches!(self.mode, AccessMode::User)
    }
}

/// Page-fault error code, P (bit 0). SDM §4.7 / Figure 4-12.
pub const PF_ERR_P: u32 = 1 << 0;
/// Page-fault error code, W/R (bit 1).
pub const PF_ERR_WR: u32 = 1 << 1;
/// Page-fault error code, U/S (bit 2).
pub const PF_ERR_US: u32 = 1 << 2;
/// Page-fault error code, RSVD (bit 3).
pub const PF_ERR_RSVD: u32 = 1 << 3;
/// Page-fault error code, I/D (bit 4).
///
/// Never set by this engine: §4.7 sets it only if the access was an instruction
/// fetch **and** either `CR4.SMEP = 1` or (`CR4.PAE = 1` and
/// `IA32_EFER.NXE = 1`). With 32-bit paging and no SMEP, none of those hold.
pub const PF_ERR_ID: u32 = 1 << 4;

/// A structured page fault. Delivery — vector 14, pushing the error code, and
/// loading `CR2` — is the interpreter's job; this engine only reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageFault {
    /// The linear address whose use caused the fault. This is the value that
    /// belongs in `CR2` (SDM §4.7 / Vol. 3 "Interrupt 14—Page-Fault
    /// Exception").
    pub linear_address: u32,
    /// The access that caused the fault. The §4.7 error code describes the
    /// access, not the access rights, so it is recorded verbatim.
    pub access: Access,
    /// What went wrong.
    pub reason: FaultReason,
}

impl PageFault {
    /// The value the interpreter must load into `CR2` when it delivers this
    /// fault.
    pub fn cr2(&self) -> u64 {
        u64::from(self.linear_address)
    }

    /// The `#PF` error code, composed exactly as SDM §4.7 defines it.
    ///
    /// * P (bit 0) is 0 "if there is no translation for the linear address
    ///   because the P flag was 0 in one of the paging-structure entries" — so
    ///   it is set for a protection violation and for a reserved-bit violation,
    ///   and clear only for a not-present fault.
    /// * W/R (bit 1) is 1 if the causing access was a write.
    /// * U/S (bit 2) is 1 if a user-mode access caused the fault.
    /// * RSVD (bit 3) is 1 if a reserved bit was set in one of the entries
    ///   used. Because reserved bits are not checked in an entry whose P flag
    ///   is 0, bit 3 can be set only if bit 0 is also set.
    /// * I/D (bit 4) is always 0 here; see [`PF_ERR_ID`].
    /// * PK (bit 5) and SGX (bit 15) are always 0: neither protection keys nor
    ///   SGX exist in this model.
    pub fn error_code(&self) -> u32 {
        let mut code = 0;
        if !matches!(self.reason, FaultReason::NotPresent(_)) {
            code |= PF_ERR_P;
        }
        if self.access.is_write() {
            code |= PF_ERR_WR;
        }
        if self.access.is_user() {
            code |= PF_ERR_US;
        }
        if matches!(self.reason, FaultReason::ReservedBit(_)) {
            code |= PF_ERR_RSVD;
        }
        code
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

/// Failure of a structural walk.
///
/// A walk knows nothing about the access being attempted, so it reports the
/// fault *reason* rather than a complete [`PageFault`]; [`translate`] pairs the
/// reason with the access to produce the error code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WalkError {
    /// The linear address has no translation (§4.3, §4.7).
    Fault(FaultReason),
    /// Outside what this engine models.
    Unsupported(UnsupportedPaging),
}

impl WalkError {
    /// The fault reason, if this failure is one.
    pub fn as_fault_reason(&self) -> Option<FaultReason> {
        match self {
            WalkError::Fault(reason) => Some(*reason),
            WalkError::Unsupported(_) => None,
        }
    }

    /// Pair this failure with the access that provoked it.
    pub fn into_translate_error(self, linear: u32, access: Access) -> TranslateError {
        match self {
            WalkError::Fault(reason) => TranslateError::Fault(PageFault {
                linear_address: linear,
                access,
                reason,
            }),
            WalkError::Unsupported(kind) => TranslateError::Unsupported(kind),
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

impl Walk {
    /// Combined R/W: the logical-AND of the R/W flags of every entry used.
    ///
    /// Spec: SDM §4.6.1 — a write is permitted only "with a translation for
    /// which the R/W flag (bit 1) is 1 in every paging-structure entry
    /// controlling the translation"; §4.10.2.2 records the same logical-AND in
    /// a TLB entry.
    pub fn writable(&self) -> bool {
        self.pde.read_write() && self.pte.is_none_or(|(_, pte)| pte.read_write())
    }

    /// Combined U/S: the logical-AND of the U/S flags of every entry used.
    ///
    /// Spec: SDM §4.6.1 — "If the U/S flag (bit 2) is 0 in at least one of the
    /// paging-structure entries, the address is a supervisor-mode address.
    /// Otherwise, the address is a user-mode address."
    pub fn user_accessible(&self) -> bool {
        self.pde.user_supervisor() && self.pte.is_none_or(|(_, pte)| pte.user_supervisor())
    }

    /// The G flag of the entry that maps the page, before `CR4.PGE` gating
    /// (SDM Tables 4-4/4-6 bit 8, §4.10.2.4).
    pub fn global_flag(&self) -> bool {
        match self.pte {
            Some((_, pte)) => pte.global(),
            None => self.pde.global(),
        }
    }

    /// Physical address of the entry that identifies the final page frame — the
    /// PTE, or the PDE when `PS = 1`. This is the entry whose dirty flag a
    /// write updates (SDM §4.8).
    pub fn final_entry_addr(&self) -> u64 {
        match self.pte {
            Some((addr, _)) => addr,
            None => self.pde_addr,
        }
    }

    /// Raw value of the entry that identifies the final page frame.
    pub fn final_entry_bits(&self) -> u32 {
        match self.pte {
            Some((_, pte)) => pte.bits(),
            None => self.pde.bits(),
        }
    }

    /// The dirty flag of the entry that identifies the final page frame.
    pub fn dirty(&self) -> bool {
        self.final_entry_bits() & entry::ENTRY_D != 0
    }
}

/// A permitted translation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Translation {
    /// The linear address translated.
    pub linear_address: u32,
    /// The physical address it maps to.
    pub phys_addr: u64,
    /// Physical base of the page frame.
    pub frame_base: u64,
    /// 4 KiB or 4 MiB.
    pub page_size: PageSize,
    /// Combined R/W of the entries used (SDM §4.6.1).
    pub writable: bool,
    /// Combined U/S of the entries used (SDM §4.6.1).
    pub user_accessible: bool,
    /// Whether the translation is global: the final entry's G flag with
    /// `CR4.PGE = 1` (SDM §4.10.2.4).
    pub global: bool,
    /// Physical address of the entry that identifies the final page frame.
    pub final_entry_addr: u64,
    /// Dirty flag of that entry, after any update this translation performed.
    pub dirty: bool,
}

/// Does the translation permit this access?
///
/// Spec: SDM §4.6.1, restricted to what 32-bit paging without SMEP, SMAP,
/// protection keys or execute-disable can express:
///
/// * Supervisor-mode data reads are permitted to any address (SMAP is not
///   modeled, so `EFLAGS.AC` and the implicit/explicit distinction never
///   matter).
/// * Supervisor-mode data writes: if `CR0.WP = 0` they are permitted to any
///   address; if `CR0.WP = 1` they require R/W = 1 in every entry — including
///   for a *user-mode* address, which is the classic supervisor-write-to-a-
///   read-only-user-page case.
/// * Supervisor-mode instruction fetches are permitted from any address: SMEP
///   is not modeled, and "for 32-bit paging or if IA32_EFER.NXE = 0,
///   instructions may be fetched from any ... address".
/// * User-mode accesses are permitted only to user-mode addresses, and a
///   user-mode write additionally requires R/W = 1 in every entry, regardless
///   of `CR0.WP`.
///
/// A TLB hit runs the same rules over the rights it cached, through the shared
/// [`tlb::rights_permit`], so the two paths cannot disagree.
pub fn access_permitted(ctx: &PagingContext, walk: &Walk, access: Access) -> bool {
    tlb::rights_permit(ctx, walk.writable(), walk.user_accessible(), access)
}

/// Translate `linear` for `access`: walk the paging structures, apply the §4.6
/// access rights, and only then update the §4.8 accessed and dirty flags.
///
/// Returns a structured [`PageFault`] rather than raising anything. Delivering
/// `#PF` with [`PageFault::error_code`] and [`PageFault::cr2`] is the
/// interpreter's job.
///
/// # Accessed/dirty ordering
///
/// Every fault check runs before any paging-structure write, so **a faulting
/// access leaves the paging structures byte-for-byte unchanged**. See
/// [`commit_accessed_dirty`] for what the SDM does and does not pin down here.
pub fn translate<M: PageTableMemory>(
    ctx: &PagingContext,
    mem: &mut M,
    linear: u32,
    access: Access,
) -> Result<Translation, TranslateError> {
    let walk = walk(ctx, mem, linear).map_err(|err| err.into_translate_error(linear, access))?;

    if !access_permitted(ctx, &walk, access) {
        return Err(TranslateError::Fault(PageFault {
            linear_address: linear,
            access,
            reason: FaultReason::Protection,
        }));
    }

    let dirty = commit_accessed_dirty(mem, &walk, access);

    Ok(Translation {
        linear_address: linear,
        phys_addr: walk.phys_addr,
        frame_base: walk.frame_base,
        page_size: walk.page_size,
        writable: walk.writable(),
        user_accessible: walk.user_accessible(),
        global: walk.global_flag() && ctx.pge(),
        final_entry_addr: walk.final_entry_addr(),
        dirty,
    })
}

/// Set the accessed flag in every entry the translation used, and the dirty
/// flag in the entry that maps the page when the access is a write. Returns the
/// dirty flag of that entry afterwards.
///
/// Spec: SDM §4.8 — "Whenever the processor uses a paging-structure entry as
/// part of linear-address translation, it sets the accessed flag in that entry
/// (if it is not already set)"; "Whenever there is a write to a linear address,
/// the processor sets the dirty flag (if it is not already set) in the
/// paging-structure entry that identifies the final physical address for the
/// linear address (either a PTE or a paging-structure entry in which the PS
/// flag is 1)". The flags are sticky, so an entry that already has the bits set
/// is not rewritten. Bit 6 of a PDE that references a page table is ignored
/// (Table 4-5) and is never touched.
///
/// # The one model choice here
///
/// This function runs only after the walk succeeded **and** the access-rights
/// check passed, so a faulting access performs no paging-structure write at
/// all. The SDM's own ordering is looser than that in two places:
///
/// * An entry with `P = 0`, or one that sets a reserved bit, "is used neither
///   to reference another paging-structure entry nor to map a page" (§4.3), so
///   its own accessed flag is certainly not set. But a *higher-level* entry was
///   read before the failure, and a literal reading of §4.8 would set its
///   accessed flag. This engine does not.
/// * §4.6 checks access rights after the translation has been produced, so a
///   literal reading would also set accessed flags on a protection violation.
///   This engine does not.
///
/// §4.10.2.3 states the tighter rule this follows: "the processor does not
/// cache a translation for a page number unless the accessed flag is 1 in each
/// of the paging-structure entries used during translation; before caching a
/// translation, the processor sets any of these accessed flags that is not
/// already 1" — accessed-flag setting tied to a translation that actually
/// completes. The difference is invisible to conforming software, because these
/// flags are sticky hints that software may not read as "this did not happen"
/// (§4.10.4.2); it matters only to a guest inspecting flags after a fault, and
/// erring toward *fewer* updates cannot invent an access that never occurred.
pub fn commit_accessed_dirty<M: PageTableMemory>(mem: &mut M, walk: &Walk, access: Access) -> bool {
    let write = access.is_write();
    let large_page = walk.pte.is_none();

    // One write per entry: an entry needing both A and D gets them together.
    let mut pde_bits = walk.pde.bits();
    let mut wanted = pde_bits | entry::ENTRY_A;
    if large_page && write {
        wanted |= entry::ENTRY_D;
    }
    if wanted != pde_bits {
        mem.write_entry_u32(walk.pde_addr, wanted);
        pde_bits = wanted;
    }

    match walk.pte {
        Some((pte_addr, pte)) => {
            let mut pte_bits = pte.bits();
            let mut wanted = pte_bits | entry::ENTRY_A;
            if write {
                wanted |= entry::ENTRY_D;
            }
            if wanted != pte_bits {
                mem.write_entry_u32(pte_addr, wanted);
                pte_bits = wanted;
            }
            pte_bits & entry::ENTRY_D != 0
        }
        None => pde_bits & entry::ENTRY_D != 0,
    }
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
) -> Result<Walk, WalkError> {
    match ctx.mode() {
        PagingMode::Disabled => {
            return Err(WalkError::Unsupported(UnsupportedPaging::PagingDisabled))
        }
        PagingMode::PaeOrLongMode => {
            return Err(WalkError::Unsupported(UnsupportedPaging::PaeOrLongMode))
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
        return Err(WalkError::Fault(FaultReason::NotPresent(
            PagingLevel::PageDirectory,
        )));
    }
    if pde.sets_reserved_bit(cr4_pse, &profile) {
        return Err(WalkError::Fault(FaultReason::ReservedBit(
            PagingLevel::PageDirectory,
        )));
    }

    if pde.maps_large_page(cr4_pse) {
        // SDM §4.3: "Bits 39:32 are bits 20:13 of the PDE. Bits 31:22 are bits
        // 31:22 of the PDE. Bits 21:0 are from the original linear address."
        let frame_base = pde.large_page_base(&profile);
        let offset = u64::from(linear) & PageSize::Size4MiB.offset_mask();
        return Ok(Walk {
            linear_address: linear,
            page_size: PageSize::Size4MiB,
            frame_base,
            phys_addr: frame_base + offset,
            pde_addr,
            pde,
            pte: None,
        });
    }

    // SDM §4.3: "Bits 31:12 are from the PDE. Bits 11:2 are bits 21:12 of the
    // linear address. Bits 1:0 are 0."
    let pte_addr = pde.page_table_base() + 4 * u64::from((linear >> 12) & 0x3FF);
    let pte = Pte(mem.read_entry_u32(pte_addr));

    if !pte.present() {
        return Err(WalkError::Fault(FaultReason::NotPresent(
            PagingLevel::PageTable,
        )));
    }
    if pte.sets_reserved_bit(cr4_pse, &profile) {
        return Err(WalkError::Fault(FaultReason::ReservedBit(
            PagingLevel::PageTable,
        )));
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
