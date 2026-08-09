//! Round-4 slice 2: guest data accesses translated through the 32-bit paging
//! engine, and `#PF` delivered through the 386 interrupt gate.
//!
//! Every test here builds real paging structures in guest memory and executes
//! real instructions against them, so what is under test is the interpreter's
//! memory path rather than the engine's helpers.
//!
//! Spec: Intel SDM Vol. 3 §4.1.1 (paging-mode selection), §4.3 (32-bit
//! paging), §4.6.1 (access rights), §4.7 (`#PF`, `CR2`, the error code), §4.8
//! (accessed/dirty), §4.10.4.1 (a page fault invalidates the cached
//! translation); Vol. 2 "IRET/IRETD".

mod common;

use common::*;
use x86_core::CpuState;
use x86_interpreter::{run_with_mmu, step_with_mmu};

/// A data read at a linear address outside the identity-mapped region returns
/// the byte at the *physical* address the page tables produce.
/// Spec: SDM Vol. 3 §4.3 (Figure 4-2).
#[test]
fn data_read_uses_the_physical_address_the_walk_produces() {
    // A0 00 00 40 00   MOV AL, [0x00400000]
    let (mut cpu, mut bus, mut mmu) = paged_fixture(&[0xA0, 0x00, 0x00, 0x40, 0x00, 0xF4]);
    map_high_page(&mut bus, DATA | P | RW | US);
    bus.mem[DATA as usize] = 0x5A;

    // The bus is only 0x2_0000 bytes, so an untranslated access to 0x0040_0000
    // would be a bus fault rather than a wrong answer.
    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();
    assert_eq!(cpu.al(), 0x5A);
}

/// A write to a not-present page raises `#PF`, loads `CR2` with the faulting
/// linear address, pushes error code `W` (bit 1) with `P` clear, and saves the
/// faulting instruction's `EIP` — `#PF` is a fault, not a trap.
/// Spec: SDM Vol. 3 §4.7 Figure 4-12; §6.5 (fault semantics).
#[test]
fn not_present_write_delivers_pf_with_cr2_error_code_and_faulting_eip() {
    // C6 05 00 00 40 00 AA   MOV byte [0x00400000], 0xAA
    let (mut cpu, mut bus, mut mmu) =
        paged_fixture(&[0xC6, 0x05, 0x00, 0x00, 0x40, 0x00, 0xAA, 0xF4]);
    map_high_page(&mut bus, 0);

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();

    assert_eq!(cpu.rip, u64::from(HANDLER), "#PF handler entered");
    assert_eq!(cpu.cr2, u64::from(HIGH), "CR2 holds the faulting address");
    assert_eq!(bus.peek_u32(FRAME_ERROR_CODE), 0x2, "P=0, W/R=1, U/S=0");
    assert_eq!(bus.peek_u32(FRAME_EIP), CODE, "fault, so EIP is restarted");
}

/// A faulting instruction fetch reports the fetched linear address in `CR2`
/// and an error code whose I/D bit stays **clear**: §4.7 sets bit 4 only with
/// `CR4.SMEP = 1` or PAE + `IA32_EFER.NXE = 1`, and neither exists here.
#[test]
fn faulting_instruction_fetch_leaves_the_id_bit_clear() {
    // E9 FB AF 3F 00   JMP 0x00400000
    let displacement = (HIGH - (CODE + 5)) as i32;
    let mut code = vec![0xE9];
    code.extend_from_slice(&displacement.to_le_bytes());
    let (mut cpu, mut bus, mut mmu) = paged_fixture(&code);
    map_high_page(&mut bus, 0);

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();
    assert_eq!(cpu.rip, u64::from(HIGH), "the jump itself succeeds");

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();
    assert_eq!(cpu.rip, u64::from(HANDLER));
    assert_eq!(cpu.cr2, u64::from(HIGH));
    let error_code = bus.peek_u32(FRAME_ERROR_CODE);
    assert_eq!(error_code, 0x0, "not present, not a write, supervisor");
    assert_eq!(error_code & (1 << 4), 0, "I/D is clear without SMEP or NX");
    assert_eq!(
        bus.peek_u32(FRAME_EIP),
        HIGH,
        "the un-fetchable EIP is saved"
    );
}

/// `CR0.WP` decides whether a supervisor write to a read-only page is
/// permitted. With `WP = 0` it succeeds; with `WP = 1` it takes `#PF` with
/// `P` and `W/R` set and `U/S` clear. Spec: SDM Vol. 3 §4.1.3, §4.6.1.
#[test]
fn supervisor_write_to_a_read_only_page_follows_cr0_wp() {
    for write_protect in [false, true] {
        // C6 05 00 90 00 00 AA   MOV byte [0x9000], 0xAA
        let (mut cpu, mut bus, mut mmu) =
            paged_fixture(&[0xC6, 0x05, 0x00, 0x90, 0x00, 0x00, 0xAA, 0xF4]);
        // Drop R/W on the page that maps linear 0x9000.
        bus.poke_u32(pte_addr(DATA), DATA | P | US);
        if write_protect {
            cpu.cr0 |= CR0_WP;
        }

        step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();

        if write_protect {
            assert_eq!(cpu.rip, u64::from(HANDLER), "WP=1 denies the write");
            assert_eq!(cpu.cr2, u64::from(DATA));
            assert_eq!(bus.peek_u32(FRAME_ERROR_CODE), 0x3, "P=1, W/R=1, U/S=0");
            assert_eq!(bus.peek_u8(DATA as usize), 0, "nothing was written");
        } else {
            assert_eq!(bus.peek_u8(DATA as usize), 0xAA, "WP=0 permits the write");
        }
    }
}

