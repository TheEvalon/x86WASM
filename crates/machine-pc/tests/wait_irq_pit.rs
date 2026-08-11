//! SeaBIOS-shaped `wait_irq` yields must wake on PIT IRQ0 more than once.
//!
//! SeaBIOS `wait_irq` (rel-1.16.3 `src/stacks.c`) is `sti; hlt; cli; cld; ret`.
//! Late POST and POST-with-media sampling at `F000:C897` lands on the `cli`
//! after HLT when the step budget ends mid-yield — so the IRQ0 path has to keep
//! delivering across successive yields (mode-2 rate generator + edge IR0).
//!
//! Spec: Intel SDM Vol. 2 HLT / Vol. 3A §6.8.1; Intel 8254 mode 2; Intel 8259A
//! edge-triggered IR0; `docs/post-c897-cf9-diagnosis.md`.

use machine_pc::{Machine, PostStopReason};

const HANDLER: u16 = 0x0050;
const CODE_WAKE1: u8 = 0xB1;
const CODE_WAKE2: u8 = 0xB2;
const CODE_DONE: u8 = 0xEE;

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
    // Shared handler: OUT 0x80 with DL, EOI, IRET. Caller sets DL before each yield.
    vec![
        0x88, 0xD0,       // MOV AL, DL
        0xE6, 0x80,       // OUT 0x80, AL
        0xB0, 0x20,       // MOV AL, 0x20
        0xE6, 0x20,       // OUT 0x20, AL — non-specific EOI
        0xCF,             // IRET
    ]
}

/// Two SeaBIOS-shaped yields (`sti; hlt; cli`) both wake on mode-2 IRQ0.
#[test]
fn seabios_shaped_wait_irq_yields_twice_on_pit_mode2() {
    let [handler_lo, handler_hi] = HANDLER.to_le_bytes();
    #[rustfmt::skip]
    let entry: Vec<u8> = vec![
        // IVT vector 8 := F000:HANDLER
        0xC7, 0x06, 0x20, 0x00, handler_lo, handler_hi,
        0xC7, 0x06, 0x22, 0x00, 0x00, 0xF0,
        // 8259A master cascade + ICW4, base 0x08; unmask IR0.
        0xB0, 0x11, 0xE6, 0x20,
        0xB0, 0x08, 0xE6, 0x21,
        0xB0, 0x04, 0xE6, 0x21,
        0xB0, 0x01, 0xE6, 0x21,
        0xB0, 0xFE, 0xE6, 0x21,
        // 8254 ch0 mode 2, LSB+MSB, count 32 — short model period.
        0xB0, 0x34, 0xE6, 0x43,
        0xB0, 0x20, 0xE6, 0x40,
        0xB0, 0x00, 0xE6, 0x40,
        // Yield 1 — SeaBIOS wait_irq shape (omit far ret; fall through).
        0xB2, CODE_WAKE1, // MOV DL, wake1
        0xFB,             // STI
        0xF4,             // HLT
        0xFA,             // CLI
        0xFC,             // CLD
        // Yield 2
        0xB2, CODE_WAKE2,
        0xFB,
        0xF4,
        0xFA,
        0xFC,
        0xB0, CODE_DONE, 0xE6, 0x80, // MOV AL,done / OUT 0x80,AL
        0xFA,
        0xF4, // permanent halt
    ];
    assert!(entry.len() <= usize::from(HANDLER));

    let mut m = Machine::with_bios_rom(1024 * 1024, &bios_image_64k(&entry, &irq0_handler()))
        .expect("map BIOS");
    let report = m.probe_post(50_000);

    assert_eq!(report.stop, PostStopReason::Halted, "{report}");
    // Mode-2 may deliver more than one IRQ0 between `STI` and `CLI` (IRET
    // restores IF=1; a pending edge can nest before `CLI`). Both yields must
    // still observe at least one wake, then reach DONE.
    assert!(
        report.post_codes.contains(&CODE_WAKE1),
        "first wait_irq yield must wake: {report}"
    );
    assert!(
        report.post_codes.contains(&CODE_WAKE2),
        "second wait_irq yield must wake: {report}"
    );
    assert_eq!(
        *report.post_codes.last().expect("codes"),
        CODE_DONE,
        "guest must finish after two yields: {report}"
    );
    assert!(report.idle_steps > 0, "yields must count as idle: {report}");
}
