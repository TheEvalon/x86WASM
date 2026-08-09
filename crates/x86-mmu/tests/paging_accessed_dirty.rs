//! Accessed and dirty flags, and their ordering against fault detection.
//!
//! Spec: Intel SDM Vol. 3 §4.8 "Accessed and Dirty Flags" (A set whenever an
//! entry is used for translation, D set in the entry that identifies the final
//! physical address whenever there is a write, both sticky), Table 4-5 (bit 6
//! of a PDE that references a page table is ignored), §4.10.2.3 (accessed flags
//! are set before a translation is cached).

mod common;

use common::PageTables;
use x86_mmu::paging::entry::{ENTRY_A, ENTRY_D, ENTRY_RW, ENTRY_US, PTE_PAT};
use x86_mmu::paging::{
    translate, Access, AccessKind, AccessMode, PagingContext, CR0_PG, CR0_WP, CR4_PSE,
};

const LINEAR: u32 = 0x0204_2000;
const FRAME: u32 = 0x0055_0000;

fn context(tables: &PageTables, cr0_extra: u64, cr4: u64) -> PagingContext {
    PagingContext::new(CR0_PG | cr0_extra, tables.pd_base, cr4)
}

fn supervisor(kind: AccessKind) -> Access {
    Access::new(kind, AccessMode::Supervisor)
}

fn user(kind: AccessKind) -> Access {
    Access::new(kind, AccessMode::User)
}

/// SDM §4.8: a read sets A in every entry used and sets no dirty flag.
#[test]
fn read_sets_accessed_in_every_entry_used() {
    let mut tables = PageTables::new();
    tables.map_4kib(LINEAR, FRAME, ENTRY_RW, ENTRY_RW);
    let ctx = context(&tables, 0, 0);

    let translation = translate(&ctx, &mut tables.mem, LINEAR, supervisor(AccessKind::Read))
        .expect("translation");

    assert_ne!(tables.pde(LINEAR) & ENTRY_A, 0);
    assert_ne!(tables.pte(LINEAR) & ENTRY_A, 0);
    assert_eq!(tables.pte(LINEAR) & ENTRY_D, 0);
    assert!(!translation.dirty);
}

/// SDM §4.8: an instruction fetch is a use of the entries, so it sets A, but it
/// is not a write, so it sets no D.
#[test]
fn instruction_fetch_sets_accessed_but_not_dirty() {
    let mut tables = PageTables::new();
    tables.map_4kib(LINEAR, FRAME, ENTRY_RW, ENTRY_RW);
    let ctx = context(&tables, 0, 0);

    translate(
        &ctx,
        &mut tables.mem,
        LINEAR,
        supervisor(AccessKind::InstructionFetch),
    )
    .expect("translation");

    assert_ne!(tables.pte(LINEAR) & ENTRY_A, 0);
    assert_eq!(tables.pte(LINEAR) & ENTRY_D, 0);
}

/// SDM §4.8 and Table 4-5: a write sets D in the PTE — the entry that
/// identifies the final physical address — and never in the PDE that merely
/// references the page table, where bit 6 is ignored.
#[test]
fn write_sets_dirty_in_the_pte_only() {
    let mut tables = PageTables::new();
    tables.map_4kib(LINEAR, FRAME, ENTRY_RW, ENTRY_RW);
    let ctx = context(&tables, CR0_WP, 0);

    let translation = translate(&ctx, &mut tables.mem, LINEAR, supervisor(AccessKind::Write))
        .expect("translation");

    assert_ne!(tables.pde(LINEAR) & ENTRY_A, 0);
    assert_eq!(tables.pde(LINEAR) & ENTRY_D, 0, "PDE bit 6 is ignored");
    assert_ne!(tables.pte(LINEAR) & ENTRY_A, 0);
    assert_ne!(tables.pte(LINEAR) & ENTRY_D, 0);
    assert!(translation.dirty);
}

/// An entry that needs both flags gets one write, not two: the SDM describes a
/// single locked update of the entry.
#[test]
fn accessed_and_dirty_land_in_one_write_per_entry() {
    let mut tables = PageTables::new();
    tables.map_4kib(LINEAR, FRAME, ENTRY_RW, ENTRY_RW);
    let ctx = context(&tables, 0, 0);
    tables.mem.clear_log();

    translate(&ctx, &mut tables.mem, LINEAR, supervisor(AccessKind::Write)).expect("translation");

    assert_eq!(tables.mem.writes.len(), 2, "one PDE write, one PTE write");
    let (pte_addr, pte_value) = tables.mem.writes[1];
    assert_eq!(pte_addr, tables.pte_addr(LINEAR));
    assert_ne!(pte_value & (ENTRY_A | ENTRY_D), 0);
}

/// SDM §4.8: the flags are sticky, so an entry that already has them needs no
/// write at all.
#[test]
fn sticky_flags_are_not_rewritten() {
    let mut tables = PageTables::new();
    tables.map_4kib(
        LINEAR,
        FRAME,
        ENTRY_RW | ENTRY_A,
        ENTRY_RW | ENTRY_A | ENTRY_D,
    );
    let ctx = context(&tables, 0, 0);
    tables.mem.clear_log();

    translate(&ctx, &mut tables.mem, LINEAR, supervisor(AccessKind::Write)).expect("translation");

    assert!(
        tables.mem.writes.is_empty(),
        "rewrote {:?}",
        tables.mem.writes
    );
}

