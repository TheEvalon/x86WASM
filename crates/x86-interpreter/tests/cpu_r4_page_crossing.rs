//! Round-4 slice 4: accesses and instruction fetches that straddle a 4-KiB
//! page boundary.
//!
//! The engine translates one address. Splitting a word or doubleword at the
//! boundary, translating both halves, and finding a second-half fault *before*
//! the first half is committed is caller work, and it is the difference
//! between a restartable instruction and a half-written operand.
//!
//! Spec: Intel SDM Vol. 3 §4.3 (one translation per linear address), §4.7
//! (`#PF` and `CR2`), §4.8 (accessed/dirty), §6.5 (fault semantics); Vol. 2
//! "MOV".

mod common;

use common::*;
use x86_core::CpuState;
use x86_interpreter::{run_with_mmu, step_with_mmu};

/// Remove a page from the identity mapping.
fn unmap(bus: &mut RamBus, linear: u32) {
    bus.poke_u32(pte_addr(linear), 0);
}

/// Map `linear` to a frame somewhere else entirely, so a test can tell a real
/// translation from an accidental identity pass-through.
fn remap(bus: &mut RamBus, linear: u32, frame: u32) {
    bus.poke_u32(pte_addr(linear), frame | P | RW | US);
}

/// A dword read whose four bytes fall in two different pages assembles them
/// from both translations — which are deliberately not adjacent physically.
#[test]
fn a_dword_read_across_a_page_boundary_uses_both_translations() {
    // A1 FE 0F 01 00   MOV EAX, [0x00010FFE]
    let (mut cpu, mut bus, mut mmu) = paged_fixture(&[0xA1, 0xFE, 0x0F, 0x01, 0x00, 0xF4]);
    // Linear 0x10000 keeps its identity frame; linear 0x11000 moves to 0xD000,
    // so an untranslated read would see zeroes instead of the high half.
    remap(&mut bus, 0x1_1000, 0xD000);
    bus.mem[0x1_0FFE] = 0x11;
    bus.mem[0x1_0FFF] = 0x22;
    bus.mem[0xD000] = 0x33;
    bus.mem[0xD001] = 0x44;

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();
    assert_eq!(cpu.gpr_u32(CpuState::RAX), 0x4433_2211);
}

/// The matching write splits the same way.
#[test]
fn a_dword_write_across_a_page_boundary_lands_in_both_frames() {
    // A3 FE 0F 01 00   MOV [0x00010FFE], EAX
    let (mut cpu, mut bus, mut mmu) = paged_fixture(&[0xA3, 0xFE, 0x0F, 0x01, 0x00, 0xF4]);
    remap(&mut bus, 0x1_1000, 0xD000);
    cpu.set_gpr_u32(CpuState::RAX, 0xDDCC_BBAA);

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();
    assert_eq!(
        bus.peek_u16(0x1_0FFE),
        0xBBAA,
        "low half in the first frame"
    );
    assert_eq!(bus.peek_u16(0xD000), 0xDDCC, "high half in the second");
}

/// The case the split exists for: the second page faults, and not one byte of
/// the first half has been written. A retry after the handler repairs the
/// mapping writes the whole operand.
#[test]
fn a_write_whose_second_page_faults_commits_no_byte_of_the_first() {
    // A3 FE 0F 01 00   MOV [0x00010FFE], EAX
    // F4               HLT
    let (mut cpu, mut bus, mut mmu) = paged_fixture(&[0xA3, 0xFE, 0x0F, 0x01, 0x00, 0xF4]);
    unmap(&mut bus, 0x1_1000);
    cpu.set_gpr_u32(CpuState::RAX, 0xDDCC_BBAA);
    bus.poke_u32(0x1_0FFC, 0x5555_5555);

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();

    assert_eq!(cpu.rip, u64::from(HANDLER));
    assert_eq!(cpu.cr2, 0x1_1000, "CR2 names the unreachable half");
    assert_eq!(
        bus.peek_u32(0x1_0FFC),
        0x5555_5555,
        "the reachable half was left alone"
    );

    // BB <pte>      MOV EBX, 0x11003
    // 89 1D <addr>  MOV [pte], EBX
    // 83 C4 04      ADD ESP, 4
    // CF            IRETD
    let mut handler = vec![0xBB];
    handler.extend_from_slice(&(0x1_1000u32 | P | RW).to_le_bytes());
    handler.extend_from_slice(&[0x89, 0x1D]);
    handler.extend_from_slice(&(pte_addr(0x1_1000) as u32).to_le_bytes());
    handler.extend_from_slice(&[0x83, 0xC4, 0x04, 0xCF]);
    bus.write_bytes(HANDLER as usize, &handler);

    run_with_mmu(&mut cpu, &mut bus, &mut mmu, 64).unwrap();
    assert!(cpu.halted);
    assert_eq!(bus.peek_u16(0x1_0FFE), 0xBBAA);
    assert_eq!(bus.peek_u16(0x1_1000), 0xDDCC);
}

/// A split access that faults also writes no accessed or dirty flag on the
/// half it *could* have reached: both halves are probed before either is
/// translated. Spec: SDM Vol. 3 §4.8, §4.10.2.3.
#[test]
fn a_faulting_split_write_sets_no_flag_on_the_reachable_half() {
    // A3 FE 0F 01 00   MOV [0x00010FFE], EAX
    let (mut cpu, mut bus, mut mmu) = paged_fixture(&[0xA3, 0xFE, 0x0F, 0x01, 0x00, 0xF4]);
    unmap(&mut bus, 0x1_1000);
    // Start the reachable half with A and D clear so a stray update shows.
    bus.poke_u32(pte_addr(0x1_0000), 0x1_0000 | P | RW | US);
    cpu.set_gpr_u32(CpuState::RAX, 0xDDCC_BBAA);

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();

    assert_eq!(cpu.rip, u64::from(HANDLER), "the access faulted");
    assert_eq!(
        bus.peek_u32(pte_addr(0x1_0000)) & (A | D),
        0,
        "the first half was probed, never translated"
    );
}

