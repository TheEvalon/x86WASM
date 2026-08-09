//! Deterministic instruction-count time source for the PIT and the RTC.
//!
//! Nothing in this machine advanced a timer unless the host called
//! [`crate::Machine::tick_pit`] / [`crate::Machine::tick_cmos`] by hand, so
//! firmware that waits on the PIT or the RTC spun until its step budget ran
//! out. This clock ties device time to **retired instructions** instead of to
//! wall clock, which keeps a run reproducible.
//!
//! The ratio is a **model choice, not accurate timing**: charging one PIT input
//! clock to each retired instruction implies a machine that retires 1.193182
//! million instructions per emulated second. Firmware that measures the CPU
//! against the PIT will compute a nonsense frequency. See
//! `docs/machine-r2-pam-memory.md`.
//!
//! Spec: Intel 8254 datasheet (the counter is clocked by the external CLK
//! input); IBM PC/AT — that input is 14.31818 MHz / 12; Motorola MC146818A —
//! the periodic rate comes from Status A RS (POST default `0110b` = 1024 Hz)
//! and the update cycle runs once per second.

/// PIT input clocks in one emulated second (IBM PC/AT 1.193182 MHz).
pub const PIT_CLOCKS_PER_SECOND: u64 = 1_193_182;

/// Periodic-interrupt rate the model uses for the RTC quantum (POST default).
pub const CMOS_PERIODIC_HZ: u64 = 1024;

/// PIT input clocks per modeled RTC periodic quantum.
pub const PIT_CLOCKS_PER_CMOS_PERIOD: u64 = PIT_CLOCKS_PER_SECOND / CMOS_PERIODIC_HZ;

/// PIT input clocks charged to one retired instruction by default.
pub const DEFAULT_PIT_CLOCKS_PER_STEP: u64 = 1;

/// Device ticks owed after one retired instruction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StepTicks {
    /// 8254 input clocks.
    pub pit_clocks: u64,
    /// MC146818 periodic quanta.
    pub cmos_periods: u64,
    /// MC146818 one-second update cycles.
    pub cmos_seconds: u64,
}

impl StepTicks {
    pub fn is_empty(&self) -> bool {
        self.pit_clocks == 0 && self.cmos_periods == 0 && self.cmos_seconds == 0
    }
}

/// Instruction-count-driven time source configuration and accumulators.
///
/// Disabled by default: [`crate::Machine::step`] behaves exactly as it did
/// before this existed, so hosts and tests that tick devices by hand are
/// unaffected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StepClock {
    /// Whether a retired instruction advances any device time.
    pub enabled: bool,
    /// PIT input clocks charged to each retired instruction.
    pub pit_clocks_per_step: u64,
    /// Whether accumulated clocks also drive the RTC periodic quantum.
    pub cmos_periodic: bool,
    /// Whether accumulated clocks also drive the RTC one-second update cycle.
    pub cmos_seconds: bool,
    period_accumulator: u64,
    second_accumulator: u64,
}

impl Default for StepClock {
    fn default() -> Self {
        Self::disabled()
    }
}

impl StepClock {
    /// No device time advances on a step (the machine's power-on state).
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            pit_clocks_per_step: DEFAULT_PIT_CLOCKS_PER_STEP,
            cmos_periodic: true,
            cmos_seconds: true,
            period_accumulator: 0,
            second_accumulator: 0,
        }
    }

    /// One PIT input clock per retired instruction, RTC derived from that.
    pub fn enabled_default() -> Self {
        Self {
            enabled: true,
            ..Self::disabled()
        }
    }

    /// [`Self::enabled_default`] with an explicit clocks-per-instruction ratio.
    ///
    /// A ratio of 0 disables every derived tick while leaving the clock armed.
    pub fn with_pit_clocks_per_step(pit_clocks_per_step: u64) -> Self {
        Self {
            enabled: true,
            pit_clocks_per_step,
            ..Self::disabled()
        }
    }

    /// Drop partial quanta without changing the configuration (used by reset).
    pub fn reset_accumulators(&mut self) {
        self.period_accumulator = 0;
        self.second_accumulator = 0;
    }

    /// Charge one retired instruction and return the device ticks it owes.
    pub fn charge_step(&mut self) -> StepTicks {
        if !self.enabled || self.pit_clocks_per_step == 0 {
            return StepTicks::default();
        }
        let pit_clocks = self.pit_clocks_per_step;

        let cmos_periods = if self.cmos_periodic {
            self.period_accumulator += pit_clocks;
            let periods = self.period_accumulator / PIT_CLOCKS_PER_CMOS_PERIOD;
            self.period_accumulator -= periods * PIT_CLOCKS_PER_CMOS_PERIOD;
            periods
        } else {
            0
        };

        let cmos_seconds = if self.cmos_seconds {
            self.second_accumulator += pit_clocks;
            let seconds = self.second_accumulator / PIT_CLOCKS_PER_SECOND;
            self.second_accumulator -= seconds * PIT_CLOCKS_PER_SECOND;
            seconds
        } else {
            0
        };

        StepTicks {
            pit_clocks,
            cmos_periods,
            cmos_seconds,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_clock_owes_nothing() {
        let mut clock = StepClock::disabled();
        for _ in 0..1000 {
            assert!(clock.charge_step().is_empty());
        }
    }

    /// Spec: IBM PC/AT 8254 input clock 1.193182 MHz; MC146818 1024 Hz POST
    /// periodic default. One clock per instruction accumulates both quanta.
    #[test]
    fn default_ratio_accumulates_rtc_quanta() {
        let mut clock = StepClock::enabled_default();
        let mut periods = 0;
        let mut seconds = 0;
        for _ in 0..PIT_CLOCKS_PER_CMOS_PERIOD {
            let ticks = clock.charge_step();
            assert_eq!(ticks.pit_clocks, 1);
            periods += ticks.cmos_periods;
            seconds += ticks.cmos_seconds;
        }
        assert_eq!(
            periods, 1,
            "one periodic quantum per {PIT_CLOCKS_PER_CMOS_PERIOD} clocks"
        );
        assert_eq!(seconds, 0);
    }

    #[test]
    fn a_second_of_clocks_runs_one_update_cycle() {
        let mut clock = StepClock::with_pit_clocks_per_step(PIT_CLOCKS_PER_SECOND);
        let ticks = clock.charge_step();
        assert_eq!(ticks.pit_clocks, PIT_CLOCKS_PER_SECOND);
        assert_eq!(ticks.cmos_seconds, 1);
        assert_eq!(ticks.cmos_periods, CMOS_PERIODIC_HZ);
    }

    /// Partial quanta carry across steps rather than being rounded away.
    #[test]
    fn remainders_carry_between_steps() {
        let mut clock = StepClock::with_pit_clocks_per_step(PIT_CLOCKS_PER_CMOS_PERIOD - 1);
        assert_eq!(clock.charge_step().cmos_periods, 0);
        assert_eq!(clock.charge_step().cmos_periods, 1);
        assert_eq!(clock.charge_step().cmos_periods, 1);
    }

    #[test]
    fn reset_accumulators_keeps_configuration() {
        let mut clock = StepClock::with_pit_clocks_per_step(7);
        clock.charge_step();
        clock.reset_accumulators();
        assert_eq!(clock.pit_clocks_per_step, 7);
        assert!(clock.enabled);
        assert_eq!(clock.charge_step().pit_clocks, 7);
    }

    #[test]
    fn zero_ratio_owes_nothing_but_stays_armed() {
        let mut clock = StepClock::with_pit_clocks_per_step(0);
        assert!(clock.enabled);
        assert!(clock.charge_step().is_empty());
    }
}
