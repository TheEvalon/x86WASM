//! A `HLT` waiting for a timer interrupt is an idle, not a stop.
//!
//! The POST probe ended a run the moment `CPU.halted` became true, before the
//! machine had a chance to deliver anything. Every firmware idle — `usleep`,
//! `yield`, "wait for the timer" — therefore looked identical to a permanent
//! hang, and the run stopped there. This is the end-to-end path that has to
//! work instead: guest programs the PIT and the 8259, enables interrupts,
//! halts, and is woken by IRQ0 through the real-mode IVT.
//!
//! Spec: Intel SDM Vol. 2 "HLT" — the halt state ends on an enabled interrupt,
//! NMI, SMI, debug exception, INIT or RESET; Vol. 3 §6.8.1 — a maskable
//! interrupt requires `IF = 1`. Intel 8254 datasheet — the counter is clocked
//! by the CLK input regardless of what the processor is doing, so the timer
//! keeps counting through a halt. Intel 8259A — ICW1–ICW4 then OCW1, master
//! IR0 carries PIT channel 0; the handler ends with a non-specific EOI.

use machine_pc::{Machine, PostStopReason};

/// Image offset of the IRQ0 handler inside the test BIOS.
const HANDLER: u16 = 0x0040;
/// Checkpoint the handler writes to the IBM PC/AT diagnostic port.
const CODE_IN_HANDLER: u8 = 0x5A;
/// Checkpoint written after the `HLT` resumes.
const CODE_AFTER_WAKE: u8 = 0xA5;

/// 64 KiB BIOS whose reset vector far-jumps to `F000:0000`, so `CS.base` is
/// `0xF0000` and a real-mode IVT entry of `F000:HANDLER` reaches this image.
///
/// Spec: Intel SDM Vol. 3 §9.1.4 (reset vector), §6.11 (real-address mode IDT
/// is the four-byte-per-vector IVT at linear 0).
fn bios_image_64k(entry: &[u8], handler: &[u8]) -> Vec<u8> {
    let mut rom = vec![0xF4u8; 64 * 1024];
    rom[..entry.len()].copy_from_slice(entry);
    let at = usize::from(HANDLER);
    rom[at..at + handler.len()].copy_from_slice(handler);
    rom[0xFFF0..0xFFF5].copy_from_slice(&[0xEA, 0x00, 0x00, 0x00, 0xF0]);
    rom
}

#[rustfmt::skip]
fn irq0_handler() -> Vec<u8> {
    vec![
        0xB0, CODE_IN_HANDLER, 0xE6, 0x80, // MOV AL,code / OUT 0x80,AL
        0xB0, 0x20, 0xE6, 0x20,            // MOV AL,0x20 / OUT 0x20,AL — EOI
        0xCF,                              // IRET
    ]
}

