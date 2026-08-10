//! Round-4 slice 1: `CR2`/`CR3`/`CR4` as guest-writable state, the `CPUID`
//! bits that licence `CR4.PSE` and `CR4.PGE`, and the TLB-invalidation hooks.
//!
//! Spec: Intel SDM Vol. 2 "MOV—Move to/from Control Registers", "INVLPG",
//! "LMSW", "CPUID"; Vol. 3 §2.5 (CR0), §4.1.1 (paging-mode selection), §4.1.3
//! (paging-mode modifiers), §4.1.4 (feature enumeration), Table 4-3 (CR3),
//! §4.7 (CR2), §4.10.4.1 (invalidation), §5.5 (privilege levels).

mod common;

use common::*;
use x86_core::CpuState;
use x86_interpreter::{step, step_with_mmu, ExecError};
use x86_mmu::paging::{Mmu, PageSize, TlbEntry};

/// A cached translation, so a test can watch an invalidation happen.
fn cached(global: bool) -> TlbEntry {
    TlbEntry {
        frame_base: 0x0004_0000,
        page_size: PageSize::Size4KiB,
        writable: true,
        user_accessible: false,
        global,
        dirty: false,
        final_entry_addr: 0x2000,
    }
}

/// `MOV CR4, r32` then `MOV r32, CR4` round-trips the two implemented bits.
/// Reset leaves `CR4` zero (SDM Vol. 3 Table 9-1 / §2.5).
#[test]
fn mov_cr4_round_trips_pse_and_pge() {
    let mut bus = RamBus::new(0x10000);
    // B8 90 00 00 00   MOV EAX, 0x90       (PSE | PGE)
    // 0F 22 E0         MOV CR4, EAX
    // 0F 20 E3         MOV EBX, CR4
    // F4               HLT
    bus.write_bytes(
        0x1000,
        &[
            0x66, 0xB8, 0x90, 0x00, 0x00, 0x00, 0x0F, 0x22, 0xE0, 0x0F, 0x20, 0xE3, 0xF4,
        ],
    );

    let mut cpu = real_mode_cpu(0x1000, 0xFFFE);
    assert_eq!(cpu.cr4, 0, "CR4 is zero at reset");

    step(&mut cpu, &mut bus).unwrap();
    step(&mut cpu, &mut bus).unwrap();
    assert_eq!(cpu.cr4, CR4_PSE | CR4_PGE);
    step(&mut cpu, &mut bus).unwrap();
    assert_eq!(cpu.gpr_u32(CpuState::RBX), 0x90);
}

/// Every `CR4` bit outside `VME`/`PSE`/`PGE` is reserved here, so writing 1 to
/// one raises `#GP(0)` and leaves `CR4` alone. `CR4.VME` is writable for the
/// Round-12 redirect stub without `CPUID.VME` (see `cpu_r12_cr4_vme_honesty`).
/// `CR4.PAE` matters most among the reserved bits: refusing it stops a guest
/// selecting the paging mode the engine reports as unsupported.
/// Spec: SDM Vol. 2 MOV CRn (#GP on reserved CR4 bit); Vol. 3 §4.1.4 / §2.5.
#[test]
fn mov_cr4_reserved_bits_raise_gp_and_commit_nothing() {
    // bit0 VME is implemented (sticky); probe PVI, PAE, PCE, OSXSAVE.
    for reserved in [1u32 << 1, 1 << 5, 1 << 8, 1 << 18] {
        let mut bus = RamBus::new(0x10000);
        let mut cpu = real_mode_cpu(0x1000, 0xFFFE);
        install_ivt(&mut bus, &mut cpu, 13, 0x0000, 0x0900);

        let mut code = vec![0x66, 0xB8];
        code.extend_from_slice(&reserved.to_le_bytes());
        code.extend_from_slice(&[0x0F, 0x22, 0xE0, 0xF4]);
        bus.write_bytes(0x1000, &code);
        bus.write_bytes(0x0900, &[0xF4]);

        step(&mut cpu, &mut bus).unwrap();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.cr4, 0, "reserved bit {reserved:#x} must not be stored");
        assert_eq!(cpu.rip, 0x0900, "#GP handler entered");
        // Fault semantics: the pushed IP is the faulting instruction.
        assert_eq!(bus.peek_u16(0xFFF8), 0x1006);
    }
}

