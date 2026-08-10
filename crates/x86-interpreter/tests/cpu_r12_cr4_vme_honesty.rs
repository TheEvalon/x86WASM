//! Round-12 slice 1: `CR4.VME` write honesty without advertising `CPUID.VME`.
//!
//! Partial Virtual-8086 Mode Extensions: `CR4` bit 0 may be set/cleared so later
//! slices can exercise the interrupt-redirection stub, but leaf-1 `EDX[1]`
//! (`VME`) stays clear until VIF/VIP/`CLI`/`STI` and full Table 20-2 land.
//!
//! Spec: Intel SDM Vol. 3 §2.5 (CR4.VME); Vol. 2 "MOV—Move to/from Control
//! Registers", "CPUID" Table 3-11; Vol. 3 §4.1.4 (feature enumeration).

mod common;

use common::*;
use x86_core::CpuState;
use x86_interpreter::step;

/// `CR4.VME` is bit 0 (SDM Vol. 3 §2.5).
const CR4_VME: u64 = 1 << 0;

/// `MOV CR4` may store `VME`; `MOV` from `CR4` reads it back.
#[test]
fn mov_cr4_vme_round_trips_without_cpuid_vme() {
    let mut bus = RamBus::new(0x10000);
    // 66 B8 01 00 00 00  MOV EAX, 1        (VME)
    // 0F 22 E0           MOV CR4, EAX
    // 0F 20 E3           MOV EBX, CR4
    // 0F A2              CPUID (EAX=1)
    // F4                 HLT
    bus.write_bytes(
        0x1000,
        &[
            0x66, 0xB8, 0x01, 0x00, 0x00, 0x00, // MOV EAX, 1
            0x0F, 0x22, 0xE0, // MOV CR4, EAX
            0x0F, 0x20, 0xE3, // MOV EBX, CR4
            0x66, 0xB8, 0x01, 0x00, 0x00, 0x00, // MOV EAX, 1
            0x0F, 0xA2, // CPUID
            0xF4,
        ],
    );

    let mut cpu = real_mode_cpu(0x1000, 0xFFFE);
    assert_eq!(cpu.cr4, 0);

    step(&mut cpu, &mut bus).unwrap(); // MOV EAX,1
    step(&mut cpu, &mut bus).unwrap(); // MOV CR4
    assert_eq!(cpu.cr4, CR4_VME, "CR4.VME must stick");
    step(&mut cpu, &mut bus).unwrap(); // MOV EBX, CR4
    assert_eq!(cpu.gpr_u32(CpuState::RBX), 1);

    step(&mut cpu, &mut bus).unwrap(); // MOV EAX,1
    step(&mut cpu, &mut bus).unwrap(); // CPUID
    let edx = cpu.gpr_u32(CpuState::RDX);
    assert_eq!(edx & (1 << 1), 0, "CPUID.01H:EDX.VME must stay clear");
}

/// Clearing `CR4.VME` after it was set leaves other implemented bits alone.
#[test]
fn mov_cr4_clears_vme_while_keeping_pse() {
    let mut bus = RamBus::new(0x10000);
    // MOV EAX, VME|PSE; MOV CR4,EAX; MOV EAX, PSE; MOV CR4,EAX; MOV EBX,CR4
    bus.write_bytes(
        0x1000,
        &[
            0x66, 0xB8, 0x11, 0x00, 0x00, 0x00, // EAX = VME|PSE
            0x0F, 0x22, 0xE0, // MOV CR4, EAX
            0x66, 0xB8, 0x10, 0x00, 0x00, 0x00, // EAX = PSE
            0x0F, 0x22, 0xE0, // MOV CR4, EAX
            0x0F, 0x20, 0xE3, // MOV EBX, CR4
            0xF4,
        ],
    );
    let mut cpu = real_mode_cpu(0x1000, 0xFFFE);
    for _ in 0..5 {
        step(&mut cpu, &mut bus).unwrap();
    }
    assert_eq!(cpu.cr4, CR4_PSE);
    assert_eq!(cpu.gpr_u32(CpuState::RBX), CR4_PSE as u32);
}

/// `CR4.PVI` (bit 1) remains reserved — writing 1 → `#GP(0)`, CR4 unchanged.
#[test]
fn mov_cr4_pvi_still_reserved_raises_gp() {
    let mut bus = RamBus::new(0x10000);
    let mut cpu = real_mode_cpu(0x1000, 0xFFFE);
    install_ivt(&mut bus, &mut cpu, 13, 0x0000, 0x0900);

    // MOV EAX, 2 (PVI); MOV CR4, EAX
    bus.write_bytes(
        0x1000,
        &[0x66, 0xB8, 0x02, 0x00, 0x00, 0x00, 0x0F, 0x22, 0xE0, 0xF4],
    );
    bus.write_bytes(0x0900, &[0xF4]);

    step(&mut cpu, &mut bus).unwrap();
    step(&mut cpu, &mut bus).unwrap();
    assert_eq!(cpu.cr4, 0, "PVI must not stick");
    assert_eq!(cpu.rip, 0x0900, "#GP handler");
}