/// A halt with `IF = 1` is an idle: the probe keeps the machine running, the
/// PIT reaches its terminal count, IRQ0 is delivered through the IVT, and the
/// guest resumes after the `HLT`.
#[test]
fn a_halt_with_interrupts_enabled_is_woken_by_the_timer() {
    let [handler_lo, handler_hi] = HANDLER.to_le_bytes();
    #[rustfmt::skip]
    let entry: Vec<u8> = vec![
        // IVT vector 8 (offset 0x20) := F000:HANDLER. DS is 0 out of reset.
        0xC7, 0x06, 0x20, 0x00, handler_lo, handler_hi,
        0xC7, 0x06, 0x22, 0x00, 0x00, 0xF0,
        // 8259A master: ICW1 cascade+ICW4, ICW2 vector base 0x08, ICW3, ICW4.
        0xB0, 0x11, 0xE6, 0x20,
        0xB0, 0x08, 0xE6, 0x21,
        0xB0, 0x04, 0xE6, 0x21,
        0xB0, 0x01, 0xE6, 0x21,
        0xB0, 0xFE, 0xE6, 0x21,            // OCW1: unmask IR0 only
        // 8254 channel 0, LSB+MSB, mode 0, count 100 — one interrupt on
        // terminal count, so exactly one wake is expected.
        0xB0, 0x30, 0xE6, 0x43,
        0xB0, 0x64, 0xE6, 0x40,
        0xB0, 0x00, 0xE6, 0x40,
        0xFB,                              // STI
        0xF4,                              // HLT   — the idle under test
        0xB0, CODE_AFTER_WAKE, 0xE6, 0x80, // MOV AL,code / OUT 0x80,AL
        0xFA,                              // CLI
        0xF4,                              // HLT   — permanent, ends the probe
    ];
    assert!(
        entry.len() <= usize::from(HANDLER),
        "entry overruns handler"
    );

    let mut m = Machine::with_bios_rom(1024 * 1024, &bios_image_64k(&entry, &irq0_handler()))
        .expect("map BIOS");
    let report = m.probe_post(10_000);

    assert_eq!(report.stop, PostStopReason::Halted, "{report}");
    assert_eq!(
        report.post_codes,
        vec![CODE_IN_HANDLER, CODE_AFTER_WAKE],
        "IRQ0 must run the handler and the guest must resume after HLT: {report}"
    );
    assert!(
        report.idle_steps > 0,
        "the wait must be reported as idle quanta: {report}"
    );
    assert!(
        report.to_string().contains("halt-idle      idle-steps="),
        "{report}"
    );
    assert!(
        report.to_string().contains("busy-steps="),
        "idle accounting must name busy work: {report}"
    );
    assert!(
        report.to_string().contains("idle-pct="),
        "idle accounting must report the idle share of the budget: {report}"
    );
    // The idle is not counted as retired instructions, and not sampled into the
    // spin window either — the summary still shows the code that ran.
    let spin = report.spin.as_ref().expect("armed");
    assert!(spin.cycle.is_none(), "{report}");
}

/// `CLI; HLT` is a permanent hang and still ends the probe on the spot, with no
/// idle quanta and no budget burned.
///
/// Spec: Intel SDM Vol. 3 §6.8.1 — with `IF = 0` a maskable interrupt cannot
/// resume the processor, and this machine has no autonomous NMI source.
#[test]
fn a_halt_with_interrupts_disabled_still_stops_immediately() {
    #[rustfmt::skip]
    let entry: Vec<u8> = vec![
        0xB0, 0x11, 0xE6, 0x80, // MOV AL,0x11 / OUT 0x80,AL
        0xFA,                   // CLI
        0xF4,                   // HLT
    ];
    let mut m =
        Machine::with_bios_rom(1024 * 1024, &bios_image_64k(&entry, &[0xCF])).expect("map BIOS");
    let report = m.probe_post(100_000);

    assert_eq!(report.stop, PostStopReason::Halted, "{report}");
    assert_eq!(report.idle_steps, 0, "{report}");
    // Reset-vector far jump, MOV, OUT, CLI, HLT.
    assert_eq!(report.steps, 5, "{report}");
    assert!(!report.to_string().contains("halt-idle"), "{report}");
    // A permanent hang is self-describing: the window backs up onto the `HLT`
    // that stopped the machine rather than starting at the resume point.
    assert_eq!(report.stop_bytes[0], Some(0xF4), "{report}");
    assert!(report.to_string().contains("bytes=[F4"), "{report}");
}

/// An idle nobody can end is bounded by the step budget rather than hanging the
/// host: `IF = 1` with IR0 masked never receives a wake.
#[test]
fn an_idle_that_never_wakes_is_bounded_by_the_step_budget() {
    #[rustfmt::skip]
    let entry: Vec<u8> = vec![
        0xB0, 0xFF, 0xE6, 0x21, // MOV AL,0xFF / OUT 0x21,AL — mask every IR
        0xFB,                   // STI
        0xF4,                   // HLT
    ];
    let mut m =
        Machine::with_bios_rom(1024 * 1024, &bios_image_64k(&entry, &[0xCF])).expect("map BIOS");
    let report = m.probe_post(2_000);

    assert_eq!(report.stop, PostStopReason::StepBudgetExhausted, "{report}");
    assert_eq!(report.steps + report.idle_steps, 2_000, "{report}");
    assert!(report.idle_steps > 1_900, "{report}");
}
