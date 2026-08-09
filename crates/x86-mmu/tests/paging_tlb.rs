//! TLB caching and invalidation.
//!
//! Spec: Intel SDM Vol. 3 §4.10.2.2 (what a TLB entry holds), §4.10.2.3
//! (details of TLB use; a translation is cached only once every entry used has
//! P = 1, no reserved bit, and A = 1), §4.10.2.4 (global pages), §4.10.4.1
//! (operations that invalidate TLBs, and the invalidation a page fault
//! performs).

mod common;

use common::PageTables;
use x86_mmu::paging::entry::{ENTRY_A, ENTRY_D, ENTRY_G, ENTRY_RW, ENTRY_US};
use x86_mmu::paging::{
    Access, AccessKind, AccessMode, FaultReason, Mmu, PageSize, PagingContext, CR0_PG, CR0_WP,
    CR4_PGE, CR4_PSE, PF_ERR_P, PF_ERR_US, PF_ERR_WR,
};

const LINEAR: u32 = 0x0808_1000;
const OTHER: u32 = 0x0C0C_2000;
const FRAME: u32 = 0x0044_0000;

fn context(tables: &PageTables, cr0_extra: u64, cr4: u64) -> PagingContext {
    PagingContext::new(CR0_PG | cr0_extra, tables.pd_base, cr4)
}

fn supervisor(kind: AccessKind) -> Access {
    Access::new(kind, AccessMode::Supervisor)
}

fn user(kind: AccessKind) -> Access {
    Access::new(kind, AccessMode::User)
}

/// A second access to the same page is answered from the TLB: the paging
/// structures are not read again (SDM §4.10.2.3 — "the processor may not
/// actually consult the paging structures in memory").
#[test]
fn a_cached_translation_does_not_reread_the_paging_structures() {
    let mut tables = PageTables::new();
    tables.map_4kib(LINEAR, FRAME, ENTRY_RW, ENTRY_RW);
    let ctx = context(&tables, 0, 0);
    let mut mmu = Mmu::new();

    let first = mmu
        .translate(&ctx, &mut tables.mem, LINEAR, supervisor(AccessKind::Read))
        .expect("miss");
    assert_eq!(mmu.tlb().len(), 1);
    tables.mem.clear_log();

    let second = mmu
        .translate(&ctx, &mut tables.mem, LINEAR, supervisor(AccessKind::Read))
        .expect("hit");
    assert!(tables.mem.reads.is_empty(), "read {:?}", tables.mem.reads);
    assert!(tables.mem.writes.is_empty());
    assert_eq!(first, second);
    assert_eq!(second.phys_addr, u64::from(FRAME));
}

/// The point of the model: a paging-structure edit without an invalidation is
/// not visible, and `INVLPG` makes it visible (SDM §4.10.4.1, §4.10.4.2).
#[test]
fn a_stale_entry_survives_until_invlpg() {
    let mut tables = PageTables::new();
    tables.map_4kib(LINEAR, FRAME, ENTRY_RW, ENTRY_RW);
    let ctx = context(&tables, 0, 0);
    let mut mmu = Mmu::new();

    let original = mmu
        .translate(&ctx, &mut tables.mem, LINEAR, supervisor(AccessKind::Read))
        .expect("miss")
        .phys_addr;

    // Repoint the page somewhere else, without telling the processor.
    tables.set_pte(LINEAR, 0x0099_0000 | ENTRY_RW | 1);
    let stale = mmu
        .translate(&ctx, &mut tables.mem, LINEAR, supervisor(AccessKind::Read))
        .expect("hit")
        .phys_addr;
    assert_eq!(stale, original, "the stale translation must still be used");

    mmu.invlpg(LINEAR);
    let fresh = mmu
        .translate(&ctx, &mut tables.mem, LINEAR, supervisor(AccessKind::Read))
        .expect("miss")
        .phys_addr;
    assert_eq!(fresh, 0x0099_0000);
}

/// `INVLPG` takes a single linear address and leaves other pages cached.
#[test]
fn invlpg_invalidates_only_the_named_page() {
    let mut tables = PageTables::new();
    tables.map_4kib(LINEAR, FRAME, ENTRY_RW, ENTRY_RW);
    tables.map_4kib(OTHER, 0x0055_0000, ENTRY_RW, ENTRY_RW);
    let ctx = context(&tables, 0, 0);
    let mut mmu = Mmu::new();

    mmu.translate(&ctx, &mut tables.mem, LINEAR, supervisor(AccessKind::Read))
        .unwrap();
    mmu.translate(&ctx, &mut tables.mem, OTHER, supervisor(AccessKind::Read))
        .unwrap();
    assert_eq!(mmu.tlb().len(), 2);

    mmu.invlpg(LINEAR);
    assert!(mmu.tlb().lookup(LINEAR).is_none());
    assert!(mmu.tlb().lookup(OTHER).is_some());

    // Invalidating an address with no entry is harmless.
    mmu.invlpg(0x7777_0000);
    assert_eq!(mmu.tlb().len(), 1);
}

