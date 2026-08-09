//! "It spins" must become "it spins *here*" without a hand bisection.
//!
//! Round 3's probe reported `stop=step-budget-exhausted` and nothing else — no
//! `CS:IP`, no `EIP`, no indication of which instructions were executing. These
//! tests pin the replacement: a stop program counter for every non-failure
//! stop, plus a trailing PC histogram and tight-cycle detection.
//!
//! Spec: Intel SDM Vol. 1 §3.5 (`EIP`/`IP`); Vol. 3 §3.4.2 (linear address =
//! cached base + offset); Vol. 3 §3.4.5 (the `D` flag chooses the execution
//! window); Vol. 3 §9.1.4 (reset vector, `CS.base = 0xFFFF0000`).

use machine_pc::{Machine, PostSpinConfig, PostStopReason};

/// 64 KiB BIOS image whose reset vector far-jumps to `F000:0000`.
fn bios_image_64k(code: &[u8]) -> Vec<u8> {
    let mut rom = vec![0xF4u8; 64 * 1024];
    rom[..code.len()].copy_from_slice(code);
    rom[0xFFF0..0xFFF5].copy_from_slice(&[0xEA, 0x00, 0x00, 0x00, 0xF0]);
    rom
}

/// A one-instruction spin is the simplest thing the probe used to be unable to
/// describe. It should name the address and say the cycle is one long.
#[test]
fn a_one_instruction_spin_is_named_and_detected() {
    // EB FE = JMP $ at F000:0000, i.e. linear 0x000F0000.
    let mut m = Machine::with_bios_rom(1024 * 1024, &bios_image_64k(&[0xEB, 0xFE])).expect("BIOS");
    let report = m.probe_post(5_000);

    assert_eq!(report.stop, PostStopReason::StepBudgetExhausted);
    let site = report.stop_site;
    assert_eq!(site.linear_pc, 0x000F_0000, "{report}");
    assert_eq!(
        (site.cs, site.eip, site.cs_default_big),
        (0xF000, 0x0000, false)
    );

    let spin = report.spin.as_ref().expect("spin summary armed by default");
    assert_eq!(spin.distinct, 1, "{report}");
    let cycle = spin.cycle.as_ref().expect("a self-jump is a cycle");
    assert_eq!(cycle.period, 1);
    assert_eq!(cycle.sites[0].linear_pc, 0x000F_0000);
    assert_eq!(spin.hot[0].0.linear_pc, 0x000F_0000);
    assert_eq!(spin.hot[0].1, spin.sampled);

    // And the reader sees it without parsing a struct, including the bytes
    // that make `EB FE` recognisable as a self-jump.
    let text = report.to_string();
    assert!(text.contains("stop-pc"), "{text}");
    assert!(text.contains("linear_pc=0x00000000000F0000"), "{text}");
    assert!(text.contains("bytes=[EB FE"), "{text}");
    assert!(text.contains("cycle=1"), "{text}");
}

/// A multi-instruction loop reports its period and every site in it, in
/// execution order — the shape needed to recognise a firmware poll loop.
#[test]
fn a_multi_instruction_loop_reports_its_period_and_members() {
    #[rustfmt::skip]
    let code: &[u8] = &[
        0x90,             // NOP        @ 0x000F0000
        0x90,             // NOP        @ 0x000F0001
        0xEB, 0xFC,       // JMP -4     @ 0x000F0002
    ];
    let mut m = Machine::with_bios_rom(1024 * 1024, &bios_image_64k(code)).expect("BIOS");
    let report = m.probe_post(5_000);

    assert_eq!(report.stop, PostStopReason::StepBudgetExhausted);
    let spin = report.spin.as_ref().expect("armed");
    assert_eq!(spin.distinct, 3, "{report}");
    let cycle = spin.cycle.as_ref().expect("three-instruction cycle");
    assert_eq!(cycle.period, 3, "{report}");
    // The revolution ends with the last retired instruction, so it begins with
    // the instruction that would run next — the same site the header names.
    assert_eq!(
        cycle.sites[0].linear_pc, report.stop_site.linear_pc,
        "{report}"
    );
    let mut members: Vec<u64> = cycle.sites.iter().map(|s| s.linear_pc).collect();
    members.sort_unstable();
    assert_eq!(
        members,
        vec![0x000F_0000, 0x000F_0001, 0x000F_0002],
        "{report}"
    );
    assert!(cycle.repeats > 1, "{report}");
}

