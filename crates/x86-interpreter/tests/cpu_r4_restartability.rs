//! Round-4 slice 3: a `#PF` is a fault, so the instruction re-executes and
//! must have committed nothing.
//!
//! Each test drives a real instruction into a page fault partway through and
//! then checks the architectural state the handler observes. Two rules are
//! being pinned:
//!
//! * A partially executed instruction commits no register, pointer or flag.
//! * A `REP` string operation is the exception the SDM writes down: completed
//!   iterations stand, the faulting iteration is retried, and `EFLAGS` goes
//!   back to its pre-instruction value.
//!
//! Spec: Intel SDM Vol. 2 "REP/REPE/REPZ/REPNE/REPNZ" (register state after a
//! suspending exception; the `CMPS`/`SCAS` `EFLAGS` rule), "PUSHA/PUSHAD",
//! "POPA/POPAD", "ENTER", "ADD"; Vol. 3 §4.7, §6.5 (faults), §6.12.1 (gate
//! stack frames).

mod common;

use common::*;
use x86_core::CpuState;
use x86_interpreter::{run_with_mmu, step_with_mmu, ExecError, ProtectedModeDeliveryError};

/// Slot of the saved `EFLAGS` in a frame pushed from `esp`.
fn frame_eflags(esp: u32) -> usize {
    (esp - 4) as usize
}
/// Slot of the error code in a frame pushed from `esp`.
fn frame_error_code(esp: u32) -> usize {
    (esp - 16) as usize
}

/// Every GPR but `RSP`, which the fault frame legitimately moves.
fn gprs_except_stack_pointer(cpu: &CpuState) -> Vec<u64> {
    cpu.gpr
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != CpuState::RSP)
        .map(|(_, value)| *value)
        .collect()
}

/// Remove a page from the identity mapping.
fn unmap(bus: &mut RamBus, linear: u32) {
    bus.poke_u32(pte_addr(linear), 0);
}

/// Handler that maps `linear` read/write, drops the error code and returns.
///
/// It clobbers `EBX` — a real handler would save it, but the stack under test
/// is deliberately close to an unmapped page — so callers must not assert on
/// `EBX` after a resume.
fn repair_handler(linear: u32) -> Vec<u8> {
    // BB <pte>      MOV EBX, <linear | P | RW>
    // 89 1D <addr>  MOV [pte_addr(linear)], EBX
    // 83 C4 04      ADD ESP, 4
    // CF            IRETD
    let mut code = vec![0xBB];
    code.extend_from_slice(&(linear | P | RW).to_le_bytes());
    code.extend_from_slice(&[0x89, 0x1D]);
    code.extend_from_slice(&(pte_addr(linear) as u32).to_le_bytes());
    code.extend_from_slice(&[0x83, 0xC4, 0x04, 0xCF]);
    code
}

/// `PUSHAD` that faults on its fifth slot commits none of the first four: the
/// stack pointer and every register are as they were, and after the handler
/// maps the page the whole eight-element frame is written by the retry.
#[test]
fn pushad_faulting_on_a_later_slot_commits_nothing() {
    // 60   PUSHAD
    // F4   HLT
    let (mut cpu, mut bus, mut mmu) = paged_fixture(&[0x60, 0xF4]);
    unmap(&mut bus, 0xB000);
    let esp = 0xC010u32;
    cpu.set_gpr_u32(CpuState::RSP, esp);
    for (index, value) in [0x1111_1111u32, 0x2222_2222, 0x3333_3333, 0x4444_4444]
        .into_iter()
        .enumerate()
    {
        cpu.set_gpr_u32(index, value);
    }
    let before = cpu.clone();

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();

    assert_eq!(cpu.rip, u64::from(HANDLER));
    assert_eq!(cpu.cr2, 0xBFFC, "the fifth slot is the faulting address");
    // The gate frame is pushed from the rolled-back ESP, so the handler's ESP
    // is the instruction-boundary value less the 16-byte frame.
    assert_eq!(cpu.gpr_u32(CpuState::RSP), esp - 16);
    assert_eq!(
        gprs_except_stack_pointer(&cpu),
        gprs_except_stack_pointer(&before),
        "no other register moved"
    );

    bus.write_bytes(HANDLER as usize, &repair_handler(0xB000));
    run_with_mmu(&mut cpu, &mut bus, &mut mmu, 64).unwrap();

    assert!(cpu.halted);
    assert_eq!(cpu.gpr_u32(CpuState::RSP), esp - 32, "all eight pushed");
    assert_eq!(bus.peek_u32((esp - 4) as usize), 0x1111_1111, "EAX");
    assert_eq!(bus.peek_u32((esp - 8) as usize), 0x2222_2222, "ECX");
    assert_eq!(bus.peek_u32((esp - 12) as usize), 0x3333_3333, "EDX");
    assert_eq!(bus.peek_u32((esp - 20) as usize), esp, "the original ESP");
}