/// A user-mode write to a read-only user page faults with `P`, `W/R` and `U/S`
/// all set, regardless of `CR0.WP`. Spec: SDM Vol. 3 §4.6.1, §4.7.
#[test]
fn user_write_to_a_read_only_user_page_sets_the_us_bit() {
    // C6 05 00 90 00 00 AA   MOV byte [0x9000], 0xAA
    let (mut cpu, mut bus, mut mmu) =
        paged_fixture(&[0xC6, 0x05, 0x00, 0x90, 0x00, 0x00, 0xAA, 0xF4]);
    bus.poke_u32(pte_addr(DATA), DATA | P | US);
    // Same-CPL delivery only, so the gate has to target the ring-3 code
    // selector; a ring-0 handler would need the stack switch this interpreter
    // does not implement yet.
    bus.write_bytes(0x4000 + 14 * 8 + 2, &[0x1B, 0x00]);
    to_ring3(&mut cpu);

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();

    assert_eq!(cpu.rip, u64::from(HANDLER));
    assert_eq!(bus.peek_u32(FRAME_ERROR_CODE), 0x7, "P=1, W/R=1, U/S=1");
    assert_eq!(bus.peek_u8(DATA as usize), 0);
}

/// A user-mode access to a supervisor-mode page faults even for a read.
/// Spec: SDM Vol. 3 §4.6.1 ("user-mode accesses are not permitted to
/// supervisor-mode addresses").
#[test]
fn user_read_of_a_supervisor_page_faults() {
    // A0 00 90 00 00   MOV AL, [0x9000]
    let (mut cpu, mut bus, mut mmu) = paged_fixture(&[0xA0, 0x00, 0x90, 0x00, 0x00, 0xF4]);
    bus.poke_u32(pte_addr(DATA), DATA | P | RW);
    bus.write_bytes(0x4000 + 14 * 8 + 2, &[0x1B, 0x00]);
    to_ring3(&mut cpu);

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();
    assert_eq!(cpu.rip, u64::from(HANDLER));
    assert_eq!(bus.peek_u32(FRAME_ERROR_CODE), 0x5, "P=1, W/R=0, U/S=1");
}

/// A read sets `A` in every entry used; a write also sets `D` in the entry
/// that maps the page. Bit 6 of a PDE that references a page table is ignored
/// and never written. Spec: SDM Vol. 3 §4.8, Table 4-5.
#[test]
fn accessed_and_dirty_flags_follow_the_access() {
    // A0 00 90 00 00           MOV AL, [0x9000]
    // C6 05 00 90 00 00 AA     MOV byte [0x9000], 0xAA
    let (mut cpu, mut bus, mut mmu) = paged_fixture(&[
        0xA0, 0x00, 0x90, 0x00, 0x00, 0xC6, 0x05, 0x00, 0x90, 0x00, 0x00, 0xAA, 0xF4,
    ]);

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();
    assert_eq!(
        bus.peek_u32(pte_addr(DATA)) & (A | D),
        A,
        "read sets A only"
    );
    assert_eq!(bus.peek_u32(PD_BASE) & A, A, "the PDE used is accessed");

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();
    assert_eq!(
        bus.peek_u32(pte_addr(DATA)) & (A | D),
        A | D,
        "write sets D"
    );
    assert_eq!(
        bus.peek_u32(PD_BASE) & D,
        0,
        "PDE bit 6 is ignored, not set"
    );
}

/// A faulting access leaves the paging structures byte-for-byte unchanged,
/// including the accessed flag of the higher-level entry the walk did read.
///
/// This is the engine's one documented model choice: §4.8 read literally would
/// set that flag, and the engine follows §4.10.2.3 instead, tying accessed-flag
/// updates to a translation that completes. The test pins the choice against
/// real instruction execution rather than against the engine's own unit tests.
#[test]
fn a_faulting_access_writes_no_paging_structure_byte() {
    // C6 05 00 00 40 00 AA   MOV byte [0x00400000], 0xAA
    let (mut cpu, mut bus, mut mmu) =
        paged_fixture(&[0xC6, 0x05, 0x00, 0x00, 0x40, 0x00, 0xAA, 0xF4]);
    map_high_page(&mut bus, 0);
    let pde_before = bus.peek_u32(PD_BASE + 4);
    let pte_before = bus.peek_u32(PT2);

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();

    assert_eq!(cpu.rip, u64::from(HANDLER), "the access did fault");
    assert_eq!(bus.peek_u32(PD_BASE + 4), pde_before, "PDE.A left clear");
    assert_eq!(
        bus.peek_u32(PT2),
        pte_before,
        "the not-present PTE is intact"
    );
}

