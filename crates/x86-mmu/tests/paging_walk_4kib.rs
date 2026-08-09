//! 32-bit two-level page walk for 4-KiB pages.
//!
//! Spec: Intel SDM Vol. 3 §4.3 "32-Bit Paging" (PDE/PTE selection and the final
//! physical address), Table 4-3 (use of CR3), Tables 4-5 and 4-6 (entry
//! formats), §4.7 (a paging-structure entry with P = 0 or a reserved bit set
//! yields no translation).

mod common;

use common::PageTables;
use x86_mmu::paging::entry::{ENTRY_A, ENTRY_P, ENTRY_RW, ENTRY_US, PDE_PS, PTE_PAT};
use x86_mmu::paging::{
    walk, FaultReason, PageSize, PagingContext, PagingLevel, UnsupportedPaging, WalkError, CR0_PG,
    CR4_PAE, CR4_PSE,
};

fn ctx(tables: &PageTables, cr4: u64) -> PagingContext {
    PagingContext::new(CR0_PG, tables.pd_base, cr4)
}

/// SDM §4.3: the PDE comes from linear bits 31:22, the PTE from bits 21:12, and
/// the final physical address is PTE bits 31:12 plus linear bits 11:0.
#[test]
fn two_level_walk_resolves_a_4kib_page() {
    let mut tables = PageTables::new();
    let linear = 0x1234_5678;
    tables.map_4kib(linear, 0x00AB_C000, ENTRY_RW, ENTRY_RW);

    let ctx = ctx(&tables, 0);
    let result = walk(&ctx, &mut tables.mem, linear).expect("translation");

    assert_eq!(result.page_size, PageSize::Size4KiB);
    assert_eq!(result.frame_base, 0x00AB_C000);
    assert_eq!(result.phys_addr, 0x00AB_C678);
    assert_eq!(result.linear_address, linear);
    assert_eq!(result.pde_addr, tables.pd_base + 4 * 0x48);
    let (pte_addr, _) = result.pte.expect("4-KiB translation uses a PTE");
    assert_eq!(pte_addr, tables.pte_addr(linear));
}

/// Two addresses in the same 4-MiB region share a PDE and differ only in the
/// PTE index (SDM §4.3: a PDE controls a 4-MiB region of the linear-address
/// space).
#[test]
fn pde_covers_a_4mib_region_of_linear_space() {
    let mut tables = PageTables::new();
    let low = 0x0080_0000;
    let high = low + 0x0010_0000;
    tables.map_4kib(low, 0x0010_0000, ENTRY_RW, ENTRY_RW);
    tables.map_4kib(high, 0x0020_0000, ENTRY_RW, ENTRY_RW);

    let ctx = ctx(&tables, 0);
    let a = walk(&ctx, &mut tables.mem, low).expect("low");
    let b = walk(&ctx, &mut tables.mem, high).expect("high");

    assert_eq!(a.pde_addr, b.pde_addr);
    assert_ne!(a.pte.unwrap().0, b.pte.unwrap().0);
    assert_eq!(a.phys_addr, 0x0010_0000);
    assert_eq!(b.phys_addr, 0x0020_0000);
}

/// SDM Table 4-3: only CR3 bits 31:12 locate the page directory; bits 11:0 and
/// 63:32 are ignored with 32-bit paging.
#[test]
fn cr3_low_and_high_bits_are_ignored() {
    let mut tables = PageTables::new();
    let linear = 0x0000_2000;
    tables.map_4kib(linear, 0x0030_0000, ENTRY_RW, ENTRY_RW);

    let noisy_cr3 = 0xFFFF_FFFF_0000_0000 | tables.pd_base | 0xFFF;
    let ctx = PagingContext::new(CR0_PG, noisy_cr3, 0);
    assert!(ctx.cr3_write_through());
    assert!(ctx.cr3_cache_disable());

    let result = walk(&ctx, &mut tables.mem, linear).expect("translation");
    assert_eq!(result.phys_addr, 0x0030_0000);
}

