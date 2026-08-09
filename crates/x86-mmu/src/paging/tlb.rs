//! Translation lookaside buffer.
//!
//! Spec: Intel SDM Vol. 3 §4.10.2 "Translation Lookaside Buffers (TLBs)",
//! §4.10.2.4 (global pages), §4.10.4.1 (operations that invalidate TLBs).
//!
//! # This is a correctness model, not a performance cache
//!
//! Nothing here exists to make translation faster — the walker is already a
//! handful of array reads. It exists because a TLB is *architecturally
//! visible*: software that edits a paging-structure entry and does not
//! invalidate must be able to observe the stale translation, because that is
//! what real hardware does and what operating systems are written against. A
//! guest that forgets an `INVLPG` should misbehave here in the same way it
//! misbehaves on silicon, rather than working by accident.
//!
//! So the implementation is deliberately simple: an unbounded map keyed by
//! 4-KiB page number, no capacity, no replacement policy, no set associativity.
//! The SDM permits all of that — "Processors need not implement any TLBs.
//! Processors that do implement TLBs may invalidate any TLB entry at any time"
//! (§4.10.2.2) — and a size-limited model would only add nondeterminism.
//!
//! Not modeled: the paging-structure caches (§4.10.3), PCIDs and `INVPCID`
//! (§4.10.1), multiple logical processors, and speculative or prefetch-driven
//! caching of translations that never occur (§4.10.2.3).

use std::collections::BTreeMap;

use super::entry::ENTRY_D;
use super::{
    Access, AccessKind, AccessMode, FaultReason, PageFault, PageSize, PageTableMemory,
    PagingContext, PagingMode, TranslateError, Translation,
};

/// One cached translation.
///
/// Spec: SDM §4.10.2.2 — a TLB entry holds the page frame, the access rights
/// (the logical-AND of the R/W flags and of the U/S flags over the entries used
/// to translate), and, from the entry that identifies the final page frame, the
/// dirty flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TlbEntry {
    /// Physical base of the page frame.
    pub frame_base: u64,
    /// Page size of the translation the entry came from.
    pub page_size: PageSize,
    /// Logical-AND of the R/W flags.
    pub writable: bool,
    /// Logical-AND of the U/S flags.
    pub user_accessible: bool,
    /// Global (§4.10.2.4): the final entry's G flag, cached while
    /// `CR4.PGE = 1`.
    pub global: bool,
    /// Dirty flag of the entry that identifies the final page frame.
    pub dirty: bool,
    /// Physical address of that entry, so a write through a cached translation
    /// can still set the dirty flag in memory (§4.8).
    pub final_entry_addr: u64,
}

impl TlbEntry {
    /// Does this entry translate `linear`? For a 4-MiB translation the same
    /// page may be cached under several 4-KiB page numbers (§4.10.2.3), so
    /// coverage is checked at the entry's own granularity.
    fn covers(&self, key_page: u32, linear: u32) -> bool {
        match self.page_size {
            PageSize::Size4KiB => key_page == linear >> 12,
            PageSize::Size4MiB => (key_page >> 10) == (linear >> 22),
        }
    }
}

/// A set of cached translations.
#[derive(Clone, Debug, Default)]
pub struct Tlb {
    entries: BTreeMap<u32, TlbEntry>,
}

impl Tlb {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of cached translations. Diagnostic only; software cannot observe
    /// this.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The cached translation for `linear`, if any.
    pub fn lookup(&self, linear: u32) -> Option<&TlbEntry> {
        self.entries.get(&(linear >> 12))
    }

    /// Cache a translation for the 4-KiB page number containing `linear`.
    pub fn insert(&mut self, linear: u32, entry: TlbEntry) {
        self.entries.insert(linear >> 12, entry);
    }

    /// `INVLPG`, and the invalidation a page fault performs.
    ///
    /// Spec: SDM §4.10.4.1 — `INVLPG` "invalidates any TLB entries that are for
    /// a page number corresponding to the linear address ... It also
    /// invalidates any global TLB entries with that page number, regardless of
    /// PCID", and if the page is larger than 4 KiB and several entries exist
    /// for it, "the instruction invalidates all of them". A page-fault
    /// exception invalidates the same entries, so that re-executing the
    /// faulting instruction cannot repeat a fault that the paging structures in
    /// memory no longer describe.
    pub fn invalidate_page(&mut self, linear: u32) {
        self.entries
            .retain(|&page, entry| !entry.covers(page, linear));
    }

    /// `MOV to CR3` with `CR4.PCIDE = 0`.
    ///
    /// Spec: SDM §4.10.4.1 — the instruction "invalidates all TLB entries
    /// associated with PCID 000H **except those for global pages**". An entry
    /// is global only if it was cached from a final entry with `G = 1` while
    /// `CR4.PGE = 1` (§4.10.2.4), so with `CR4.PGE = 0` no entry is global and
    /// this is a full flush.
    pub fn flush_non_global(&mut self) {
        self.entries.retain(|_, entry| entry.global);
    }

