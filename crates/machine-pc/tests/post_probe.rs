//! BIOS POST first-contact harness tests.
//!
//! The harness answers "how far does a BIOS image get, and what stopped it"
//! as a structured, assertable report. It is a diagnostic, not a claim that
//! POST succeeds.

use machine_pc::{
    seabios_image_path, Machine, PostFailureKind, PostReport, PostStopReason,
    DEFAULT_POST_PROBE_STEPS,
};

/// Build a 64 KiB BIOS image whose reset vector jumps to image offset 0.
///
/// Spec: Intel SDM Vol. 3 §9.1.4 — the first instruction is fetched from
/// `0xFFFFFFF0` with `CS.base = 0xFFFF0000`, so a near `JMP rel16` to `IP=0`
/// lands at the start of a 64 KiB image mapped at `0xFFFF0000`. The rest of the
/// image is `HLT` so a runaway fetch stops instead of wandering.
fn bios_rom_with_code(code: &[u8]) -> Vec<u8> {
    let mut rom = vec![0xF4u8; 64 * 1024];
    rom[..code.len()].copy_from_slice(code);
    // JMP rel16 from IP=0xFFF3 back to IP=0x0000 → rel = 0x000D.
    rom[0xFFF0..0xFFF3].copy_from_slice(&[0xE9, 0x0D, 0x00]);
    rom
}

#[test]
fn probe_reports_halt_and_console_output() {
    // MOV AL,'A' / MOV DX,0x402 / OUT DX,AL (debug console) / HLT.
    let rom = bios_rom_with_code(&[0xB0, b'A', 0xBA, 0x02, 0x04, 0xEE, 0xF4]);
    let mut m = Machine::with_bios_rom(1024 * 1024, &rom).expect("load BIOS");

    let report = m.probe_post(1_000);

    assert_eq!(report.stop, PostStopReason::Halted, "{report}");
    assert!(report.steps >= 4, "{report}");
    assert_eq!(report.debug, "A");
    assert!(report.failure().is_none());
}

/// The first failure is captured precisely: kind, `CS:IP`, RIP, linear PC, and
/// an eight-byte wrapping opcode window.
#[test]
fn probe_reports_first_unsupported_opcode_with_opcode_window() {
    // NOP, WAIT (valid but unimplemented), then recognisable filler.
    let rom = bios_rom_with_code(&[0x90, 0x9B, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77]);
    let mut m = Machine::with_bios_rom(1024 * 1024, &rom).expect("load BIOS");

    let report = m.probe_post(64);

    let failure = report.failure().expect("first failure recorded");
    assert_eq!(failure.kind, PostFailureKind::UnsupportedOpcode(0x9B));
    assert_eq!(failure.cs, 0xF000);
    assert_eq!(failure.ip, 0x0001);
    assert_eq!(failure.linear_pc, 0xFFFF_0001);
    assert_eq!(
        failure.opcode_bytes,
        [
            Some(0x9B),
            Some(0x11),
            Some(0x22),
            Some(0x33),
            Some(0x44),
            Some(0x55),
            Some(0x66),
            Some(0x77),
        ]
    );
    // Reset-vector JMP plus the NOP retired before the unsupported opcode.
    assert_eq!(report.steps, 2);
}

/// A two-byte opcode is named in full, not by the second byte alone.
///
/// Spec: Intel SDM Vol. 2 §2.1.1 — an instruction is prefixes followed by a
/// one-, two-, or three-byte opcode; `0F 85` is the two-byte near `JNZ`, which
/// is where the pinned SeaBIOS image stops. The decoder reports only the byte
/// its tables missed, so the report reconstructs the escape from the window.
#[test]
fn probe_names_two_byte_opcode_with_its_escape() {
    // 0F 85 rel16 (near JNZ) — not in the decode tables.
    let rom = bios_rom_with_code(&[0x0F, 0x85, 0x8E, 0xF9]);
    let mut m = Machine::with_bios_rom(1024 * 1024, &rom).expect("load BIOS");

    let report = m.probe_post(16);

    let failure = report.failure().expect("first failure recorded");
    let site = failure.opcode_site().expect("opcode recovered from window");
    assert_eq!(site.opcode, vec![0x0F, 0x85]);
    assert!(site.prefixes.is_empty());
    let text = report.to_string();
    assert!(text.contains("unsupported opcode 0x0F 0x85"), "{text}");
    assert!(!text.contains("unsupported opcode 0x85 "), "{text}");
}