/// SDM §4.3 / §4.7: a PDE with P = 0 yields no translation.
#[test]
fn absent_pde_faults_at_the_directory_level() {
    let mut tables = PageTables::new();
    let linear = 0x0040_1000;

    let ctx = ctx(&tables, 0);
    let err = walk(&ctx, &mut tables.mem, linear).expect_err("no PDE installed");
    assert_eq!(
        err.as_fault_reason().expect("architectural fault"),
        FaultReason::NotPresent(PagingLevel::PageDirectory)
    );
}

/// SDM §4.3 / §4.7: a PTE with P = 0 yields no translation, and the walk
/// reports the level it failed at.
#[test]
fn absent_pte_faults_at_the_table_level() {
    let mut tables = PageTables::new();
    let linear = 0x0040_1000;
    tables.ensure_page_table(linear, ENTRY_RW);

    let ctx = ctx(&tables, 0);
    let err = walk(&ctx, &mut tables.mem, linear).expect_err("no PTE installed");
    assert_eq!(
        err.as_fault_reason().unwrap(),
        FaultReason::NotPresent(PagingLevel::PageTable)
    );
}

/// SDM §4.7, RSVD note: "reserved bits are not checked in a paging-structure
/// entry whose P flag is 0", so an absent entry that happens to set one still
/// reports a not-present fault.
#[test]
fn reserved_bits_are_not_checked_in_an_absent_entry() {
    let mut tables = PageTables::new();
    let linear = 0x0040_1000;
    tables.ensure_page_table(linear, ENTRY_RW);
    tables.set_pte(linear, PTE_PAT); // P = 0, bit 7 set

    let ctx = ctx(&tables, CR4_PSE);
    let err = walk(&ctx, &mut tables.mem, linear).expect_err("absent PTE");
    assert_eq!(
        err.as_fault_reason().unwrap(),
        FaultReason::NotPresent(PagingLevel::PageTable)
    );
}

/// SDM §4.3: "With 32-bit paging, there are reserved bits only if CR4.PSE = 1
/// ... If the PAT is not supported: if the P flag of a PTE is 1, bit 7 is
/// reserved." The same entry translates without complaint when CR4.PSE = 0.
#[test]
fn pte_bit7_is_reserved_only_when_cr4_pse_is_set() {
    let mut tables = PageTables::new();
    let linear = 0x0040_1000;
    tables.map_4kib(linear, 0x0050_0000, ENTRY_RW, ENTRY_RW | PTE_PAT);

    let with_pse = ctx(&tables, CR4_PSE);
    let err = walk(&with_pse, &mut tables.mem, linear).expect_err("reserved bit");
    assert_eq!(
        err.as_fault_reason().unwrap(),
        FaultReason::ReservedBit(PagingLevel::PageTable)
    );

    let without_pse = ctx(&tables, 0);
    let ok = walk(&without_pse, &mut tables.mem, linear).expect("bit 7 is ignored");
    assert_eq!(ok.phys_addr, 0x0050_0000);
}

/// SDM Table 4-4: in a PDE that maps a 4-MiB page with neither the PAT nor
/// PSE-36 supported, bits 21:13 and bit 12 are reserved. The reserved-bit check
/// precedes any use of the entry.
#[test]
fn reserved_bits_in_a_large_pde_fault_at_the_directory_level() {
    let mut tables = PageTables::new();
    let linear = 0x0080_0000;
    tables.map_4mib(linear, 0x0080_0000, ENTRY_RW | (1 << 13));

    let ctx = ctx(&tables, CR4_PSE);
    let err = walk(&ctx, &mut tables.mem, linear).expect_err("reserved bit");
    assert_eq!(
        err.as_fault_reason().unwrap(),
        FaultReason::ReservedBit(PagingLevel::PageDirectory)
    );
}

