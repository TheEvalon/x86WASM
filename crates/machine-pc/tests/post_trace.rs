//! The POST probe's bounded event trace.
//!
//! `Machine::probe_post` reports where firmware stopped. `probe_post_traced`
//! additionally records what it was doing on the way there: port I/O, PCI
//! configuration cycles, PAM programming, VGA aperture accesses, and memory
//! faults. These tests pin both the contents and the one property every
//! consumer depends on — the existing single-line report format is untouched.
//!
//! Spec: PCI Local Bus Specification Revision 3.0 §3.2.2.3.2 (Mechanism #1);
//! Intel 440FX 82441FX (PMC) §3.2.18 (PAM); IBM PS/2 Video Subsystems (the
//! `0xA0000`-`0xBFFFF` display aperture); Intel SDM Vol. 3 §9.1.4 (reset
//! vector).

use machine_pc::{Machine, PostTrace, PostTraceConfig, PostTraceEvent};

/// 64 KiB BIOS image whose reset vector far-jumps to `F000:0000`.
fn bios_image_64k(code: &[u8]) -> Vec<u8> {
    let mut rom = vec![0xF4u8; 64 * 1024];
    rom[..code.len()].copy_from_slice(code);
    rom[0xFFF0..0xFFF5].copy_from_slice(&[0xEA, 0x00, 0x00, 0x00, 0xF0]);
    rom
}

/// Four 8-bit stores that assemble CONFIG_ADDRESS for host-bridge `00:00.0`.
///
/// Real hardware ignores non-dword writes to `0xCF8`-`0xCFB` (PCI 3.0
/// §3.2.2.3.2); the caller arms the documented compatibility policy because
/// this build's decoder has no `EF` (`OUT DX, eAX`) form yet.
#[rustfmt::skip]
fn config_address_host_bridge(reg: u8) -> Vec<u8> {
    vec![
        0xBA, 0xF8, 0x0C, 0xB0, reg & !0x03, 0xEE,  // 0xCF8 <- register
        0xBA, 0xF9, 0x0C, 0xB0, 0x00, 0xEE,         // 0xCF9 <- bus 0
        0xBA, 0xFA, 0x0C, 0xB0, 0x00, 0xEE,         // 0xCFA <- device/function 0
        0xBA, 0xFB, 0x0C, 0xB0, 0x80, 0xEE,         // 0xCFB <- enable
    ]
}

fn traced_run(code: &[u8], capacity: usize) -> (Machine, PostTrace) {
    let mut m = Machine::with_bios_rom(1024 * 1024, &bios_image_64k(code)).expect("map BIOS");
    m.pci.set_config_address_byte_lane_compat(true);
    let traced = m.probe_post_traced(10_000, Some(PostTraceConfig::with_capacity(capacity)));
    assert!(
        m.cpu.halted,
        "guest reached HLT; stopped instead with {}",
        traced.report
    );
    let trace = traced.trace.expect("tracing was armed");
    (m, trace)
}

/// The whole point of the harness: a reader sees the platform sequence, not
/// just the last instruction.
#[test]
fn a_traced_run_records_ports_config_cycles_pam_and_the_vga_aperture() {
    #[rustfmt::skip]
    let tail: &[u8] = &[
        // PAM0 high nibble WE only: writes to the BIOS area go to DRAM while
        // reads still come from ROM, so the guest keeps executing this ROM.
        0xBA, 0xFD, 0x0C, 0xB0, 0x20, 0xEE, // OUT 0xCFD, AL  — PAM0 = WE
        0xBA, 0xFD, 0x0C, 0xEC,             // IN  AL, 0xCFD  — read it back
        0xB0, 0x12, 0xE6, 0x80,             // OUT 0x80, AL   — POST checkpoint
        0xB8, 0x00, 0xB8, 0x8E, 0xC0,       // MOV ES, 0xB800
        0x31, 0xFF,                         // XOR DI, DI
        0xB0, b'T', 0x26, 0x88, 0x05,       // MOV ES:[DI], AL — text aperture
        0xF4,                               // HLT
    ];
    let mut code = config_address_host_bridge(0x58);
    code.extend_from_slice(tail);

    let (_, trace) = traced_run(&code, 256);

    assert_eq!(trace.dropped(), 0, "256 events is more than this run needs");

    // CONFIG_ADDRESS accesses carry the latch they produced.
    let latched: Vec<u32> = trace
        .events()
        .filter_map(|(_, e)| match e {
            PostTraceEvent::PciConfigAddress { latched, .. } => Some(latched),
            _ => None,
        })
        .collect();
    assert_eq!(latched.len(), 4, "four byte-lane stores");
    assert_eq!(*latched.last().unwrap(), 0x8000_0058, "assembled latch");

    // CONFIG_DATA accesses carry the decoded target, both directions.
    let data: Vec<PostTraceEvent> = trace
        .events()
        .map(|(_, e)| e)
        .filter(|e| matches!(e, PostTraceEvent::PciConfigData { .. }))
        .collect();
    assert_eq!(data.len(), 2, "one write and one read at 0xCFD");
    for event in &data {
        let PostTraceEvent::PciConfigData {
            port,
            register,
            bus,
            device,
            function,
            enabled,
            ..
        } = event
        else {
            unreachable!()
        };
        assert_eq!(*port, 0x0CFD);
        assert_eq!((*bus, *device, *function, *register), (0, 0, 0, 0x58));
        assert!(*enabled);
    }

    // The configuration write re-attributed a PAM segment, and the trace names
    // which register moved rather than leaving the reader to infer it.
    assert_eq!(
        trace.count_matching(|e| matches!(
            e,
            PostTraceEvent::PamProgram {
                index: 0,
                value: 0x20
            }
        )),
        1
    );

    // Ordinary port I/O and the VGA aperture write are both present.
    assert_eq!(
        trace.count_matching(|e| matches!(
            e,
            PostTraceEvent::PortOut {
                port: 0x0080,
                value: 0x12,
                ..
            }
        )),
        1
    );
    assert_eq!(
        trace.count_matching(|e| matches!(
            e,
            PostTraceEvent::VgaAperture {
                write: true,
                addr: 0x000B_8000,
                value: b'T'
            }
        )),
        1
    );
}

