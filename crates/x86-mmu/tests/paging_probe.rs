//! `Mmu::probe`: "would this access translate?" with no side effect.
//!
//! A caller needs this when one architectural access spans two pages, or when
//! an unrepeatable side effect has to be ordered after the guarantee that a
//! store can happen. The property that makes it usable is that it answers the
//! same question the real access would, and changes nothing while doing it.
//!
//! Spec: Intel SDM Vol. 3 §4.6.1 (access rights), §4.7 (fault reasons), §4.8
//! (the flags a real access writes), §4.10.2.2/.3 (TLB contents; a translation
//! may be cached only once its accessed flags are set), §4.10.4.1 (a page
//! fault invalidates the entry for the faulting page).

mod common;

use common::*;
use x86_mmu::paging::{
    Access, AccessKind, AccessMode, FaultReason, Mmu, PagingContext, PagingLevel, TranslateError,
    CR0_PG, CR0_WP,
};

const RW: u32 = 1 << 1;
const US: u32 = 1 << 2;
const A: u32 = 1 << 5;
const D: u32 = 1 << 6;

fn supervisor(kind: AccessKind) -> Access {
    Access::new(kind, AccessMode::Supervisor)
}

/// A probe of a permitted access succeeds and writes nothing: no accessed
/// flag, no dirty flag, no cached translation. A real access afterwards is
/// what sets them.
#[test]
fn a_successful_probe_writes_no_flag_and_caches_nothing() {
    let mut tables = PageTables::new();
    tables.map_4kib(0x1000, 0x8000, RW | US, RW | US);
    let ctx = PagingContext::new(CR0_PG, PD_BASE, 0);
    let mut mmu = Mmu::new();
    tables.mem.clear_log();

    mmu.probe(&ctx, &mut tables.mem, 0x1000, supervisor(AccessKind::Write))
        .expect("the page is present and writable");

    assert!(tables.mem.writes.is_empty(), "no paging-structure write");
    assert_eq!(tables.pte(0x1000) & (A | D), 0);
    assert!(mmu.tlb().is_empty(), "a probe may not cache a translation");

    mmu.translate(&ctx, &mut tables.mem, 0x1000, supervisor(AccessKind::Write))
        .expect("the real access still works");
    assert_eq!(tables.pte(0x1000) & (A | D), A | D, "the access sets them");
    assert_eq!(mmu.tlb().len(), 1);
}

/// A probe reports the same fault a real access would, with the same error
/// code, and still writes nothing.
#[test]
fn a_probe_reports_the_faults_a_real_access_would() {
    let mut tables = PageTables::new();
    tables.map_4kib(0x1000, 0x8000, RW | US, US); // present, read-only
    tables.ensure_page_table(0x2000, RW | US);
    tables.set_pte(0x2000, 0); // not present
    let ctx = PagingContext::new(CR0_PG | CR0_WP, PD_BASE, 0);
    let mut mmu = Mmu::new();
    tables.mem.clear_log();

    let denied = mmu
        .probe(&ctx, &mut tables.mem, 0x1000, supervisor(AccessKind::Write))
        .unwrap_err();
    match denied {
        TranslateError::Fault(fault) => {
            assert_eq!(fault.reason, FaultReason::Protection);
            assert_eq!(fault.error_code(), 0x3, "P=1, W/R=1, U/S=0");
        }
        other => panic!("expected a protection fault, got {other:?}"),
    }

    let absent = mmu
        .probe(&ctx, &mut tables.mem, 0x2000, supervisor(AccessKind::Read))
        .unwrap_err();
    match absent {
        TranslateError::Fault(fault) => {
            assert_eq!(
                fault.reason,
                FaultReason::NotPresent(PagingLevel::PageTable)
            );
            assert_eq!(fault.error_code(), 0x0);
        }
        other => panic!("expected a not-present fault, got {other:?}"),
    }

    assert!(
        tables.mem.writes.is_empty(),
        "a faulting probe writes nothing"
    );
}

/// The probe consults the TLB, because the access it stands in for would.
/// A stale cached translation must make the probe agree with the access, not
/// with the paging structures in memory.
#[test]
fn a_probe_sees_what_the_tlb_sees() {
    let mut tables = PageTables::new();
    tables.map_4kib(0x1000, 0x8000, RW | US, RW | US);
    let ctx = PagingContext::new(CR0_PG | CR0_WP, PD_BASE, 0);
    let mut mmu = Mmu::new();

    mmu.translate(&ctx, &mut tables.mem, 0x1000, supervisor(AccessKind::Read))
        .expect("caches a writable translation");

    // Revoke write permission in memory without invalidating.
    tables.set_pte(0x1000, 0x8000 | US | A | 1);
    mmu.probe(&ctx, &mut tables.mem, 0x1000, supervisor(AccessKind::Write))
        .expect("the stale entry still permits the write");

    mmu.invlpg(0x1000);
    let denied = mmu.probe(&ctx, &mut tables.mem, 0x1000, supervisor(AccessKind::Write));
    assert!(denied.is_err(), "after INVLPG the probe sees memory");
}

/// A probe that faults performs the same invalidation a page fault does
/// (§4.10.4.1), so a stale entry cannot make the retry fault again after
/// software has repaired the mapping.
#[test]
fn a_faulting_probe_invalidates_the_page() {
    let mut tables = PageTables::new();
    tables.map_4kib(0x1000, 0x8000, RW | US, US); // read-only
    let ctx = PagingContext::new(CR0_PG | CR0_WP, PD_BASE, 0);
    let mut mmu = Mmu::new();

    mmu.translate(&ctx, &mut tables.mem, 0x1000, supervisor(AccessKind::Read))
        .expect("caches a read-only translation");
    assert_eq!(mmu.tlb().len(), 1);

    mmu.probe(&ctx, &mut tables.mem, 0x1000, supervisor(AccessKind::Write))
        .expect_err("write to a read-only page with CR0.WP=1");
    assert!(mmu.tlb().is_empty(), "the faulting page is invalidated");
}

/// With paging off there is nothing to probe, and the engine says so rather
/// than pretending an identity mapping exists (SDM Vol. 3 §4.1.1).
#[test]
fn probing_with_paging_disabled_reports_unsupported() {
    let mut tables = PageTables::new();
    let ctx = PagingContext::new(0, PD_BASE, 0);
    let mut mmu = Mmu::new();

    let result = mmu.probe(&ctx, &mut tables.mem, 0x1000, supervisor(AccessKind::Read));
    assert!(matches!(result, Err(TranslateError::Unsupported(_))));
}