/// SDM §4.10.4.1 footnote: if the page is larger than 4 KiB and several entries
/// exist for it, `INVLPG` invalidates all of them — one execution suffices even
/// for a 4-MiB page.
#[test]
fn invlpg_on_a_large_page_invalidates_every_cached_subpage() {
    let region = 0x0400_0000;
    let mut tables = PageTables::new();
    tables.map_4mib(region, 0x0080_0000, ENTRY_RW);
    let ctx = context(&tables, 0, CR4_PSE);
    let mut mmu = Mmu::new();

    for step in 0..4u32 {
        let linear = region + step * 0x1000;
        let translation = mmu
            .translate(&ctx, &mut tables.mem, linear, supervisor(AccessKind::Read))
            .expect("translation");
        assert_eq!(translation.page_size, PageSize::Size4MiB);
    }
    assert_eq!(mmu.tlb().len(), 4);

    // One INVLPG, naming a different sub-page of the same 4-MiB page.
    mmu.invlpg(region + 0x0020_0000);
    assert!(mmu.tlb().is_empty());
}

/// SDM §4.10.4.1: `MOV to CR3` invalidates every entry except those for global
/// pages, and §4.10.2.4 makes an entry global only when `G = 1` **and**
/// `CR4.PGE = 1`.
#[test]
fn mov_to_cr3_spares_global_entries_only_when_pge_is_set() {
    let mut tables = PageTables::new();
    tables.map_4kib(LINEAR, FRAME, ENTRY_RW, ENTRY_RW | ENTRY_G);
    tables.map_4kib(OTHER, 0x0055_0000, ENTRY_RW, ENTRY_RW);

    // With CR4.PGE set, the G page survives a CR3 load.
    let ctx = context(&tables, 0, CR4_PGE);
    let mut mmu = Mmu::new();
    let global = mmu
        .translate(&ctx, &mut tables.mem, LINEAR, supervisor(AccessKind::Read))
        .unwrap();
    assert!(global.global);
    mmu.translate(&ctx, &mut tables.mem, OTHER, supervisor(AccessKind::Read))
        .unwrap();

    mmu.on_mov_to_cr3(ctx.cr3);
    assert!(mmu.tlb().lookup(LINEAR).is_some(), "global entry survives");
    assert!(mmu.tlb().lookup(OTHER).is_none());

    // With CR4.PGE clear the same G bit means nothing, so a CR3 load is a full
    // flush.
    let ctx = context(&tables, 0, 0);
    let mut mmu = Mmu::new();
    let not_global = mmu
        .translate(&ctx, &mut tables.mem, LINEAR, supervisor(AccessKind::Read))
        .unwrap();
    assert!(!not_global.global);
    mmu.on_mov_to_cr3(ctx.cr3);
    assert!(mmu.tlb().is_empty(), "no entry is global without CR4.PGE");
}

/// SDM §4.10.4.1: `MOV to CR4` that changes `CR4.PGE` invalidates all entries,
/// global ones included, in either direction.
#[test]
fn changing_cr4_pge_flushes_global_entries_too() {
    let mut tables = PageTables::new();
    tables.map_4kib(LINEAR, FRAME, ENTRY_RW, ENTRY_RW | ENTRY_G);
    let ctx = context(&tables, 0, CR4_PGE);
    let mut mmu = Mmu::new();

    mmu.translate(&ctx, &mut tables.mem, LINEAR, supervisor(AccessKind::Read))
        .unwrap();
    mmu.on_mov_to_cr4(CR4_PGE, 0);
    assert!(mmu.tlb().is_empty());

    mmu.translate(&ctx, &mut tables.mem, LINEAR, supervisor(AccessKind::Read))
        .unwrap();
    mmu.on_mov_to_cr4(0, CR4_PGE);
    assert!(mmu.tlb().is_empty());
}