/// `CR2` is plain architectural state with no reserved bits (SDM Vol. 3 §4.7).
#[test]
fn mov_cr2_round_trips_a_full_linear_address() {
    let mut bus = RamBus::new(0x10000);
    // 66 B8 EF BE AD DE  MOV EAX, 0xDEADBEEF
    // 0F 22 D0           MOV CR2, EAX
    // 0F 20 D1           MOV ECX, CR2
    bus.write_bytes(
        0x1000,
        &[
            0x66, 0xB8, 0xEF, 0xBE, 0xAD, 0xDE, 0x0F, 0x22, 0xD0, 0x0F, 0x20, 0xD1, 0xF4,
        ],
    );
    let mut cpu = real_mode_cpu(0x1000, 0xFFFE);

    for _ in 0..3 {
        step(&mut cpu, &mut bus).unwrap();
    }
    assert_eq!(cpu.cr2, 0xDEAD_BEEF);
    assert_eq!(cpu.gpr_u32(CpuState::RCX), 0xDEAD_BEEF);
}

/// `MOV to CR3` ignores bits 2:0 and 11:5 rather than reserving them, so a
/// write that sets them stores them and raises nothing; the page-directory
/// base is bits 31:12. Spec: SDM Vol. 3 Table 4-3.
#[test]
fn mov_cr3_stores_ignored_bits_without_faulting() {
    let mut bus = RamBus::new(0x10000);
    bus.write_bytes(
        0x1000,
        &[
            0x66, 0xB8, 0xFF, 0x3F, 0x12, 0x00, 0x0F, 0x22, 0xD8, 0x0F, 0x20, 0xD9, 0xF4,
        ],
    );
    let mut cpu = real_mode_cpu(0x1000, 0xFFFE);

    for _ in 0..3 {
        step(&mut cpu, &mut bus).unwrap();
    }
    assert_eq!(cpu.cr3, 0x0012_3FFF);
    assert_eq!(cpu.gpr_u32(CpuState::RCX), 0x0012_3FFF);
}

/// `MOV to CR3` invalidates every non-global entry — including when it stores
/// the value `CR3` already held, which is why the hook is per-instruction and
/// not a changed-value comparison. Spec: SDM Vol. 3 §4.10.4.1.
#[test]
fn mov_cr3_flushes_non_global_entries_even_when_the_value_is_unchanged() {
    let mut bus = RamBus::new(0x10000);
    // 66 B8 00 10 00 00  MOV EAX, 0x1000
    // 0F 22 D8           MOV CR3, EAX
    bus.write_bytes(
        0x1000,
        &[0x66, 0xB8, 0x00, 0x10, 0x00, 0x00, 0x0F, 0x22, 0xD8, 0xF4],
    );
    let mut cpu = real_mode_cpu(0x1000, 0xFFFE);
    cpu.cr3 = 0x1000;

    let mut mmu = Mmu::new();
    mmu.tlb_mut().insert(0x0040_0000, cached(false));
    mmu.tlb_mut().insert(0x0080_0000, cached(true));

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();
    assert_eq!(mmu.tlb().len(), 2, "MOV EAX, imm32 invalidates nothing");

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();
    assert_eq!(cpu.cr3, 0x1000);
    assert_eq!(mmu.tlb().len(), 1, "non-global entry dropped");
    assert!(mmu.tlb().lookup(0x0080_0000).is_some(), "global entry kept");
}

/// `MOV to CR0` that clears `CR0.PG` invalidates everything, global entries
/// included. Spec: SDM Vol. 3 §4.10.4.1.
#[test]
fn mov_cr0_clearing_pg_flushes_global_entries_too() {
    // B8 01 00 00 00   MOV EAX, 1        (PE set, PG clear)
    // 0F 22 C0         MOV CR0, EAX
    let (mut cpu, mut bus, mut mmu) =
        paged_fixture(&[0xB8, 0x01, 0x00, 0x00, 0x00, 0x0F, 0x22, 0xC0, 0xF4]);
    mmu.tlb_mut().insert(HIGH, cached(true));

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();
    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();
    assert_eq!(cpu.cr0, CR0_PE);
    assert!(mmu.tlb().is_empty());
}