    /// Invalidate everything, global entries included.
    ///
    /// Spec: SDM §4.10.4.1 — `MOV to CR0` clearing `CR0.PG`, and `MOV to CR4`
    /// changing `CR4.PGE`, invalidate all TLB entries including global ones.
    pub fn flush_all(&mut self) {
        self.entries.clear();
    }
}

/// A [`Tlb`] plus the translation path that uses it, and the control-register
/// shadow needed to notice an invalidating control-register change.
#[derive(Clone, Debug, Default)]
pub struct Mmu {
    tlb: Tlb,
    last_cr0: Option<u64>,
    last_cr3: Option<u64>,
    last_cr4: Option<u64>,
}

impl Mmu {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tlb(&self) -> &Tlb {
        &self.tlb
    }

    pub fn tlb_mut(&mut self) -> &mut Tlb {
        &mut self.tlb
    }

    /// `INVLPG <linear>` (SDM §4.10.4.1).
    pub fn invlpg(&mut self, linear: u32) {
        self.tlb.invalidate_page(linear);
    }

    /// `MOV to CR3` (SDM §4.10.4.1): flush everything except global entries.
    pub fn on_mov_to_cr3(&mut self, new_cr3: u64) {
        self.tlb.flush_non_global();
        self.last_cr3 = Some(new_cr3);
    }

    /// `MOV to CR0` (SDM §4.10.4.1): a full flush, global entries included,
    /// when the instruction changes `CR0.PG` from 1 to 0. A change of `CR0.WP`
    /// requires no invalidation, because a TLB entry caches the combined R/W
    /// flag and `CR0.WP` is applied per access.
    pub fn on_mov_to_cr0(&mut self, old_cr0: u64, new_cr0: u64) {
        if (old_cr0 & super::CR0_PG) != 0 && (new_cr0 & super::CR0_PG) == 0 {
            self.tlb.flush_all();
        }
        self.last_cr0 = Some(new_cr0);
    }

    /// `MOV to CR4` (SDM §4.10.4.1).
    ///
    /// * Changing `CR4.PGE` invalidates all TLB entries **including global
    ///   ones**, in either direction.
    /// * Changing `CR4.PAE` invalidates all entries for the current PCID.
    /// * Changing `CR4.PSE` "may invalidate TLB entries" — the SDM permits it
    ///   rather than requiring it. This model flushes, because a `CR4.PSE`
    ///   change reinterprets every `PS` bit and so changes the page size of
    ///   existing translations, and §4.10.2.3 warns that leaving both sizes
    ///   cached makes which one is used implementation-specific.
    pub fn on_mov_to_cr4(&mut self, old_cr4: u64, new_cr4: u64) {
        let changed = old_cr4 ^ new_cr4;
        if changed & (super::CR4_PGE | super::CR4_PAE | super::CR4_PSE) != 0 {
            self.tlb.flush_all();
        }
        self.last_cr4 = Some(new_cr4);
    }

    /// Apply whatever invalidation the current control-register values imply
    /// since the last time this MMU saw them.
    ///
    /// The explicit `on_mov_to_*` hooks are the precise interface; this is the
    /// polled equivalent for an integration that changes `CR0` / `CR3` / `CR4`
    /// in more than one place — a task switch, an `IRET`, a reset. Calling it
    /// is always safe: with nothing changed it does nothing.
    pub fn sync_control_registers(&mut self, ctx: &PagingContext) {
        if let Some(old_cr0) = self.last_cr0 {
            if old_cr0 != ctx.cr0 {
                self.on_mov_to_cr0(old_cr0, ctx.cr0);
            }
        }
        if let Some(old_cr4) = self.last_cr4 {
            if old_cr4 != ctx.cr4 {
                self.on_mov_to_cr4(old_cr4, ctx.cr4);
            }
        }
        if let Some(old_cr3) = self.last_cr3 {
            if old_cr3 != ctx.cr3 {
                self.on_mov_to_cr3(ctx.cr3);
            }
        }
        self.last_cr0 = Some(ctx.cr0);
        self.last_cr3 = Some(ctx.cr3);
        self.last_cr4 = Some(ctx.cr4);
    }