/// SDM §4.10.4.1: `MOV to CR4` "may invalidate TLB entries when changing
/// CR4.PSE"; this model always does, because the change reinterprets every PS
/// bit. `MOV to CR0` clearing `CR0.PG` invalidates everything.
#[test]
fn cr4_pse_and_cr0_pg_changes_flush() {
    let mut tables = PageTables::new();
    tables.map_4kib(LINEAR, FRAME, ENTRY_RW, ENTRY_RW);
    let ctx = context(&tables, 0, 0);
    let mut mmu = Mmu::new();

    mmu.translate(&ctx, &mut tables.mem, LINEAR, supervisor(AccessKind::Read))
        .unwrap();
    mmu.on_mov_to_cr4(0, CR4_PSE);
    assert!(mmu.tlb().is_empty());

    mmu.translate(&ctx, &mut tables.mem, LINEAR, supervisor(AccessKind::Read))
        .unwrap();
    mmu.on_mov_to_cr0(CR0_PG, 0);
    assert!(mmu.tlb().is_empty());

    // Setting CR0.WP requires no invalidation: the entry caches the combined
    // R/W flag and CR0.WP is applied per access.
    mmu.translate(&ctx, &mut tables.mem, LINEAR, supervisor(AccessKind::Read))
        .unwrap();
    mmu.on_mov_to_cr0(CR0_PG, CR0_PG | CR0_WP);
    assert_eq!(mmu.tlb().len(), 1);
}

/// `CR0.WP` is honored on a TLB hit, without any flush: the same cached entry
/// permits a supervisor write with `WP = 0` and denies it with `WP = 1`.
#[test]
fn cr0_wp_is_applied_per_access_on_a_tlb_hit() {
    let mut tables = PageTables::new();
    tables.map_4kib(LINEAR, FRAME, 0, 0); // read-only supervisor page
    let mut mmu = Mmu::new();

    let permissive = context(&tables, 0, 0);
    mmu.translate(
        &permissive,
        &mut tables.mem,
        LINEAR,
        supervisor(AccessKind::Write),
    )
    .expect("WP = 0 permits the write");
    assert_eq!(mmu.tlb().len(), 1);

    let protected = context(&tables, CR0_WP, 0);
    let fault = mmu
        .translate(
            &protected,
            &mut tables.mem,
            LINEAR,
            supervisor(AccessKind::Write),
        )
        .expect_err("WP = 1 denies it")
        .as_fault()
        .unwrap();
    assert_eq!(fault.reason, FaultReason::Protection);
    assert_eq!(fault.error_code(), PF_ERR_P | PF_ERR_WR);
}

/// SDM §4.10.4.1: "page faults invalidate entries in the TLBs ... These
/// invalidations ensure that the page-fault exception will not recur (if the
/// faulting instruction is re-executed) if it would not be caused by the
/// contents of the paging structures in memory."
#[test]
fn a_fault_through_a_cached_entry_invalidates_it() {
    let mut tables = PageTables::new();
    // Writable PDE, read-only PTE: the combined R/W flag is 0.
    tables.map_4kib(LINEAR, FRAME, ENTRY_US | ENTRY_RW, ENTRY_US);
    let ctx = context(&tables, 0, 0);
    let mut mmu = Mmu::new();

    mmu.translate(&ctx, &mut tables.mem, LINEAR, user(AccessKind::Read))
        .expect("user read is permitted");
    assert_eq!(mmu.tlb().len(), 1);

    let fault = mmu
        .translate(&ctx, &mut tables.mem, LINEAR, user(AccessKind::Write))
        .expect_err("read-only")
        .as_fault()
        .unwrap();
    assert_eq!(fault.error_code(), PF_ERR_P | PF_ERR_WR | PF_ERR_US);
    assert!(mmu.tlb().is_empty(), "the faulting page is invalidated");

    // Software makes the page writable and re-executes; without the
    // invalidation above the stale read-only entry would fault again.
    tables.set_pte(LINEAR, FRAME | ENTRY_US | ENTRY_RW | 1);
    mmu.translate(&ctx, &mut tables.mem, LINEAR, user(AccessKind::Write))
        .expect("now writable");
}

/// A translation that faults is never cached (SDM §4.10.2.3: an entry exists
/// only for a page number that has a translation).
#[test]
fn a_faulting_translation_caches_nothing() {
    let mut tables = PageTables::new();
    let ctx = context(&tables, 0, 0);
    let mut mmu = Mmu::new();

    mmu.translate(&ctx, &mut tables.mem, LINEAR, supervisor(AccessKind::Read))
        .expect_err("nothing mapped");
    assert!(mmu.tlb().is_empty());
}

