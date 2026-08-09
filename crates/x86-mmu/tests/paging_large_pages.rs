//! 4-MiB pages (`CR4.PSE`), including the PSE-36 high address bits.
//!
//! Spec: Intel SDM Vol. 3 §4.1.3 (`CR4.PSE` enables 4-MiB pages for 32-bit
//! paging), §4.3 and Figure 4-3 (a PDE with `PS = 1` maps a 4-MiB page; the
//! final physical address takes bits 39:32 from PDE bits 20:13, bits 31:22 from
//! PDE bits 31:22, and bits 21:0 from the linear address), Table 4-4 (entry
//! format and reserved bits), §4.8 (a large PDE carries both the accessed and
//! the dirty flag), §4.10.2.1 (the page number is linear bits 31:22).

mod common;

use common::PageTables;
use x86_mmu::paging::entry::{ENTRY_A, ENTRY_D, ENTRY_RW, ENTRY_US, PDE_LARGE_PAT};
use x86_mmu::paging::{
    translate, walk, Access, AccessKind, AccessMode, FaultReason, PageSize, PagingContext,
    PagingLevel, PagingProfile, CR0_PG, CR0_WP, CR4_PSE,
};

const REGION: u32 = 0x0400_0000;
const FRAME: u32 = 0x0080_0000;

fn context(tables: &PageTables, cr0_extra: u64) -> PagingContext {
    PagingContext::new(CR0_PG | cr0_extra, tables.pd_base, CR4_PSE)
}

fn supervisor(kind: AccessKind) -> Access {
    Access::new(kind, AccessMode::Supervisor)
}

fn user(kind: AccessKind) -> Access {
    Access::new(kind, AccessMode::User)
}

/// SDM §4.3: with `CR4.PSE = 1` and `PS = 1` the PDE maps a 4-MiB page, the
/// frame comes from PDE bits 31:22, and the offset is linear bits 21:0 — so no
/// page table is consulted at all.
#[test]
fn large_pde_maps_a_4mib_page_without_a_page_table() {
    let mut tables = PageTables::new();
    tables.map_4mib(REGION, FRAME, ENTRY_RW);
    let ctx = context(&tables, 0);

    let result = walk(&ctx, &mut tables.mem, REGION + 0x0012_3456).expect("translation");
    assert_eq!(result.page_size, PageSize::Size4MiB);
    assert_eq!(result.frame_base, u64::from(FRAME));
    assert_eq!(result.phys_addr, u64::from(FRAME) + 0x0012_3456);
    assert!(result.pte.is_none());
    assert_eq!(result.final_entry_addr(), tables.pde_addr(REGION));

    // Only the PDE was read.
    assert_eq!(tables.mem.reads, vec![tables.pde_addr(REGION)]);
}

/// The offset spans the whole 4 MiB: the first and last byte of the region map
/// to the first and last byte of the frame.
#[test]
fn the_whole_4mib_offset_is_taken_from_the_linear_address() {
    let mut tables = PageTables::new();
    tables.map_4mib(REGION, FRAME, ENTRY_RW);
    let ctx = context(&tables, 0);

    let first = walk(&ctx, &mut tables.mem, REGION).expect("first byte");
    let last = walk(&ctx, &mut tables.mem, REGION + 0x003F_FFFF).expect("last byte");
    assert_eq!(first.phys_addr, u64::from(FRAME));
    assert_eq!(last.phys_addr, u64::from(FRAME) + 0x003F_FFFF);

    // The next linear address belongs to the following PDE, which is absent.
    let err = walk(&ctx, &mut tables.mem, REGION + 0x0040_0000).expect_err("next region");
    assert_eq!(
        err.as_fault_reason().unwrap(),
        FaultReason::NotPresent(PagingLevel::PageDirectory)
    );
}

/// With no page table in the translation, the PDE alone supplies the U/S and
/// R/W rights (SDM §4.6.1: the logical-AND is over "every paging-structure
/// entry controlling the translation", which here is one entry).
#[test]
fn a_large_page_takes_its_rights_from_the_pde_alone() {
    let mut tables = PageTables::new();
    tables.map_4mib(REGION, FRAME, ENTRY_US | ENTRY_RW);
    let ctx = context(&tables, CR0_WP);

    let translation =
        translate(&ctx, &mut tables.mem, REGION, user(AccessKind::Write)).expect("user write");
    assert!(translation.writable);
    assert!(translation.user_accessible);

    let mut tables = PageTables::new();
    tables.map_4mib(REGION, FRAME, ENTRY_US);
    let ctx = context(&tables, CR0_WP);
    let fault = translate(&ctx, &mut tables.mem, REGION, user(AccessKind::Write))
        .expect_err("read-only")
        .as_fault()
        .unwrap();
    assert_eq!(fault.reason, FaultReason::Protection);
}

