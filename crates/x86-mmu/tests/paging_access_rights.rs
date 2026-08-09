//! Access rights and `#PF` error-code composition.
//!
//! Spec: Intel SDM Vol. 3 §4.6.1 "Determination of Access Rights" (the U/S and
//! R/W combining rule, and `CR0.WP`), §4.7 "Page-Fault Exceptions" with
//! Figure 4-12 (error-code bits P, W/R, U/S, RSVD, I/D).

mod common;

use common::PageTables;
use x86_mmu::paging::entry::{ENTRY_RW, ENTRY_US, PTE_PAT};
use x86_mmu::paging::{
    translate, Access, AccessKind, AccessMode, FaultReason, PageFault, PagingContext, PagingLevel,
    TranslateError, CR0_PG, CR0_WP, CR4_PSE, PF_ERR_ID, PF_ERR_P, PF_ERR_RSVD, PF_ERR_US,
    PF_ERR_WR,
};

const LINEAR: u32 = 0x0100_5000;
const FRAME: u32 = 0x0033_0000;

fn ctx(tables: &PageTables, cr0_extra: u64, cr4: u64) -> PagingContext {
    PagingContext::new(CR0_PG | cr0_extra, tables.pd_base, cr4)
}

fn supervisor(kind: AccessKind) -> Access {
    Access::new(kind, AccessMode::Supervisor)
}

fn user(kind: AccessKind) -> Access {
    Access::new(kind, AccessMode::User)
}

/// Try one access against a page mapped with the given PDE and PTE flags.
fn attempt(
    pde_flags: u32,
    pte_flags: u32,
    cr0_extra: u64,
    access: Access,
) -> Result<u64, PageFault> {
    let mut tables = PageTables::new();
    tables.map_4kib(LINEAR, FRAME, pde_flags, pte_flags);
    let ctx = ctx(&tables, cr0_extra, 0);
    match translate(&ctx, &mut tables.mem, LINEAR, access) {
        Ok(translation) => Ok(translation.phys_addr),
        Err(TranslateError::Fault(fault)) => Err(fault),
        Err(other) => panic!("unexpected {other:?}"),
    }
}

/// SDM §4.6.1: the R/W and U/S flags combine as a logical-AND over every entry
/// controlling the translation, so a permissive PTE cannot widen a restrictive
/// PDE and vice versa.
#[test]
fn user_and_write_permission_combine_as_logical_and() {
    let mut tables = PageTables::new();
    // Case matrix over (PDE flags, PTE flags) -> (user-accessible, writable).
    let cases = [
        (0, 0, false, false),
        (ENTRY_RW, 0, false, false),
        (0, ENTRY_RW, false, false),
        (ENTRY_RW, ENTRY_RW, false, true),
        (ENTRY_US, ENTRY_US, true, false),
        (ENTRY_US, 0, false, false),
        (0, ENTRY_US, false, false),
        (ENTRY_US | ENTRY_RW, ENTRY_US | ENTRY_RW, true, true),
        (ENTRY_US | ENTRY_RW, ENTRY_US, true, false),
        (ENTRY_US, ENTRY_US | ENTRY_RW, true, false),
    ];

    let ctx = ctx(&tables, 0, 0);
    for (index, (pde_flags, pte_flags, want_user, want_write)) in cases.into_iter().enumerate() {
        let linear = 0x0040_0000 * (index as u32 + 1);
        tables.map_4kib(linear, FRAME, pde_flags, pte_flags);
        let translation = translate(&ctx, &mut tables.mem, linear, supervisor(AccessKind::Read))
            .expect("supervisor read is always permitted");
        assert_eq!(
            translation.user_accessible, want_user,
            "case {index}: U/S combination"
        );
        assert_eq!(
            translation.writable, want_write,
            "case {index}: R/W combination"
        );
    }
}

/// SDM §4.6.1: "Data may be read (implicitly or explicitly) from any
/// supervisor-mode address", and with SMAP unmodeled a supervisor read of a
/// user page is permitted too. Instruction fetches likewise, because there is
/// no SMEP and no execute-disable with 32-bit paging.
#[test]
fn supervisor_reads_and_fetches_are_always_permitted() {
    for flags in [0, ENTRY_US, ENTRY_RW, ENTRY_US | ENTRY_RW] {
        for kind in [AccessKind::Read, AccessKind::InstructionFetch] {
            assert!(
                attempt(flags, flags, 0, supervisor(kind)).is_ok(),
                "flags {flags:#x} kind {kind:?}"
            );
            assert!(
                attempt(flags, flags, CR0_WP, supervisor(kind)).is_ok(),
                "flags {flags:#x} kind {kind:?} with CR0.WP"
            );
        }
    }
}