/// A word access that stops one byte short of the boundary is not a split, and
/// a word access that starts on the boundary is not either — the split logic
/// must trigger on the actual overlap, not on being near a page edge.
#[test]
fn accesses_that_only_touch_the_boundary_are_not_split() {
    // 66 A1 FC 0F 01 00   MOV AX, [0x00010FFC]
    // 66 A1 00 10 01 00   MOV AX, [0x00011000]
    let (mut cpu, mut bus, mut mmu) = paged_fixture(&[
        0x66, 0xA1, 0xFC, 0x0F, 0x01, 0x00, 0x66, 0xA1, 0x00, 0x10, 0x01, 0x00, 0xF4,
    ]);
    unmap(&mut bus, 0x1_1000);
    bus.poke_u32(0x1_0FFC, 0x0000_9876);

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();
    assert_eq!(cpu.ax(), 0x9876, "wholly inside the mapped page");

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();
    assert_eq!(cpu.rip, u64::from(HANDLER), "wholly inside the absent page");
    assert_eq!(cpu.cr2, 0x1_1000);
}

/// An instruction that straddles a page boundary faults on the byte it cannot
/// fetch, with that byte's linear address in `CR2` and the instruction's own
/// `EIP` saved — the fetch is a fault from the "fetching next instruction"
/// class, so nothing of the instruction has begun.
/// Spec: SDM Vol. 3 §6.9 Table 6-2 (priority class 8), §4.7.
#[test]
fn an_instruction_fetch_that_straddles_a_boundary_faults_on_the_second_page() {
    let (mut cpu, mut bus, mut mmu) = paged_fixture(&[0xF4]);
    unmap(&mut bus, 0x1_1000);
    // Place a 5-byte MOV EAX, imm32 so its last two bytes fall in 0x11000.
    let start = 0x1_0FFDu32;
    bus.write_bytes(start as usize, &[0xB8, 0x11, 0x22, 0x33, 0x44]);
    // E9 <rel32>  JMP 0x00010FFD
    let displacement = (start as i64 - (CODE as i64 + 5)) as i32;
    let mut jump = vec![0xE9];
    jump.extend_from_slice(&displacement.to_le_bytes());
    bus.write_bytes(CODE as usize, &jump);
    let eax_before = cpu.gpr_u32(CpuState::RAX);

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();
    assert_eq!(cpu.rip, u64::from(start), "the jump lands mid-page");

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();
    assert_eq!(cpu.rip, u64::from(HANDLER));
    assert_eq!(cpu.cr2, 0x1_1000, "the first unreachable instruction byte");
    assert_eq!(bus.peek_u32(FRAME_ERROR_CODE), 0x0, "not present, a fetch");
    assert_eq!(bus.peek_u32(FRAME_EIP), start, "the instruction restarts");
    assert_eq!(cpu.gpr_u32(CpuState::RAX), eax_before, "nothing executed");
}

/// Once the second page is mapped the same straddling instruction executes
/// normally, so the fault really was about reachability and not about decode.
#[test]
fn a_straddling_instruction_executes_once_both_pages_are_present() {
    let (mut cpu, mut bus, mut mmu) = paged_fixture(&[0xF4]);
    remap(&mut bus, 0x1_1000, 0xD000);
    let start = 0x1_0FFDu32;
    // B8 11 22 33 44  MOV EAX, 0x44332211, split 3/2 across the boundary.
    bus.write_bytes(start as usize, &[0xB8, 0x11, 0x22]);
    bus.write_bytes(0xD000, &[0x33, 0x44]);
    bus.write_bytes(0xD002, &[0xF4]);
    let displacement = (start as i64 - (CODE as i64 + 5)) as i32;
    let mut jump = vec![0xE9];
    jump.extend_from_slice(&displacement.to_le_bytes());
    bus.write_bytes(CODE as usize, &jump);

    run_with_mmu(&mut cpu, &mut bus, &mut mmu, 16).unwrap();
    assert!(cpu.halted);
    assert_eq!(cpu.gpr_u32(CpuState::RAX), 0x4433_2211);
}

/// `INS` reads its port before it stores, and a port read cannot be replayed
/// by an instruction-boundary rollback. The destination is therefore probed
/// first, so an unwritable destination costs no port read at all.
/// Spec: SDM Vol. 2 "INS/INSB/INSW/INSD".
#[test]
fn ins_probes_its_destination_before_reading_the_port() {
    // 6C   INSB
    // F4   HLT
    let (mut cpu, mut bus, mut mmu) = paged_fixture(&[0x6C, 0xF4]);
    unmap(&mut bus, 0xD000);
    cpu.set_gpr_u32(CpuState::RDI, 0xD000);
    cpu.set_gpr_u32(CpuState::RDX, 0x0060);

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();

    assert_eq!(cpu.rip, u64::from(HANDLER));
    assert_eq!(cpu.cr2, 0xD000);
    assert!(bus.ports.is_empty(), "no port cycle was started");
    assert_eq!(cpu.gpr_u32(CpuState::RDI), 0xD000, "EDI did not advance");
}