/// Paging requires protected mode: `MOV to CR0` that would set `PG` while `PE`
/// is clear raises `#GP(0)` and stores nothing.
/// Spec: SDM Vol. 2 MOV CRn; Vol. 3 §4.1.1.
#[test]
fn mov_cr0_setting_pg_without_pe_raises_gp() {
    let mut bus = RamBus::new(0x10000);
    // 66 B8 00 00 00 80  MOV EAX, 0x80000000
    // 0F 22 C0           MOV CR0, EAX
    bus.write_bytes(
        0x1000,
        &[0x66, 0xB8, 0x00, 0x00, 0x00, 0x80, 0x0F, 0x22, 0xC0, 0xF4],
    );
    let mut cpu = real_mode_cpu(0x1000, 0xFFFE);
    install_ivt(&mut bus, &mut cpu, 13, 0x0000, 0x0900);
    bus.write_bytes(0x0900, &[0xF4]);
    let cr0_before = cpu.cr0;

    step(&mut cpu, &mut bus).unwrap();
    step(&mut cpu, &mut bus).unwrap();
    assert_eq!(cpu.cr0, cr0_before);
    assert_eq!(cpu.rip, 0x0900);
}

/// `INVLPG m` drops the entries for exactly that page number and leaves the
/// rest — including other global entries — alone.
/// Spec: SDM Vol. 2 "INVLPG"; Vol. 3 §4.10.4.1.
#[test]
fn invlpg_invalidates_only_the_addressed_page() {
    let mut bus = RamBus::new(0x10000);
    // 67 0F 01 38        INVLPG [EAX]
    bus.write_bytes(0x1000, &[0x67, 0x0F, 0x01, 0x38, 0xF4]);
    let mut cpu = real_mode_cpu(0x1000, 0xFFFE);
    // Unreal-mode expanded DS limit, so the operand address is legal without a
    // segment-limit #GP (SDM Vol. 3 §3.4.3).
    cpu.ds.limit = 0xFFFF_FFFF;
    cpu.set_gpr_u32(CpuState::RAX, 0x0040_0000);

    let mut mmu = Mmu::new();
    mmu.tlb_mut().insert(0x0040_0000, cached(true));
    mmu.tlb_mut().insert(0x0080_0000, cached(true));

    // The operand address is far outside this bus's RAM: a step that succeeds
    // proves INVLPG read and wrote nothing there.
    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();
    assert_eq!(cpu.rip, 0x1004, "INVLPG advances past the instruction");
    assert!(
        mmu.tlb().lookup(0x0040_0000).is_none(),
        "global entry for the addressed page is invalidated"
    );
    assert!(mmu.tlb().lookup(0x0080_0000).is_some());
}

/// `INVLPG` with a register operand is `#UD` (SDM Vol. 2 "INVLPG").
#[test]
fn invlpg_register_form_is_ud() {
    let mut bus = RamBus::new(0x10000);
    bus.write_bytes(0x1000, &[0x0F, 0x01, 0xF8, 0xF4]);
    let mut cpu = real_mode_cpu(0x1000, 0xFFFE);
    install_ivt(&mut bus, &mut cpu, 6, 0x0000, 0x0B00);
    bus.write_bytes(0x0B00, &[0xF4]);

    step(&mut cpu, &mut bus).unwrap();
    assert_eq!(cpu.rip, 0x0B00);
}

/// `CPUID.01H:EDX` advertises exactly the four implemented features, and the
/// paging features the engine does *not* model stay clear — the engine's
/// default reserved-bit profile depends on `PAT` and `PSE-36` being absent.
/// Spec: SDM Vol. 2 "CPUID"; Vol. 3 §4.1.4; `AGENTS.md` truthful CPUID.
#[test]
fn cpuid_leaf1_advertises_pse_pge_cmov_and_msr_only() {
    let mut bus = RamBus::new(0x10000);
    // 66 B8 01 00 00 00  MOV EAX, 1
    // 0F A2              CPUID
    bus.write_bytes(
        0x1000,
        &[0x66, 0xB8, 0x01, 0x00, 0x00, 0x00, 0x0F, 0xA2, 0xF4],
    );
    let mut cpu = real_mode_cpu(0x1000, 0xFFFE);

    step(&mut cpu, &mut bus).unwrap();
    step(&mut cpu, &mut bus).unwrap();

    let edx = cpu.gpr_u32(CpuState::RDX);
    let expected = (1 << 3) | (1 << 5) | (1 << 13) | (1 << 15);
    assert_eq!(edx, expected, "PSE | MSR | PGE | CMOV and nothing else");
    assert_eq!(edx & (1 << 6), 0, "PAE not implemented");
    assert_eq!(edx & (1 << 16), 0, "PAT not implemented");
    assert_eq!(edx & (1 << 17), 0, "PSE-36 not implemented");
    assert_eq!(
        cpu.gpr_u32(CpuState::RCX),
        0,
        "no ECX feature is implemented"
    );
    // Family 6 is the generation that introduced PGE and CMOV.
    assert_eq!(cpu.gpr_u32(CpuState::RAX), 0x0000_0600);
}