    /// Translate through the TLB, walking the paging structures on a miss.
    ///
    /// A successful walk is cached (§4.10.2.3: a translation may be cached only
    /// once the P flag is 1, no reserved bit is set, and the accessed flag is 1
    /// in every entry used — which [`super::translate`] has just guaranteed). A
    /// fault invalidates any entry for the faulting page (§4.10.4.1) and caches
    /// nothing.
    pub fn translate<M: PageTableMemory>(
        &mut self,
        ctx: &PagingContext,
        mem: &mut M,
        linear: u32,
        access: Access,
    ) -> Result<Translation, TranslateError> {
        if ctx.mode() != PagingMode::Bits32 {
            return super::translate(ctx, mem, linear, access);
        }

        if let Some(entry) = self.tlb.lookup(linear).copied() {
            return self.translate_hit(ctx, mem, linear, access, entry);
        }

        match super::translate(ctx, mem, linear, access) {
            Ok(translation) => {
                self.tlb.insert(
                    linear,
                    TlbEntry {
                        frame_base: translation.frame_base,
                        page_size: translation.page_size,
                        writable: translation.writable,
                        user_accessible: translation.user_accessible,
                        global: translation.global,
                        dirty: translation.dirty,
                        final_entry_addr: translation.final_entry_addr,
                    },
                );
                Ok(translation)
            }
            Err(err) => {
                self.tlb.invalidate_page(linear);
                Err(err)
            }
        }
    }

    fn translate_hit<M: PageTableMemory>(
        &mut self,
        ctx: &PagingContext,
        mem: &mut M,
        linear: u32,
        access: Access,
        mut entry: TlbEntry,
    ) -> Result<Translation, TranslateError> {
        if !rights_permit(ctx, entry.writable, entry.user_accessible, access) {
            self.tlb.invalidate_page(linear);
            return Err(TranslateError::Fault(PageFault {
                linear_address: linear,
                access,
                reason: FaultReason::Protection,
            }));
        }

        // SDM §4.8: the write still has to set the dirty flag in the entry that
        // identifies the final physical address, which is why §4.10.2.2 has the
        // TLB cache that flag in the first place. The accessed flag is left
        // alone: it was 1 when the translation was cached, and §4.10.4.3 says
        // the processor need not set it again if software cleared it without
        // invalidating.
        if access.is_write() && !entry.dirty {
            let bits = mem.read_entry_u32(entry.final_entry_addr);
            mem.write_entry_u32(entry.final_entry_addr, bits | ENTRY_D);
            entry.dirty = true;
            self.tlb.insert(linear, entry);
        }

        let offset = u64::from(linear) & entry.page_size.offset_mask();
        Ok(Translation {
            linear_address: linear,
            phys_addr: entry.frame_base + offset,
            frame_base: entry.frame_base,
            page_size: entry.page_size,
            writable: entry.writable,
            user_accessible: entry.user_accessible,
            global: entry.global,
            final_entry_addr: entry.final_entry_addr,
            dirty: entry.dirty,
        })
    }
}

/// The §4.6.1 rules applied to already-combined access rights, so a TLB hit and
/// a full walk cannot disagree.
///
/// `CR0.WP` is read from the context on every access rather than cached,
/// because it is a paging-mode modifier (§4.1.3) and not a property of a
/// translation. That is also why changing it needs no invalidation.
pub(crate) fn rights_permit(
    ctx: &PagingContext,
    writable: bool,
    user_accessible: bool,
    access: Access,
) -> bool {
    match access.mode {
        AccessMode::Supervisor => match access.kind {
            AccessKind::Read | AccessKind::InstructionFetch => true,
            AccessKind::Write => !ctx.write_protect() || writable,
        },
        AccessMode::User => match access.kind {
            AccessKind::Read | AccessKind::InstructionFetch => user_accessible,
            AccessKind::Write => user_accessible && writable,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(page_size: PageSize) -> TlbEntry {
        TlbEntry {
            frame_base: 0x1000,
            page_size,
            writable: true,
            user_accessible: false,
            global: false,
            dirty: false,
            final_entry_addr: 0x2000,
        }
    }

    /// A 4-KiB entry covers exactly its own page number.
    #[test]
    fn small_page_coverage() {
        let e = entry(PageSize::Size4KiB);
        assert!(e.covers(0x1234, 0x0123_4000));
        assert!(e.covers(0x1234, 0x0123_4FFF));
        assert!(!e.covers(0x1234, 0x0123_5000));
    }

    /// SDM §4.10.2.3: a 4-MiB translation may be cached under several 4-KiB
    /// page numbers, and an `INVLPG` for any address on the page invalidates
    /// all of them.
    #[test]
    fn large_page_coverage_spans_the_whole_4mib() {
        let e = entry(PageSize::Size4MiB);
        let key_page = 0x0040_0000u32 >> 12;
        assert!(e.covers(key_page, 0x0040_0000));
        assert!(e.covers(key_page, 0x007F_FFFF));
        assert!(!e.covers(key_page, 0x0080_0000));
    }

    /// `flush_non_global` keeps exactly the global entries (§4.10.4.1).
    #[test]
    fn flush_non_global_keeps_global_entries() {
        let mut tlb = Tlb::new();
        tlb.insert(0x0000_1000, entry(PageSize::Size4KiB));
        let mut global = entry(PageSize::Size4KiB);
        global.global = true;
        tlb.insert(0x0000_2000, global);

        tlb.flush_non_global();
        assert_eq!(tlb.len(), 1);
        assert!(tlb.lookup(0x0000_2000).is_some());

        tlb.flush_all();
        assert!(tlb.is_empty());
    }
}