/// A read of a page whose PTE is already dirty leaves the dirty flag alone and
/// reports it — the flags are only ever set, never cleared, by the processor.
#[test]
fn a_read_does_not_clear_an_existing_dirty_flag() {
    let mut tables = PageTables::new();
    tables.map_4kib(LINEAR, FRAME, ENTRY_RW, ENTRY_RW | ENTRY_D);
    let ctx = context(&tables, 0, 0);

    let translation = translate(&ctx, &mut tables.mem, LINEAR, supervisor(AccessKind::Read))
        .expect("translation");

    assert!(translation.dirty);
    assert_ne!(tables.pte(LINEAR) & ENTRY_D, 0);
}

/// The ordering rule that matters: a fault at the *page-table* level must not
/// leave the accessed flag set in the page-directory entry that was read on the
/// way there.
#[test]
fn a_fault_at_the_pte_leaves_no_accessed_flag_in_the_pde() {
    let mut tables = PageTables::new();
    tables.ensure_page_table(LINEAR, ENTRY_RW);
    tables.mem.clear_log();
    let ctx = context(&tables, 0, 0);

    translate(&ctx, &mut tables.mem, LINEAR, supervisor(AccessKind::Write))
        .expect_err("no PTE installed");

    assert!(
        tables.mem.writes.is_empty(),
        "faulting walk wrote {:?}",
        tables.mem.writes
    );
    assert_eq!(tables.pde(LINEAR) & ENTRY_A, 0);
}

/// The same for a fault at the page-directory level and for a reserved-bit
/// fault: no paging-structure write of any kind.
#[test]
fn not_present_and_reserved_bit_faults_write_nothing() {
    let mut tables = PageTables::new();
    let ctx = context(&tables, 0, 0);
    translate(&ctx, &mut tables.mem, LINEAR, supervisor(AccessKind::Write))
        .expect_err("no PDE installed");
    assert!(tables.mem.writes.is_empty());

    let mut tables = PageTables::new();
    tables.map_4kib(LINEAR, FRAME, ENTRY_RW, ENTRY_RW | PTE_PAT);
    tables.mem.clear_log();
    let ctx = context(&tables, 0, CR4_PSE);
    translate(&ctx, &mut tables.mem, LINEAR, supervisor(AccessKind::Write))
        .expect_err("reserved bit");
    assert!(
        tables.mem.writes.is_empty(),
        "reserved-bit fault wrote {:?}",
        tables.mem.writes
    );
    assert_eq!(tables.pde(LINEAR) & ENTRY_A, 0);
}

/// A protection violation happens after the walk has already produced a
/// translation, and still must not set A or D. This is the case a naive
/// implementation gets wrong, because the entries really were read.
#[test]
fn a_protection_violation_writes_nothing() {
    // User write to a read-only user page.
    let mut tables = PageTables::new();
    tables.map_4kib(LINEAR, FRAME, ENTRY_US, ENTRY_US);
    tables.mem.clear_log();
    let ctx = context(&tables, 0, 0);
    translate(&ctx, &mut tables.mem, LINEAR, user(AccessKind::Write)).expect_err("read-only");
    assert!(
        tables.mem.writes.is_empty(),
        "protection fault wrote {:?}",
        tables.mem.writes
    );
    assert_eq!(tables.pde(LINEAR) & ENTRY_A, 0);
    assert_eq!(tables.pte(LINEAR) & (ENTRY_A | ENTRY_D), 0);

    // Supervisor write to a read-only page with CR0.WP = 1.
    let mut tables = PageTables::new();
    tables.map_4kib(LINEAR, FRAME, 0, 0);
    tables.mem.clear_log();
    let ctx = context(&tables, CR0_WP, 0);
    translate(&ctx, &mut tables.mem, LINEAR, supervisor(AccessKind::Write))
        .expect_err("CR0.WP denies the write");
    assert!(tables.mem.writes.is_empty());

    // The same page with CR0.WP = 0 is permitted, and *then* the flags move.
    let ctx = context(&tables, 0, 0);
    translate(&ctx, &mut tables.mem, LINEAR, supervisor(AccessKind::Write)).expect("permitted");
    assert_ne!(tables.pte(LINEAR) & (ENTRY_A | ENTRY_D), 0);
}

/// A user read of a supervisor page is denied, so neither flag moves, but the
/// same page read from supervisor mode does set them.
#[test]
fn denied_user_read_leaves_flags_for_the_later_supervisor_read() {
    let mut tables = PageTables::new();
    tables.map_4kib(LINEAR, FRAME, ENTRY_RW, ENTRY_RW);
    let ctx = context(&tables, 0, 0);

    translate(&ctx, &mut tables.mem, LINEAR, user(AccessKind::Read)).expect_err("supervisor page");
    assert_eq!(tables.pte(LINEAR) & ENTRY_A, 0);

    translate(&ctx, &mut tables.mem, LINEAR, supervisor(AccessKind::Read)).expect("permitted");
    assert_ne!(tables.pte(LINEAR) & ENTRY_A, 0);
}