/// `MOV to/from CRn` and `INVLPG` are ring-0 instructions: at CPL 3 they raise
/// `#GP(0)` before touching anything. Spec: SDM Vol. 2 MOV CRn / INVLPG
/// (Protected Mode Exceptions); Vol. 3 §5.5.
#[test]
fn control_register_access_outside_ring0_raises_gp() {
    for code in [
        vec![0x0F, 0x20, 0xD8], // MOV EAX, CR3
        vec![0x0F, 0x22, 0xE0], // MOV CR4, EAX
        vec![0x0F, 0x01, 0x38], // INVLPG [EAX]
    ] {
        let mut bus = RamBus::new(0x2_0000);
        let mut cpu = flat_protected_cpu(0x1_0000, 0x1_8000);
        cpu.cr0 = CR0_PE;
        cpu.cr3 = 0;
        install_flat_gdt(&mut bus, &mut cpu, 0x3000);
        install_386_idt(&mut bus, &mut cpu, 0x4000, &[(13, 0x1_1000)]);
        // The #GP gate must enter at the faulting CPL, so target the ring-3
        // code selector.
        bus.write_bytes(0x4000 + 13 * 8 + 2, &[0x1B, 0x00]);
        bus.write_bytes(0x4000 + 13 * 8 + 5, &[0xEE]);
        to_ring3(&mut cpu);
        bus.write_bytes(0x1_0000, &code);
        bus.write_bytes(0x1_1000, &[0xF4]);
        let cr4_before = cpu.cr4;

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.rip, 0x1_1000, "#GP handler entered for {code:02X?}");
        assert_eq!(cpu.cr4, cr4_before);
    }
}

/// `LMSW` cannot reach `PG` or `WP`, so it implies no invalidation even though
/// it writes `CR0`. Spec: SDM Vol. 2 "LMSW"; Vol. 3 §4.10.4.1.
#[test]
fn lmsw_does_not_invalidate_the_tlb() {
    let mut bus = RamBus::new(0x10000);
    // B8 11 00     MOV AX, 0x11
    // 0F 01 F0     LMSW AX
    bus.write_bytes(0x1000, &[0xB8, 0x11, 0x00, 0x0F, 0x01, 0xF0, 0xF4]);
    let mut cpu = real_mode_cpu(0x1000, 0xFFFE);

    let mut mmu = Mmu::new();
    mmu.tlb_mut().insert(0x0040_0000, cached(false));

    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();
    step_with_mmu(&mut cpu, &mut bus, &mut mmu).unwrap();
    assert_eq!(cpu.cr0 & 0xFFFF, 0x11);
    assert_eq!(mmu.tlb().len(), 1);
}

/// `MOV to CR1` and `CR5`-`CR7` are `#UD` — unchanged by this slice, kept here
/// so widening the accepted `reg` set cannot silently swallow them.
/// Spec: SDM Vol. 2 MOV CRn.
#[test]
fn mov_cr1_and_cr5_through_cr7_remain_ud() {
    for modrm in [0xC8u8, 0xE8, 0xF0, 0xF8] {
        let mut bus = RamBus::new(0x10000);
        bus.write_bytes(0x1000, &[0x0F, 0x22, modrm, 0xF4]);
        let mut cpu = real_mode_cpu(0x1000, 0xFFFE);
        install_ivt(&mut bus, &mut cpu, 6, 0x0000, 0x0B00);
        bus.write_bytes(0x0B00, &[0xF4]);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.rip, 0x0B00, "modrm {modrm:#04x} is #UD");
    }
}

/// The interpreter never returns `Unsupported` for `MOV CR2/CR3/CR4` any more.
#[test]
fn mov_cr2_cr3_cr4_are_no_longer_unsupported() {
    for modrm in [0xD0u8, 0xD8, 0xE0] {
        let mut bus = RamBus::new(0x10000);
        bus.write_bytes(0x1000, &[0x0F, 0x22, modrm, 0x0F, 0x20, modrm, 0xF4]);
        let mut cpu = real_mode_cpu(0x1000, 0xFFFE);

        assert!(!matches!(
            step(&mut cpu, &mut bus),
            Err(ExecError::Unsupported(_))
        ));
        assert!(!matches!(
            step(&mut cpu, &mut bus),
            Err(ExecError::Unsupported(_))
        ));
    }
}