/// SDM Table 4-5: "If CR4.PSE = 1, [PS] must be 0 ...; otherwise, ignored." A
/// PDE with PS = 1 therefore references a page table when CR4.PSE = 0.
#[test]
fn pde_ps_is_ignored_when_cr4_pse_is_clear() {
    let mut tables = PageTables::new();
    let linear = 0x0080_0000;
    let table = tables.ensure_page_table(linear, ENTRY_RW);
    let pde = tables.pde(linear);
    tables.set_pde(linear, pde | PDE_PS);
    tables.set_pte(linear, 0x0090_0000 | ENTRY_P | ENTRY_RW);
    assert_eq!(table, u64::from(tables.pde(linear) & 0xFFFF_F000));

    let ctx = ctx(&tables, 0);
    let result = walk(&ctx, &mut tables.mem, linear).expect("page-table reference");
    assert_eq!(result.page_size, PageSize::Size4KiB);
    assert_eq!(result.phys_addr, 0x0090_0000);
}

/// 4-MiB pages are a later slice. Until then the engine refuses instead of
/// translating a PS = 1 PDE as if it referenced a page table.
#[test]
fn large_page_is_reported_unsupported_for_now() {
    let mut tables = PageTables::new();
    let linear = 0x0080_1000;
    tables.map_4mib(linear, 0x0080_0000, ENTRY_RW);

    let ctx = ctx(&tables, CR4_PSE);
    let err = walk(&ctx, &mut tables.mem, linear).expect_err("4-MiB pages not implemented yet");
    assert_eq!(err, WalkError::Unsupported(UnsupportedPaging::LargePage));
    assert!(err.as_fault_reason().is_none());
}

/// SDM §4.1.1: with CR0.PG = 0 there is nothing to translate, and with
/// CR4.PAE = 1 the mode is PAE or IA-32e paging, neither of which this engine
/// implements.
#[test]
fn modes_outside_32bit_paging_are_reported_not_guessed() {
    let mut tables = PageTables::new();
    let linear = 0x0000_1000;
    tables.map_4kib(linear, 0x0010_0000, ENTRY_RW, ENTRY_RW);

    let disabled = PagingContext::new(0, tables.pd_base, 0);
    assert_eq!(
        walk(&disabled, &mut tables.mem, linear),
        Err(WalkError::Unsupported(UnsupportedPaging::PagingDisabled))
    );

    let pae = PagingContext::new(CR0_PG, tables.pd_base, CR4_PAE);
    assert_eq!(
        walk(&pae, &mut tables.mem, linear),
        Err(WalkError::Unsupported(UnsupportedPaging::PaeOrLongMode))
    );
}

/// A walk is a pure read of the paging structures: it never writes an accessed
/// or dirty flag, on success or on a fault. Accessed/dirty updates belong to
/// the translate path (SDM §4.8) and arrive in a later slice.
#[test]
fn walk_has_no_side_effects() {
    let mut tables = PageTables::new();
    let mapped = 0x0000_3000;
    let unmapped = 0x0100_0000;
    tables.map_4kib(mapped, 0x0011_0000, ENTRY_RW, ENTRY_RW);
    tables.mem.clear_log();

    let ctx = ctx(&tables, 0);
    walk(&ctx, &mut tables.mem, mapped).expect("translation");
    walk(&ctx, &mut tables.mem, unmapped).expect_err("no mapping");

    assert!(
        tables.mem.writes.is_empty(),
        "walk wrote {:?}",
        tables.mem.writes
    );
    assert_eq!(tables.pte(mapped) & ENTRY_A, 0);
    assert_eq!(tables.pde(mapped) & ENTRY_A, 0);
}

/// The walk reports every entry it used, including the U/S and R/W bits a
/// later slice combines into access rights (SDM §4.6).
#[test]
fn walk_reports_the_entries_it_used() {
    let mut tables = PageTables::new();
    let linear = 0x00C0_4000;
    tables.map_4kib(linear, 0x0022_0000, ENTRY_RW | ENTRY_US, ENTRY_US);

    let ctx = ctx(&tables, 0);
    let result = walk(&ctx, &mut tables.mem, linear).expect("translation");

    assert!(result.pde.read_write());
    assert!(result.pde.user_supervisor());
    let (_, pte) = result.pte.unwrap();
    assert!(!pte.read_write());
    assert!(pte.user_supervisor());
}