/// Prefixes are reported alongside the opcode instead of being swallowed.
#[test]
fn probe_names_prefixes_before_the_opcode() {
    // 66 0F 85 rel32 — operand-size prefixed near JNZ.
    let rom = bios_rom_with_code(&[0x66, 0x0F, 0x85, 0x00, 0x01, 0x00, 0x00]);
    let mut m = Machine::with_bios_rom(1024 * 1024, &rom).expect("load BIOS");

    let report = m.probe_post(16);

    let failure = report.failure().expect("first failure recorded");
    let site = failure.opcode_site().expect("opcode recovered from window");
    assert_eq!(site.prefixes, vec![0x66]);
    assert_eq!(site.opcode, vec![0x0F, 0x85]);
    let text = report.to_string();
    assert!(
        text.contains("unsupported opcode 0x0F 0x85 (prefixes 66)"),
        "{text}"
    );
}

/// Ports no device claims are recorded (port, direction, size, first value).
#[test]
fn probe_records_unclaimed_port_accesses() {
    // MOV DX,0x278 / MOV AL,0x5A / OUT DX,AL / IN AL,DX / HLT.
    let rom = bios_rom_with_code(&[0xBA, 0x78, 0x02, 0xB0, 0x5A, 0xEE, 0xEC, 0xEC, 0xF4]);
    let mut m = Machine::with_bios_rom(1024 * 1024, &rom).expect("load BIOS");

    let report = m.probe_post(64);

    assert_eq!(report.stop, PostStopReason::Halted, "{report}");
    let writes: Vec<_> = report
        .unclaimed_ports
        .iter()
        .filter(|a| a.port == 0x278 && a.write)
        .collect();
    assert_eq!(writes.len(), 1, "{report}");
    assert_eq!(writes[0].size, 1);
    assert_eq!(writes[0].first_value, 0x5A);
    assert_eq!(writes[0].count, 1);

    let reads: Vec<_> = report
        .unclaimed_ports
        .iter()
        .filter(|a| a.port == 0x278 && !a.write)
        .collect();
    assert_eq!(reads.len(), 1, "{report}");
    assert_eq!(reads[0].count, 2);
    assert!(!report.unclaimed_port_overflow);
}

/// Accesses outside RAM and every ROM window are recorded page-granular, so an
/// unimplemented MMIO region shows up instead of silently reading open bus.
#[test]
fn probe_records_unmapped_mmio_pages() {
    // MOV AL,[0x8000] / HLT — 0x8000 is past the 4 KiB of RAM and not ROM.
    let rom = bios_rom_with_code(&[0xA0, 0x00, 0x80, 0xF4]);
    let mut m = Machine::with_bios_rom(4096, &rom).expect("load BIOS");

    let report = m.probe_post(64);

    assert_eq!(report.stop, PostStopReason::Halted, "{report}");
    let hits: Vec<_> = report
        .unmapped_mmio
        .iter()
        .filter(|a| a.page == 0x8000 && !a.write)
        .collect();
    assert_eq!(hits.len(), 1, "{report}");
    assert_eq!(hits[0].count, 1);
}

/// A spinning BIOS stops on the step budget rather than hanging the harness.
#[test]
fn probe_stops_on_step_budget() {
    let rom = bios_rom_with_code(&[0xEB, 0xFE]); // JMP $
    let mut m = Machine::with_bios_rom(1024 * 1024, &rom).expect("load BIOS");

    let report = m.probe_post(32);

    assert_eq!(report.stop, PostStopReason::StepBudgetExhausted);
    assert_eq!(report.steps, 32);
    assert!(report.failure().is_none());
}