/// Code that makes progress is not reported as a cycle. Otherwise the summary
/// would cry wolf on every long-running run.
#[test]
fn straight_line_progress_is_not_reported_as_a_cycle() {
    // A 4 KiB run of NOPs then HLT, entered at F000:0000; the sampler window is
    // larger than the loop-free stretch that precedes the halt.
    let mut code = vec![0x90u8; 3000];
    code.push(0xF4);
    let mut m = Machine::with_bios_rom(1024 * 1024, &bios_image_64k(&code)).expect("BIOS");
    let report = m.probe_post(5_000);

    assert_eq!(report.stop, PostStopReason::Halted);
    let spin = report.spin.as_ref().expect("armed");
    assert!(spin.cycle.is_none(), "{report}");
    assert!(spin.distinct > 1000, "{report}");
    // Nothing repeated, so the histogram says so by being empty rather than
    // listing four addresses with `count=1`.
    assert!(spin.hot.is_empty(), "{report}");
    // The halt site is reported even though nothing spun. It is the resume
    // point, one byte past the single-byte `HLT` at offset 3000.
    assert_eq!(report.stop_site.linear_pc, 0x000F_0000 + 3001, "{report}");
}

/// The probe stays cheap and quiet when a caller does not want the summary.
#[test]
fn the_spin_summary_can_be_turned_off() {
    let mut m = Machine::with_bios_rom(1024 * 1024, &bios_image_64k(&[0xEB, 0xFE])).expect("BIOS");
    let traced = m.probe_post_options(2_000, None, None);
    assert!(traced.report.spin.is_none());
    let text = traced.report.to_string();
    assert!(!text.contains("spin"), "{text}");
    // The stop PC does not depend on the sampler.
    assert!(text.contains("stop-pc"), "{text}");
    assert_eq!(traced.report.stop_site.linear_pc, 0x000F_0000);

    // A zero-length window is the same as off.
    let mut m = Machine::with_bios_rom(1024 * 1024, &bios_image_64k(&[0xEB, 0xFE])).expect("BIOS");
    let traced = m.probe_post_options(2_000, None, Some(PostSpinConfig::with_window(0)));
    assert!(traced.report.spin.is_none());
}

/// The contract every other agent depends on: the single-line header does not
/// move, and a run that stops on a failure prints exactly what it printed
/// before — the new lines appear only for the stops that reported nothing.
#[test]
fn the_post_probe_header_is_unchanged_and_failures_gain_no_lines() {
    // 0F FF is not a valid opcode: the probe stops on it.
    let mut m = Machine::with_bios_rom(1024 * 1024, &bios_image_64k(&[0x0F, 0xFF])).expect("BIOS");
    let report = m.probe_post(1_000);
    assert!(report.failure().is_some(), "{report}");
    let text = report.to_string();
    assert!(
        !text.contains("stop-pc"),
        "the failure already names its site: {text}"
    );
    assert!(!text.contains("spin"), "{text}");

    // Header shape, for the stop that does gain lines.
    let mut m = Machine::with_bios_rom(1024 * 1024, &bios_image_64k(&[0xEB, 0xFE])).expect("BIOS");
    let report = m.probe_post(1_000);
    let mut lines = report.to_string();
    lines.truncate(lines.find('\n').expect("multi-line report"));
    assert_eq!(lines, "post-probe: steps=1000 stop=step-budget-exhausted");
}