/// SDM §4.1.3: "If CR0.WP = 0, supervisor-mode write accesses are allowed to
/// linear addresses with read-only access rights; if CR0.WP = 1, they are not."
/// This holds for supervisor-mode addresses and for user-mode addresses alike
/// (§4.6.1).
#[test]
fn cr0_wp_governs_supervisor_writes_to_read_only_pages() {
    let write = supervisor(AccessKind::Write);

    // Read-only supervisor page.
    assert!(attempt(0, 0, 0, write).is_ok());
    let fault = attempt(0, 0, CR0_WP, write).expect_err("WP=1 denies the write");
    assert_eq!(fault.reason, FaultReason::Protection);

    // Read-only *user* page — the case CR0.WP exists for.
    assert!(attempt(ENTRY_US, ENTRY_US, 0, write).is_ok());
    let fault = attempt(ENTRY_US, ENTRY_US, CR0_WP, write).expect_err("WP=1 denies the write");
    assert_eq!(fault.reason, FaultReason::Protection);

    // Writable pages are unaffected by CR0.WP.
    assert!(attempt(ENTRY_RW, ENTRY_RW, CR0_WP, write).is_ok());
    assert!(attempt(ENTRY_US | ENTRY_RW, ENTRY_US | ENTRY_RW, CR0_WP, write).is_ok());
}

/// SDM §4.6.1, user-mode accesses: "Data may not be read from any
/// supervisor-mode address", and a write additionally needs R/W = 1 in every
/// entry — regardless of `CR0.WP`, which only ever relaxes supervisor writes.
#[test]
fn user_accesses_need_a_user_page_and_writes_need_read_write() {
    for kind in [AccessKind::Read, AccessKind::InstructionFetch] {
        assert!(attempt(0, 0, 0, user(kind)).is_err(), "{kind:?}");
        assert!(attempt(ENTRY_US, 0, 0, user(kind)).is_err(), "{kind:?}");
        assert!(attempt(0, ENTRY_US, 0, user(kind)).is_err(), "{kind:?}");
        assert!(
            attempt(ENTRY_US, ENTRY_US, 0, user(kind)).is_ok(),
            "{kind:?}"
        );
    }

    let write = user(AccessKind::Write);
    assert!(attempt(ENTRY_US, ENTRY_US, 0, write).is_err());
    assert!(attempt(ENTRY_US, ENTRY_US, CR0_WP, write).is_err());
    assert!(attempt(ENTRY_US | ENTRY_RW, ENTRY_US, 0, write).is_err());
    assert!(attempt(ENTRY_US, ENTRY_US | ENTRY_RW, 0, write).is_err());
    assert!(attempt(ENTRY_US | ENTRY_RW, ENTRY_US | ENTRY_RW, 0, write).is_ok());
    assert!(
        attempt(ENTRY_RW, ENTRY_RW, 0, write).is_err(),
        "supervisor page"
    );
}

/// SDM §4.6.1: CPL 3 is a user-mode access, CPL 0-2 supervisor-mode.
#[test]
fn access_mode_comes_from_cpl() {
    assert_eq!(Access::from_cpl(AccessKind::Read, 3).mode, AccessMode::User);
    for cpl in 0..=2 {
        assert_eq!(
            Access::from_cpl(AccessKind::Read, cpl).mode,
            AccessMode::Supervisor
        );
    }
}