/// SDM §4.8: for an entry that maps a page, both the accessed and the dirty
/// flag live in that entry — here, in the PDE.
#[test]
fn a_large_pde_carries_both_accessed_and_dirty() {
    let mut tables = PageTables::new();
    tables.map_4mib(REGION, FRAME, ENTRY_RW);
    let ctx = context(&tables, 0);

    translate(&ctx, &mut tables.mem, REGION, supervisor(AccessKind::Read)).expect("read");
    assert_ne!(tables.pde(REGION) & ENTRY_A, 0);
    assert_eq!(tables.pde(REGION) & ENTRY_D, 0);

    let translation =
        translate(&ctx, &mut tables.mem, REGION, supervisor(AccessKind::Write)).expect("write");
    assert_ne!(tables.pde(REGION) & ENTRY_D, 0);
    assert!(translation.dirty);
}

/// A denied write to a large page must not set the dirty flag in the PDE
/// either.
#[test]
fn a_denied_write_to_a_large_page_sets_no_flags() {
    let mut tables = PageTables::new();
    tables.map_4mib(REGION, FRAME, 0);
    tables.mem.clear_log();
    let ctx = context(&tables, CR0_WP);

    translate(&ctx, &mut tables.mem, REGION, supervisor(AccessKind::Write))
        .expect_err("CR0.WP denies a write to a read-only page");
    assert!(
        tables.mem.writes.is_empty(),
        "wrote {:?}",
        tables.mem.writes
    );
    assert_eq!(tables.pde(REGION) & (ENTRY_A | ENTRY_D), 0);
}

/// SDM Table 4-4: with neither the PAT nor PSE-36 supported, bit 12 and bits
/// 21:13 of a 4-MiB PDE are reserved, and the check runs before the entry is
/// used.
#[test]
fn reserved_bits_in_a_large_pde() {
    for bad_bit in [PDE_LARGE_PAT, 1 << 13, 1 << 21] {
        let mut tables = PageTables::new();
        tables.map_4mib(REGION, FRAME, ENTRY_RW | bad_bit);
        let ctx = context(&tables, 0);
        let err = walk(&ctx, &mut tables.mem, REGION).expect_err("reserved bit {bad_bit:#x}");
        assert_eq!(
            err.as_fault_reason().unwrap(),
            FaultReason::ReservedBit(PagingLevel::PageDirectory),
            "bit {bad_bit:#x}"
        );
    }
}

/// SDM §4.3 / Table 4-4: when the PSE-36 mechanism is supported, PDE bits 20:13
/// supply physical-address bits 39:32, so a 4-MiB page can sit above 4 GiB.
#[test]
fn pse36_supplies_physical_address_bits_39_32() {
    let mut tables = PageTables::new();
    // Physical bits 39:32 = 0x03, bits 31:22 = 0x008.
    tables.map_4mib(REGION, FRAME, ENTRY_RW | (0x03 << 13));
    let ctx = PagingContext::with_profile(
        CR0_PG,
        tables.pd_base,
        CR4_PSE,
        PagingProfile::with_pse36(40),
    );

    let result = walk(&ctx, &mut tables.mem, REGION + 0x10).expect("translation");
    assert_eq!(result.frame_base, 0x0000_0003_0080_0000);
    assert_eq!(result.phys_addr, 0x0000_0003_0080_0010);
}

/// Without PSE-36 those same bits are reserved, so the entry that translated
/// above faults instead. Two profiles, same paging structures, different
/// architectural answer.
#[test]
fn without_pse36_the_high_address_bits_are_reserved() {
    let mut tables = PageTables::new();
    tables.map_4mib(REGION, FRAME, ENTRY_RW | (0x03 << 13));
    let ctx = context(&tables, 0);

    let err = walk(&ctx, &mut tables.mem, REGION).expect_err("bits 21:13 reserved");
    assert_eq!(
        err.as_fault_reason().unwrap(),
        FaultReason::ReservedBit(PagingLevel::PageDirectory)
    );
}

/// SDM Table 4-5: `PS` is ignored when `CR4.PSE = 0`, so clearing `CR4.PSE`
/// turns the very same PDE into a page-table reference — and the page table it
/// then points at is whatever the frame bits happen to address.
#[test]
fn clearing_cr4_pse_turns_the_same_pde_into_a_page_table_reference() {
    let mut tables = PageTables::new();
    tables.map_4mib(REGION, FRAME, ENTRY_RW);

    let with_pse = context(&tables, 0);
    assert_eq!(
        walk(&with_pse, &mut tables.mem, REGION).unwrap().page_size,
        PageSize::Size4MiB
    );

    let without_pse = PagingContext::new(CR0_PG, tables.pd_base, 0);
    let result = walk(&without_pse, &mut tables.mem, REGION).expect_err("empty page table");
    assert_eq!(
        result.as_fault_reason().unwrap(),
        FaultReason::NotPresent(PagingLevel::PageTable)
    );
}

/// A 4-MiB PDE with `P = 0` yields no translation, exactly as at any other
/// level, and reserved bits are not checked in it (§4.7).
#[test]
fn absent_large_pde_reports_not_present() {
    let mut tables = PageTables::new();
    // PS and a reserved address bit set, but P clear.
    tables.set_pde(REGION, (1 << 7) | (1 << 13));
    let ctx = context(&tables, 0);

    let err = walk(&ctx, &mut tables.mem, REGION).expect_err("absent");
    assert_eq!(
        err.as_fault_reason().unwrap(),
        FaultReason::NotPresent(PagingLevel::PageDirectory)
    );
}
