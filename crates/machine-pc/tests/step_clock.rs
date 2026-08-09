//! Instruction-count time source during stepping and POST probing.
//!
//! Spec: Intel 8254 datasheet (CLK-driven counter; IBM PC/AT input clock
//! 1.193182 MHz) and Motorola MC146818A (Status A RS periodic quantum, once-a-
//! second update cycle). The step-to-tick ratio is a model choice recorded in
//! `docs/machine-r2-pam-memory.md`, not accurate timing.

use machine_pc::{
    Machine, PostStopReason, StepClock, CMOS_PERIODIC_HZ, PIT_CLOCKS_PER_CMOS_PERIOD,
    PIT_CLOCKS_PER_SECOND,
};

/// MC146818 Status C (`0x0C`) and its update-ended flag (bit 4).
const CMOS_STATUS_C: usize = 0x0C;
const CMOS_STATUS_C_UF: u8 = 1 << 4;
/// MC146818 seconds register (`0x00`), BCD by default.
const CMOS_SECONDS: usize = 0x00;

/// 64 KiB BIOS image whose reset vector jumps to image offset 0.
fn bios_rom_with_code(code: &[u8]) -> Vec<u8> {
    let mut rom = vec![0xF4u8; 64 * 1024];
    rom[..code.len()].copy_from_slice(code);
    rom[0xFFF0..0xFFF3].copy_from_slice(&[0xE9, 0x0D, 0x00]);
    rom
}

/// A ROM that spins forever, so a run is bounded only by the step budget.
fn spinning_machine() -> Machine {
    let rom = bios_rom_with_code(&[0xEB, 0xFE]); // JMP $
    Machine::with_bios_rom(1024 * 1024, &rom).expect("map BIOS image")
}

/// Default machine: stepping advances no device time, so every existing host
/// that ticks by hand keeps its exact behavior.
#[test]
fn stepping_does_not_advance_time_by_default() {
    let mut m = spinning_machine();
    assert!(!m.step_clock().enabled);

    m.run(5_000).expect("spin");

    assert_eq!(m.cmos.ram[CMOS_STATUS_C] & CMOS_STATUS_C_UF, 0);
    assert_eq!(m.cmos.ram[CMOS_SECONDS], 0);
    assert!(!m.pit.out_ch0());
}

/// Armed: PIT input clocks accumulate into the RTC periodic quantum.
#[test]
fn armed_step_clock_drives_the_rtc_periodic_quantum() {
    let mut m = spinning_machine();
    m.set_step_clock(StepClock::enabled_default());

    m.run(PIT_CLOCKS_PER_CMOS_PERIOD - 1).expect("spin");
    assert_eq!(
        m.cmos.ram[CMOS_STATUS_C] & CMOS_STATUS_C_UF,
        0,
        "no quantum yet"
    );

    m.run(1).expect("spin");
    assert_ne!(
        m.cmos.ram[CMOS_STATUS_C] & CMOS_STATUS_C_UF,
        0,
        "one periodic quantum after {PIT_CLOCKS_PER_CMOS_PERIOD} clocks"
    );
}

/// The ratio is configurable: one emulated second per instruction makes the
/// once-a-second update cycle observable in three steps.
#[test]
fn configurable_ratio_drives_the_rtc_update_cycle() {
    let mut m = spinning_machine();
    m.set_step_clock(StepClock::with_pit_clocks_per_step(PIT_CLOCKS_PER_SECOND));

    m.run(3).expect("spin");

    // BCD seconds (Status B DM clear at reset).
    assert_eq!(m.cmos.ram[CMOS_SECONDS], 0x03);
}

/// PIT channel 0 counts down from guest programming and raises IRQ0.
///
/// Spec: Intel 8254 mode 2 rate generator; Intel 8259A master IR0.
#[test]
fn armed_step_clock_drives_pit_channel0_to_irq0() {
    #[rustfmt::skip]
    let rom = bios_rom_with_code(&[
        0xB0, 0x34, 0xE6, 0x43, // MOV AL,0x34 / OUT 0x43,AL  (ch0, LSB+MSB, mode 2)
        0xB0, 0x64, 0xE6, 0x40, // count LSB 0x64
        0xB0, 0x00, 0xE6, 0x40, // count MSB 0x00  -> 100 clocks
        0xEB, 0xFE,             // JMP $
    ]);
    let mut m = Machine::with_bios_rom(1024 * 1024, &rom).expect("map BIOS image");
    m.set_step_clock(StepClock::enabled_default());

    m.run(400).expect("spin");

    let snapshot = m.pic.irr_isr_snapshot();
    assert_ne!(snapshot.master_irr & 0x01, 0, "IRQ0 latched from PIT ch0");
}

/// The POST probe arms the default clock for the run and restores the host
/// configuration afterwards, so a firmware delay loop can make progress.
#[test]
fn post_probe_arms_the_default_clock_and_restores_it() {
    let mut m = spinning_machine();
    assert!(!m.step_clock().enabled);

    let report = m.probe_post(PIT_CLOCKS_PER_CMOS_PERIOD * 2);

    assert_eq!(report.stop, PostStopReason::StepBudgetExhausted);
    assert_ne!(
        m.cmos.ram[CMOS_STATUS_C] & CMOS_STATUS_C_UF,
        0,
        "RTC advanced during the probe"
    );
    assert!(!m.step_clock().enabled, "host configuration restored");
}

/// A host-configured clock is used as-is by the probe.
#[test]
fn post_probe_keeps_a_host_configured_clock() {
    let mut m = spinning_machine();
    m.set_step_clock(StepClock::with_pit_clocks_per_step(PIT_CLOCKS_PER_SECOND));

    m.probe_post(2);

    assert_eq!(m.cmos.ram[CMOS_SECONDS], 0x02);
    assert_eq!(
        m.step_clock().pit_clocks_per_step,
        PIT_CLOCKS_PER_SECOND,
        "probe did not replace the host clock"
    );
    assert_eq!(CMOS_PERIODIC_HZ, 1024);
}

/// Reset drops partial quanta but keeps the configuration, matching the way
/// host-configured fw_cfg state survives `Machine::reset`.
#[test]
fn reset_keeps_the_configuration_and_drops_partial_quanta() {
    let mut m = spinning_machine();
    m.set_step_clock(StepClock::with_pit_clocks_per_step(
        PIT_CLOCKS_PER_SECOND / 2,
    ));

    m.run(1).expect("spin");
    m.reset();
    assert!(m.step_clock().enabled);

    m.run(1).expect("spin");
    assert_eq!(
        m.cmos.ram[CMOS_SECONDS], 0,
        "the pre-reset half second did not carry over"
    );
    m.run(1).expect("spin");
    assert_eq!(m.cmos.ram[CMOS_SECONDS], 0x01);
}