/// SDM §4.7: P is 0 only for a not-present fault; W/R and U/S describe the
/// access, not the access rights; RSVD marks a reserved-bit violation and can
/// only be set together with P.
#[test]
fn error_code_bits_follow_the_sdm_definitions() {
    // Not present, supervisor read: every bit clear.
    let mut tables = PageTables::new();
    let ctx = ctx(&tables, 0, 0);
    let err = translate(&ctx, &mut tables.mem, LINEAR, supervisor(AccessKind::Read))
        .expect_err("nothing mapped");
    let fault = err.as_fault().unwrap();
    assert_eq!(fault.error_code(), 0);
    assert_eq!(fault.cr2(), u64::from(LINEAR));
    assert_eq!(
        fault.reason,
        FaultReason::NotPresent(PagingLevel::PageDirectory)
    );

    // Not present, user write: W/R and U/S set, P clear.
    let err = translate(&ctx, &mut tables.mem, LINEAR, user(AccessKind::Write))
        .expect_err("nothing mapped");
    assert_eq!(err.as_fault().unwrap().error_code(), PF_ERR_WR | PF_ERR_US);

    // Protection violation, user write to a read-only user page: P, W/R, U/S.
    let fault = attempt(ENTRY_US, ENTRY_US, 0, user(AccessKind::Write)).expect_err("read-only");
    assert_eq!(fault.error_code(), PF_ERR_P | PF_ERR_WR | PF_ERR_US);

    // Protection violation, user read of a supervisor page: P and U/S only.
    let fault =
        attempt(ENTRY_RW, ENTRY_RW, 0, user(AccessKind::Read)).expect_err("supervisor page");
    assert_eq!(fault.error_code(), PF_ERR_P | PF_ERR_US);

    // Protection violation, supervisor write with CR0.WP: P and W/R only.
    let fault = attempt(0, 0, CR0_WP, supervisor(AccessKind::Write)).expect_err("read-only");
    assert_eq!(fault.error_code(), PF_ERR_P | PF_ERR_WR);
}

/// SDM §4.7: "Because reserved bits are not checked in a paging-structure entry
/// whose P flag is 0, bit 3 of the error code can be set only if bit 0 is also
/// set."
#[test]
fn reserved_bit_fault_sets_both_p_and_rsvd() {
    let mut tables = PageTables::new();
    tables.map_4kib(LINEAR, FRAME, ENTRY_RW, ENTRY_RW | PTE_PAT);
    let ctx = ctx(&tables, 0, CR4_PSE);

    let err = translate(&ctx, &mut tables.mem, LINEAR, supervisor(AccessKind::Read))
        .expect_err("reserved bit");
    let fault = err.as_fault().unwrap();
    assert_eq!(
        fault.reason,
        FaultReason::ReservedBit(PagingLevel::PageTable)
    );
    assert_eq!(fault.error_code(), PF_ERR_P | PF_ERR_RSVD);
    assert_eq!(fault.error_code() & PF_ERR_P, PF_ERR_P);
}

/// SDM §4.7: I/D (bit 4) is set only if the access was an instruction fetch and
/// either `CR4.SMEP = 1` or (`CR4.PAE = 1` and `IA32_EFER.NXE = 1`). None of
/// those can hold with 32-bit paging in this engine, so a faulting fetch never
/// sets it.
#[test]
fn instruction_fetch_never_sets_the_id_bit() {
    let fault = attempt(ENTRY_RW, ENTRY_RW, 0, user(AccessKind::InstructionFetch))
        .expect_err("supervisor page");
    assert_eq!(fault.error_code() & PF_ERR_ID, 0);
    assert_eq!(fault.error_code(), PF_ERR_P | PF_ERR_US);

    // A fetch is not a write, so W/R stays clear too.
    assert_eq!(fault.error_code() & PF_ERR_WR, 0);
}

/// The faulting linear address is reported verbatim, offset included, because
/// `CR2` receives the address the access used — not the page base.
#[test]
fn cr2_carries_the_faulting_linear_address_including_the_offset() {
    let mut tables = PageTables::new();
    let ctx = ctx(&tables, 0, 0);
    for linear in [0x0000_0000, 0x0000_0FFF, 0xDEAD_BEEF, 0xFFFF_FFFF] {
        let err = translate(&ctx, &mut tables.mem, linear, supervisor(AccessKind::Read))
            .expect_err("nothing mapped");
        let fault = err.as_fault().unwrap();
        assert_eq!(fault.linear_address, linear);
        assert_eq!(fault.cr2(), u64::from(linear));
    }
}

/// A permitted translation reports the combined rights the TLB will cache
/// (SDM §4.10.2.2) and the address of the entry that maps the page.
#[test]
fn successful_translation_reports_combined_rights() {
    let mut tables = PageTables::new();
    tables.map_4kib(LINEAR, FRAME, ENTRY_US | ENTRY_RW, ENTRY_US | ENTRY_RW);
    let ctx = ctx(&tables, CR0_WP, 0);

    let translation = translate(&ctx, &mut tables.mem, LINEAR, user(AccessKind::Write))
        .expect("user write to a writable user page");
    assert_eq!(translation.phys_addr, u64::from(FRAME));
    assert!(translation.writable);
    assert!(translation.user_accessible);
    assert!(!translation.global, "CR4.PGE is clear");
    assert_eq!(translation.final_entry_addr, tables.pte_addr(LINEAR));
}