/// The report renders every field a bring-up session needs.
#[test]
fn probe_report_display_is_structured() {
    let rom = bios_rom_with_code(&[0x90, 0x9B]);
    let mut m = Machine::with_bios_rom(1024 * 1024, &rom).expect("load BIOS");

    let text = m.probe_post(16).to_string();

    assert!(text.contains("post-probe:"), "{text}");
    assert!(text.contains("steps=2"), "{text}");
    assert!(text.contains("unsupported opcode 0x9B"), "{text}");
    assert!(text.contains("cs:ip=F000:0001"), "{text}");
    assert!(text.contains("linear_pc=0x00000000FFFF0001"), "{text}");
    assert!(text.contains("opcode_bytes=[9B"), "{text}");
}

/// A POST-shaped prologue makes its progress visible through port `0x80`.
///
/// Spec: IBM PC/AT Technical Reference — POST writes checkpoint codes to the
/// manufacturing diagnostic port `0x80`; the AT POST sequence masks both 8259A
/// interrupt masks (OCW1 `0xFF`) and reads CMOS shutdown status `0x0F` with NMI
/// disabled (`0x70` bit 7).
#[test]
fn probe_captures_post_checkpoint_codes() {
    #[rustfmt::skip]
    let rom = bios_rom_with_code(&[
        0xFA,                   // CLI
        0xB0, 0x01, 0xE6, 0x80, // checkpoint 01
        0xB0, 0xFF,             // OCW1 mask-all
        0xE6, 0xA1,             // slave IMR
        0xE6, 0x21,             // master IMR
        0xB0, 0x02, 0xE6, 0x80, // checkpoint 02
        0xB0, 0x8F, 0xE6, 0x70, // CMOS index 0x0F, NMI disabled
        0xE4, 0x71,             // read shutdown status
        0xB0, 0x03, 0xE6, 0x80, // checkpoint 03
        0xF4,                   // HLT
    ]);
    let mut m = Machine::with_bios_rom(1024 * 1024, &rom).expect("load BIOS");

    let report = m.probe_post(64);

    assert_eq!(report.stop, PostStopReason::Halted, "{report}");
    assert_eq!(report.post_codes, vec![0x01, 0x02, 0x03], "{report}");
    assert_eq!(report.last_post_code, Some(0x03));
    assert!(!report.post_code_overflow);
    // Port 0x80 is claimed now, so it is no longer an unclaimed-port finding.
    assert!(
        !report.unclaimed_ports.iter().any(|a| a.port == 0x80),
        "{report}"
    );
    assert!(
        report.to_string().contains("post-codes=[01 02 03]"),
        "{report}"
    );
}

/// Real-firmware first contact. Skips when `firmware/seabios/bios.bin` is
/// absent (it is git-ignored and produced by
/// `firmware/build-scripts/build-seabios.sh`). Run with `--nocapture` to read
/// the report; `X86WASM_SEABIOS_BIOS` overrides the image path.
#[test]
fn seabios_post_probe_records_first_blocker() {
    let Some(path) = seabios_image_path() else {
        eprintln!(
            "skipping: no SeaBIOS image (build firmware/seabios/bios.bin or set \
             X86WASM_SEABIOS_BIOS)"
        );
        return;
    };
    let image = std::fs::read(&path).expect("read SeaBIOS image");
    let mut m = Machine::with_bios_rom(32 * 1024 * 1024, &image).expect("map SeaBIOS image");

    let report: PostReport = m.probe_post(DEFAULT_POST_PROBE_STEPS);

    eprintln!("SeaBIOS image: {}", path.display());
    eprintln!("{report}");
    // This is a diagnostic: POST is not expected to complete. Only require that
    // the probe made forward progress and produced a classified stop reason.
    assert!(report.steps > 0, "{report}");
    assert!(
        matches!(
            report.stop,
            PostStopReason::Halted
                | PostStopReason::StepBudgetExhausted
                | PostStopReason::Failure(_)
        ),
        "{report}"
    );
}