/// `CR4.PSE` plus `PS = 1` maps a 4-MiB page with no page table at all: linear
/// bits 21:0 are the offset into the frame. Spec: SDM Vol. 3 §4.3 Figure 4-3,
/// Table 4-4.
#[test]
fn four_mib_page_translates_without_a_page_table() {
    // A0 00 90 40 00   MOV AL, [0x00409000]
    let (mut cpu, mut bus, mut mmu) = paged_fixture(&[0xA0, 0x00, 0x90, 0x40, 0x00, 0xF4]);
    // PDE[1] maps linear 0x0040_0000-0x007F_FFFF onto physical 0-0x3F_FFFF.
    bus.poke_u32(PD_BASE + 4, PS | P | RW | US);
    cpu.cr4 |= CR4_PSE;
    bus.mem[DATA as usize] = 0x77;

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();
    assert_eq!(cpu.al(), 0x77);
    // §4.8: the PDE that maps the page is the entry that carries A and D.
    assert_eq!(bus.peek_u32(PD_BASE + 4) & A, A);
}

/// Without `CR4.PSE` the `PS` bit is ignored and the PDE references a page
/// table (SDM Vol. 3 Table 4-5), so the same directory entry means something
/// completely different.
#[test]
fn ps_is_ignored_when_cr4_pse_is_clear() {
    // A0 00 90 40 00   MOV AL, [0x00409000]
    let (mut cpu, mut bus, mut mmu) = paged_fixture(&[0xA0, 0x00, 0x90, 0x40, 0x00, 0xF4]);
    // With PSE this maps a 4-MiB page at physical 0; without it, bits 31:12
    // name a page table at physical 0, whose entry 9 is a zeroed IVT slot.
    bus.poke_u32(PD_BASE + 4, PS | P | RW | US);
    bus.mem[DATA as usize] = 0x77;

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();
    assert_eq!(cpu.rip, u64::from(HANDLER), "not-present PTE at physical 0");
    assert_eq!(cpu.cr2, 0x0040_9000);
}

/// With `CR0.PG = 0` a linear address is the physical address, even when
/// `CR3` points at page tables that say otherwise. This is the branch every
/// pre-round-4 test still runs on.
/// Spec: SDM Vol. 3 §4.1.1.
#[test]
fn paging_disabled_keeps_linear_equal_to_physical() {
    // C6 05 00 00 40 00 AA   MOV byte [0x00400000], 0xAA  → bus fault, not a walk
    // A0 00 90 00 00         MOV AL, [0x9000]
    let (mut cpu, mut bus, mut mmu) = paged_fixture(&[0xA0, 0x00, 0x90, 0x00, 0x00, 0xF4]);
    // A mapping that would send linear 0x9000 somewhere else if it were used.
    bus.poke_u32(pte_addr(DATA), 0x1_0000 | P | RW | US);
    bus.mem[DATA as usize] = 0x11;
    bus.mem[0x1_0000] = 0x22;
    cpu.cr0 &= !CR0_PG;

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();
    assert_eq!(cpu.al(), 0x11, "the page tables are not consulted");
}

/// The whole point of `#PF` being a fault: the handler repairs the mapping,
/// `IRETD` returns to the faulting instruction, and the instruction completes.
/// The page fault itself invalidated the cached translation (§4.10.4.1), so
/// the retry re-walks and sees the repaired entry without an `INVLPG`.
#[test]
fn handler_repairs_the_mapping_and_the_instruction_re_executes() {
    // C6 05 00 00 40 00 AA   MOV byte [0x00400000], 0xAA
    // F4                     HLT
    let (mut cpu, mut bus, mut mmu) =
        paged_fixture(&[0xC6, 0x05, 0x00, 0x00, 0x40, 0x00, 0xAA, 0xF4]);
    map_high_page(&mut bus, 0);

    // B8 03 90 00 00   MOV EAX, 0x9003        (DATA | P | RW)
    // A3 00 A0 00 00   MOV [0xA000], EAX      (the faulting PTE)
    // 83 C4 04         ADD ESP, 4             (discard the #PF error code)
    // CF               IRETD
    let mut handler = vec![0xB8];
    handler.extend_from_slice(&(DATA | P | RW).to_le_bytes());
    handler.push(0xA3);
    handler.extend_from_slice(&(PT2 as u32).to_le_bytes());
    handler.extend_from_slice(&[0x83, 0xC4, 0x04, 0xCF]);
    bus.write_bytes(HANDLER as usize, &handler);

    let steps = run_with_mmu(&mut cpu, &mut bus, &mut mmu, 64).unwrap();

    assert!(cpu.halted, "reached the HLT after the faulting instruction");
    assert_eq!(bus.peek_u8(DATA as usize), 0xAA, "the retried write landed");
    assert_eq!(
        cpu.gpr_u32(CpuState::RSP),
        STACK_TOP,
        "IRETD unwound the frame"
    );
    assert!(steps > 5, "fault, handler, and a re-execution happened");
}