/// SDM §4.10.2.2 has the TLB cache the dirty flag precisely so that a write
/// through a cached translation can still set it in memory (§4.8) — and set it
/// only once.
#[test]
fn a_write_through_a_cached_translation_still_sets_dirty() {
    let mut tables = PageTables::new();
    tables.map_4kib(LINEAR, FRAME, ENTRY_RW, ENTRY_RW);
    let ctx = context(&tables, 0, 0);
    let mut mmu = Mmu::new();

    // Cache the translation with a read, so the entry is cached with D = 0.
    mmu.translate(&ctx, &mut tables.mem, LINEAR, supervisor(AccessKind::Read))
        .unwrap();
    assert_ne!(tables.pte(LINEAR) & ENTRY_A, 0);
    assert_eq!(tables.pte(LINEAR) & ENTRY_D, 0);
    tables.mem.clear_log();

    let translation = mmu
        .translate(&ctx, &mut tables.mem, LINEAR, supervisor(AccessKind::Write))
        .expect("write hits the TLB");
    assert!(translation.dirty);
    assert_ne!(tables.pte(LINEAR) & ENTRY_D, 0);
    assert_eq!(tables.mem.writes.len(), 1);
    assert_eq!(tables.mem.writes[0].0, tables.pte_addr(LINEAR));

    // A second write finds the cached dirty flag set and writes nothing.
    tables.mem.clear_log();
    mmu.translate(&ctx, &mut tables.mem, LINEAR, supervisor(AccessKind::Write))
        .unwrap();
    assert!(tables.mem.writes.is_empty());
}

/// The polled equivalent of the `on_mov_to_*` hooks: an integration that
/// changes a control register somewhere the hooks do not cover still gets the
/// architectural invalidation.
#[test]
fn sync_control_registers_applies_the_same_invalidations() {
    let mut tables = PageTables::new();
    tables.map_4kib(LINEAR, FRAME, ENTRY_RW, ENTRY_RW | ENTRY_G);
    let mut mmu = Mmu::new();

    let ctx = context(&tables, 0, CR4_PGE);
    mmu.sync_control_registers(&ctx);
    mmu.translate(&ctx, &mut tables.mem, LINEAR, supervisor(AccessKind::Read))
        .unwrap();
    assert_eq!(mmu.tlb().len(), 1);

    // Same registers: nothing happens.
    mmu.sync_control_registers(&ctx);
    assert_eq!(mmu.tlb().len(), 1);

    // A CR3 change spares the global entry.
    let moved = PagingContext::new(CR0_PG, 0x0002_F000, CR4_PGE);
    mmu.sync_control_registers(&moved);
    assert_eq!(mmu.tlb().len(), 1);

    // Clearing CR4.PGE takes it out.
    let no_pge = PagingContext::new(CR0_PG, 0x0002_F000, 0);
    mmu.sync_control_registers(&no_pge);
    assert!(mmu.tlb().is_empty());
}

/// Translation through the TLB and through a bare walk agree, page for page,
/// over a mixed set of mappings and accesses.
#[test]
fn tlb_path_and_walk_path_agree() {
    let mut tables = PageTables::new();
    tables.map_4kib(LINEAR, FRAME, ENTRY_RW | ENTRY_US, ENTRY_RW | ENTRY_US);
    tables.map_4kib(OTHER, 0x0055_0000, ENTRY_RW, ENTRY_RW);
    tables.map_4mib(0x0400_0000, 0x0080_0000, ENTRY_RW | ENTRY_US);
    let ctx = context(&tables, CR0_WP, CR4_PSE);

    let addresses = [LINEAR, LINEAR + 0x40, OTHER, 0x0400_1234, 0x0777_0000];
    let accesses = [
        supervisor(AccessKind::Read),
        supervisor(AccessKind::Write),
        supervisor(AccessKind::InstructionFetch),
        user(AccessKind::Read),
        user(AccessKind::Write),
    ];

    for &linear in &addresses {
        for &access in &accesses {
            let mut direct_tables = PageTables::new();
            direct_tables.map_4kib(LINEAR, FRAME, ENTRY_RW | ENTRY_US, ENTRY_RW | ENTRY_US);
            direct_tables.map_4kib(OTHER, 0x0055_0000, ENTRY_RW, ENTRY_RW);
            direct_tables.map_4mib(0x0400_0000, 0x0080_0000, ENTRY_RW | ENTRY_US);

            let direct = x86_mmu::paging::translate(&ctx, &mut direct_tables.mem, linear, access);
            let mut mmu = Mmu::new();
            let cached = mmu.translate(&ctx, &mut tables.mem, linear, access);

            match (direct, cached) {
                (Ok(a), Ok(b)) => {
                    assert_eq!(a.phys_addr, b.phys_addr, "{linear:#x} {access:?}");
                    assert_eq!(a.writable, b.writable);
                    assert_eq!(a.user_accessible, b.user_accessible);
                }
                (Err(a), Err(b)) => assert_eq!(a, b, "{linear:#x} {access:?}"),
                (a, b) => panic!("{linear:#x} {access:?}: {a:?} vs {b:?}"),
            }
        }
    }
}
