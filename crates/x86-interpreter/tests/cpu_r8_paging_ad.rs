//! Round-8 slice 4: paging accessed/dirty honesty + INVLPG under PE=1/PG=1.
//!
//! Spec: Intel SDM Vol. 3 §4.8 (A/D), §4.10.2.3 (cache after A set),
//! §4.10.4.1 (`INVLPG`); Vol. 2 "INVLPG".

mod common;

use common::*;
use x86_core::CpuState;
use x86_interpreter::step_with_mmu;
use x86_mmu::paging::{PageSize, TlbEntry};

/// Successful walks already set A/D; a faulting walk writes nothing (including
/// higher-level A). This pins the Round-4 honesty choice end-to-end.
/// Spec: SDM Vol. 3 §4.8 vs §4.10.2.3.
#[test]
fn successful_walk_sets_ad_fault_writes_nothing() {
    // A0 00 90 00 00           MOV AL, [0x9000]
    // C6 05 00 90 00 00 5A     MOV byte [0x9000], 0x5A
    // C6 05 00 00 40 00 AA     MOV byte [0x00400000], 0xAA  → #PF
    let (mut cpu, mut bus, mut mmu) = paged_fixture(&[
        0xA0, 0x00, 0x90, 0x00, 0x00, 0xC6, 0x05, 0x00, 0x90, 0x00, 0x00, 0x5A, 0xC6, 0x05, 0x00,
        0x00, 0x40, 0x00, 0xAA, 0xF4,
    ]);
    map_high_page(&mut bus, 0);
    let pde_high_before = bus.peek_u32(PD_BASE + 4);

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();
    assert_eq!(bus.peek_u32(pte_addr(DATA)) & (A | D), A);

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();
    assert_eq!(bus.peek_u32(pte_addr(DATA)) & (A | D), A | D);
    assert_eq!(bus.peek_u32(PD_BASE) & D, 0, "PDE→PT bit6 ignored");

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();
    assert_eq!(cpu.rip, u64::from(HANDLER));
    assert_eq!(
        bus.peek_u32(PD_BASE + 4),
        pde_high_before,
        "fault leaves PDE.A clear"
    );
}

/// Under PE=1 and PG=1, `INVLPG` drops the cached translation so a remapped
/// PTE becomes visible. Spec: SDM Vol. 2 "INVLPG"; Vol. 3 §4.10.4.1.
#[test]
fn invlpg_pe1_pg1_makes_remapped_pte_visible() {
    // A0 00 90 00 00   MOV AL, [0x9000]          — warm TLB on DATA
    // A0 00 90 00 00   MOV AL, [0x9000]          — stale TLB after remap
    // 0F 01 38         INVLPG [EAX]              — EAX=0x9000
    // A0 00 90 00 00   MOV AL, [0x9000]          — fresh walk
    // F4
    let code = [
        0xA0, 0x00, 0x90, 0x00, 0x00, // warm
        0xA0, 0x00, 0x90, 0x00, 0x00, // stale
        0x0F, 0x01, 0x38, // INVLPG [EAX]
        0xA0, 0x00, 0x90, 0x00, 0x00, // fresh
        0xF4,
    ];
    let (mut cpu, mut bus, mut mmu) = paged_fixture(&code);
    const ALT_FRAME: u32 = 0xB000;
    bus.mem[DATA as usize] = 0x11;
    bus.mem[ALT_FRAME as usize] = 0x22;

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();
    assert_eq!(cpu.al(), 0x11, "first read from original frame");

    // Remap linear DATA → ALT_FRAME without touching CR3 (no global flush).
    bus.poke_u32(pte_addr(DATA), ALT_FRAME | P | RW | US);

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();
    assert_eq!(
        cpu.al(),
        0x11,
        "stale TLB still hits the original frame before INVLPG"
    );

    cpu.set_gpr_u32(CpuState::RAX, DATA);
    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();
    assert!(
        mmu.tlb().lookup(DATA).is_none(),
        "INVLPG dropped the DATA page"
    );

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();
    assert_eq!(cpu.al(), 0x22, "post-INVLPG walk sees the remapped frame");
}

/// `INVLPG` at CPL≠0 under PE=1 raises `#GP(0)` before invalidating.
/// Spec: SDM Vol. 2 "INVLPG" Protected Mode Exceptions; Vol. 3 §5.5.
#[test]
fn invlpg_pe1_outside_ring0_raises_gp() {
    let (mut cpu, mut bus, mut mmu) = paged_fixture(&[0x0F, 0x01, 0x38, 0xF4]);
    cpu.set_gpr_u32(CpuState::RAX, DATA);
    mmu.tlb_mut().insert(
        DATA,
        TlbEntry {
            frame_base: u64::from(DATA),
            page_size: PageSize::Size4KiB,
            writable: true,
            user_accessible: true,
            global: false,
            dirty: false,
            final_entry_addr: pte_addr(DATA) as u64,
        },
    );
    to_ring3(&mut cpu);
    // Ring-3 #GP gate must target the ring-3 code selector (same-CPL delivery).
    bus.write_bytes(0x4000 + 13 * 8 + 2, &[0x1B, 0x00]);

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();
    assert_eq!(cpu.rip, u64::from(HANDLER));
    assert!(
        mmu.tlb().lookup(DATA).is_some(),
        "failed INVLPG must not invalidate"
    );
}