/// `POPAD` that faults on its fifth slot leaves every register — including the
/// four it had already loaded — at its pre-instruction value.
#[test]
fn popad_faulting_on_a_later_slot_commits_no_register() {
    // 61   POPAD
    // F4   HLT
    let (mut cpu, mut bus, mut mmu) = paged_fixture(&[0x61, 0xF4]);
    unmap(&mut bus, 0xC000);
    let esp = 0xBFF0u32;
    cpu.set_gpr_u32(CpuState::RSP, esp);
    for slot in 0..8u32 {
        bus.poke_u32((esp + slot * 4) as usize, 0xAAAA_0000 | slot);
    }
    let before = cpu.clone();

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();

    assert_eq!(cpu.rip, u64::from(HANDLER));
    assert_eq!(cpu.cr2, 0xC000);
    assert_eq!(
        gprs_except_stack_pointer(&cpu),
        gprs_except_stack_pointer(&before),
        "EDI, ESI and EBP never took their popped values"
    );
    assert_eq!(
        cpu.gpr_u32(CpuState::RSP),
        esp - 16,
        "ESP only moved for the frame"
    );
}

/// A read-modify-write whose store faults after the read commits neither the
/// result nor the flags the computation produced. Without the rollback the
/// handler would see `ZF`/`CF` from an addition that never happened.
#[test]
fn read_modify_write_that_faults_on_the_store_commits_no_flags() {
    // 83 05 00 D0 00 00 01   ADD dword [0xD000], 1
    // F4                     HLT
    let (mut cpu, mut bus, mut mmu) =
        paged_fixture(&[0x83, 0x05, 0x00, 0xD0, 0x00, 0x00, 0x01, 0xF4]);
    // Readable but not writable, with CR0.WP=1 so even a supervisor write is
    // denied (SDM Vol. 3 §4.6.1).
    bus.poke_u32(pte_addr(0xD000), 0xD000 | P | US);
    cpu.cr0 |= CR0_WP;
    cpu.rflags = 0x0000_0003; // reserved bit 1, plus CF set
    bus.poke_u32(0xD000, 0xFFFF_FFFF);

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();

    assert_eq!(cpu.rip, u64::from(HANDLER));
    assert_eq!(cpu.cr2, 0xD000);
    assert_eq!(bus.peek_u32(0xD000), 0xFFFF_FFFF, "the store did not land");
    assert_eq!(
        bus.peek_u32(frame_eflags(STACK_TOP)),
        0x0000_0003,
        "the handler sees the pre-instruction flags, not ZF=1/CF=1 from the add"
    );
    assert_eq!(bus.peek_u32(frame_error_code(STACK_TOP)), 0x3, "P=1, W/R=1");
}

/// `ENTER` that faults while copying its display leaves both `EBP` and `ESP`
/// at the instruction boundary, so the retry rebuilds the same frame instead
/// of a frame nested one level deeper.
#[test]
fn enter_faulting_mid_display_restores_bp_and_sp() {
    // C8 00 00 03   ENTER 0, 3
    // F4            HLT
    let (mut cpu, mut bus, mut mmu) = paged_fixture(&[0xC8, 0x00, 0x00, 0x03, 0xF4]);
    unmap(&mut bus, 0xC000);
    let esp = 0xE000u32;
    let ebp = 0xD004u32;
    cpu.set_gpr_u32(CpuState::RSP, esp);
    cpu.set_gpr_u32(CpuState::RBP, ebp);

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();

    assert_eq!(cpu.rip, u64::from(HANDLER));
    assert_eq!(cpu.cr2, 0xCFFC, "the second display pointer is unreachable");
    assert_eq!(cpu.gpr_u32(CpuState::RBP), ebp, "EBP is not left mid-walk");
    assert_eq!(
        cpu.gpr_u32(CpuState::RSP),
        esp - 16,
        "ESP rolled back to the boundary before the fault frame was pushed"
    );
}

/// A `REP MOVSB` that faults partway keeps the iterations that finished.
/// `ECX` holds the count after the last successful iteration, `ESI`/`EDI`
/// point at the elements of the iteration to retry, and `EIP` points at the
/// string instruction — after which the instruction resumes rather than
/// restarting. Spec: SDM Vol. 2 "REP/REPE/REPZ/REPNE/REPNZ".
#[test]
fn rep_movsb_faulting_mid_string_resumes_from_the_failed_iteration() {
    // F3 A4   REP MOVSB
    // F4      HLT
    let (mut cpu, mut bus, mut mmu) = paged_fixture(&[0xF3, 0xA4, 0xF4]);
    unmap(&mut bus, 0xE000);
    let source = 0x1_0000u32;
    let destination = 0xDFFCu32;
    for offset in 0..8u32 {
        bus.mem[(source + offset) as usize] = 0x11 + offset as u8;
    }
    cpu.set_gpr_u32(CpuState::RSP, 0xD800);
    cpu.set_gpr_u32(CpuState::RSI, source);
    cpu.set_gpr_u32(CpuState::RDI, destination);
    cpu.set_gpr_u32(CpuState::RCX, 8);

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();

    assert_eq!(cpu.rip, u64::from(HANDLER));
    assert_eq!(cpu.cr2, 0xE000);
    assert_eq!(cpu.gpr_u32(CpuState::RCX), 4, "four iterations completed");
    assert_eq!(cpu.gpr_u32(CpuState::RSI), source + 4);
    assert_eq!(cpu.gpr_u32(CpuState::RDI), 0xE000);
    assert_eq!(bus.peek_u32(frame_eflags(0xD800) - 8), CODE, "EIP restarts");
    assert_eq!(
        &bus.mem[destination as usize..(destination + 4) as usize],
        &[0x11, 0x12, 0x13, 0x14],
        "the finished iterations really did copy"
    );

    bus.write_bytes(HANDLER as usize, &repair_handler(0xE000));
    run_with_mmu(&mut cpu, &mut bus, &mut mmu, 64).unwrap();

    assert!(cpu.halted);
    assert_eq!(
        cpu.gpr_u32(CpuState::RCX),
        0,
        "the resume drained the count"
    );
    assert_eq!(
        &bus.mem[0xE000..0xE004],
        &[0x15, 0x16, 0x17, 0x18],
        "resumption started at the faulting element, not at the beginning"
    );
}