/// The trace is a window on the *end* of a run: older events are dropped and
/// counted, so a long POST cannot exhaust memory and the reader still knows
/// what they are missing.
#[test]
fn the_trace_is_bounded_and_reports_what_it_dropped() {
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBA, 0x80, 0x00,       // MOV DX, 0x0080
        0xB9, 0x20, 0x00,       // MOV CX, 32
        0xB0, 0x00,             // MOV AL, 0
        // loop: OUT DX, AL ; INC AL ; LOOP
        0xEE, 0xFE, 0xC0, 0xE2, 0xFB,
        0xF4,                   // HLT
    ];

    let (_, trace) = traced_run(code, 4);

    assert_eq!(trace.capacity(), 4);
    assert_eq!(trace.len(), 4);
    assert_eq!(trace.total(), 32);
    assert_eq!(trace.dropped(), 28);

    // The retained window is the newest four, with their original sequence
    // numbers so the gap is visible.
    let kept: Vec<(u64, PostTraceEvent)> = trace.events().collect();
    assert_eq!(kept[0].0, 28);
    assert_eq!(
        kept[3].1,
        PostTraceEvent::PortOut {
            port: 0x0080,
            size: 1,
            value: 0x1F
        }
    );
}

/// A run that dies on a memory fault records the fault, so the trace explains
/// the stop instead of ending one access early.
///
/// The fault used here is a guest write into the top-of-4 GiB ROM window, which
/// is outside the `0xC0000`-`0xFFFFF` PAM range and therefore has no "forward
/// it to PCI" behavior to fall back on. Reads never fault in this machine
/// (unmapped physical space is open bus), so this is the reachable case.
///
/// Spec: Intel SDM Vol. 3 §9.1.4 — after reset `CS.base = 0xFFFF0000`, which a
/// near jump preserves; Intel 440FX §3.2.18 for the PAM window this is outside.
#[test]
fn a_memory_fault_is_recorded_next_to_the_accesses_that_led_to_it() {
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB0, 0xAA, 0xE6, 0x80, // MOV AL, 0xAA ; OUT 0x80, AL
        0x31, 0xFF,             // XOR DI, DI
        0x2E, 0x88, 0x05,       // MOV CS:[DI], AL — writes ROM at 0xFFFF0000
        0xF4,
    ];
    // High-map only: no below-1 MiB alias, so `CS.base` stays 0xFFFF0000.
    let mut rom = vec![0xF4u8; 64 * 1024];
    rom[..code.len()].copy_from_slice(code);
    // Reset vector at 0xFFF0: near jump back to offset 0, keeping CS.base.
    rom[0xFFF0..0xFFF3].copy_from_slice(&[0xE9, 0x0D, 0x00]);
    let mut m = Machine::new(64 * 1024);
    m.load_rom(&rom).expect("map lab ROM");
    let traced = m.probe_post_traced(1_000, Some(PostTraceConfig::default()));

    let trace = traced.trace.as_ref().expect("tracing was armed");
    assert_eq!(
        trace.count_matching(|e| matches!(
            e,
            PostTraceEvent::MemoryFault {
                write: true,
                addr: 0xFFFF_0000
            }
        )),
        1,
        "the faulting access is in the trace: {trace}"
    );
    assert!(
        trace.count_matching(|e| matches!(e, PostTraceEvent::PortOut { port: 0x0080, .. })) == 1,
        "and so is what came before it"
    );
}

/// The contract every other agent and the coordinator depend on: `--post-probe`
/// output does not move. A traced report starts with exactly the untraced
/// report's text, and an untraced probe records nothing at all.
#[test]
fn the_existing_post_probe_output_is_unchanged_by_tracing() {
    #[rustfmt::skip]
    let code: &[u8] = &[0xB0, 0x55, 0xE6, 0x80, 0xF4];
    let rom = bios_image_64k(code);

    let mut plain = Machine::with_bios_rom(256 * 1024, &rom).expect("map BIOS");
    let plain_report = plain.probe_post(1_000);

    let mut traced_machine = Machine::with_bios_rom(256 * 1024, &rom).expect("map BIOS");
    let traced = traced_machine.probe_post_traced(1_000, Some(PostTraceConfig::default()));

    assert_eq!(traced.report, plain_report, "the report itself is the same");
    let combined = traced.to_string();
    let plain_text = plain_report.to_string();
    assert!(
        combined.starts_with(&plain_text),
        "traced output must begin with the untouched report:\n{combined}"
    );
    assert_eq!(
        combined[plain_text.len()..].lines().next(),
        Some(""),
        "the trace starts on its own line"
    );
    assert!(combined[plain_text.len()..].contains("post-trace: events="));

    // Untraced runs stay untraced.
    let untraced = traced_machine.probe_post_traced(1_000, None);
    assert!(untraced.trace.is_none());
    assert_eq!(untraced.to_string(), untraced.report.to_string());
}