/// The `REPE CMPS` special case: index and count progress survives, but
/// `EFLAGS` is restored to the value it had before the instruction started.
/// Spec: SDM Vol. 2 REP — "When a fault occurs during the execution of a CMPS
/// or SCAS instruction that is prefixed with REPE or REPNE, the EFLAGS value
/// is restored to the state prior to the execution of the instruction."
#[test]
fn repe_cmpsb_faulting_mid_string_restores_pre_instruction_eflags() {
    // F3 A6   REPE CMPSB
    // F4      HLT
    let (mut cpu, mut bus, mut mmu) = paged_fixture(&[0xF3, 0xA6, 0xF4]);
    unmap(&mut bus, 0xE000);
    let source = 0x1_0000u32;
    let destination = 0xDFFDu32;
    for offset in 0..8u32 {
        bus.mem[(source + offset) as usize] = 0x42;
        if destination + offset < 0xE000 {
            bus.mem[(destination + offset) as usize] = 0x42;
        }
    }
    let esp = 0xD800u32;
    cpu.set_gpr_u32(CpuState::RSP, esp);
    cpu.set_gpr_u32(CpuState::RSI, source);
    cpu.set_gpr_u32(CpuState::RDI, destination);
    cpu.set_gpr_u32(CpuState::RCX, 8);
    // CF set and ZF clear before the instruction; three equal comparisons
    // would otherwise leave ZF set and CF clear.
    cpu.rflags = 0x0000_0003;

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();

    assert_eq!(cpu.rip, u64::from(HANDLER));
    assert_eq!(cpu.gpr_u32(CpuState::RCX), 5, "three iterations completed");
    assert_eq!(cpu.gpr_u32(CpuState::RSI), source + 3);
    assert_eq!(cpu.gpr_u32(CpuState::RDI), 0xE000);
    assert_eq!(
        bus.peek_u32(frame_eflags(esp)),
        0x0000_0003,
        "EFLAGS returns to its pre-instruction value"
    );
}

/// A gate frame that cannot be written completely writes nothing: the bytes
/// the delivery had already stored are restored, and `CS:EIP` and `ESP` stay
/// where they were. Nested `#DF` synthesis is still out of scope, so the
/// bounded delivery failure is reported to the host instead.
/// Spec: SDM Vol. 3 §6.12.1 (frame layout); §6.15 (`#DF`, not modelled).
#[test]
fn gate_frame_that_cannot_be_written_rolls_every_byte_back() {
    // CD 20   INT 0x20
    // F4      HLT
    let (mut cpu, mut bus, mut mmu) = paged_fixture(&[0xCD, 0x20, 0xF4]);
    install_386_idt(&mut bus, &mut cpu, 0x4000, &[(0x20, HANDLER)]);
    // Readable but not writable, with WP=1: the delivery's snapshot read of
    // the original bytes succeeds and only the store is refused.
    bus.poke_u32(pte_addr(0xC000), 0xC000 | P | US);
    cpu.cr0 |= CR0_WP;
    let esp = 0xD004u32;
    cpu.set_gpr_u32(CpuState::RSP, esp);
    bus.poke_u32(0xD000, 0xDEAD_BEEF);
    let before = cpu.clone();

    let result = step_with_mmu(&mut cpu, &mut bus, &mut mmu);

    assert!(
        matches!(
            result,
            Err(ExecError::ProtectedModeExceptionDelivery {
                vector: 0x20,
                reason: ProtectedModeDeliveryError::StackWrite(_)
                    | ProtectedModeDeliveryError::StackRollback(_),
            })
        ),
        "reported, not half-delivered: {result:?}"
    );
    assert_eq!(
        bus.peek_u32(0xD000),
        0xDEAD_BEEF,
        "the EFLAGS slot is intact"
    );
    assert_eq!(cpu.gpr_u32(CpuState::RSP), esp);
    assert_eq!(cpu.rip, before.rip);
    assert_eq!(cpu.cs.selector, before.cs.selector);
}
