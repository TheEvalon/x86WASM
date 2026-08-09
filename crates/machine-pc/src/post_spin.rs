//! Where a POST probe stopped, and whether it was going in circles.
//!
//! [`crate::Machine::probe_post`] could report a failure site precisely and a
//! step-budget stop not at all. That asymmetry cost round 3 a hand bisection to
//! turn "SeaBIOS spins" into "SeaBIOS spins at `0xFFFF6E06`". This module adds
//! the missing half: the program counter at the stop, plus a bounded histogram
//! of the trailing program counters and detection of a tight repeating cycle.
//!
//! Nothing here changes architectural behavior. Sampling costs one ring push
//! per retired instruction while a probe is running, and nothing at all
//! otherwise.
//!
//! Spec: Intel SDM Vol. 1 §3.5 (`EIP`/`IP`); Vol. 3 §3.4.2 (linear address =
//! cached segment base + offset); Vol. 3 §3.4.5 (the `D` flag selects the
//! 16- or 32-bit execution window).

use std::collections::HashMap;
use std::collections::VecDeque;
use std::fmt;

use x86_core::CpuState;

/// Trailing instructions sampled when a caller does not choose.
pub const DEFAULT_POST_SPIN_WINDOW: usize = 4096;
/// Distinct program counters listed when a caller does not choose.
pub const DEFAULT_POST_SPIN_HOT: usize = 4;
/// Longest repeating cycle looked for when a caller does not choose.
pub const DEFAULT_POST_SPIN_MAX_PERIOD: usize = 64;
/// Repeats of a candidate period that must be present to call it a cycle.
///
/// Two is the smallest number that distinguishes a loop from a coincidence:
/// one repetition can happen in straight-line code that revisits an address.
const MIN_CYCLE_REPEATS: usize = 3;

/// One code location, printed the way [`crate::PostFailure`] prints its site.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PostPcSite {
    pub cs: u16,
    /// The offset executing in the `CS.D` window: `IP` when `CS.D=0`, the full
    /// `EIP` when `CS.D=1`.
    pub eip: u32,
    /// Cached `CS.D`/B bit (`true` = 32-bit code segment).
    pub cs_default_big: bool,
    pub linear_pc: u64,
}

impl PostPcSite {
    /// Sample the current instruction pointer.
    pub fn from_cpu(cpu: &CpuState) -> Self {
        let cs_default_big = cpu.cs.default_big();
        let eip = if cs_default_big {
            cpu.rip as u32
        } else {
            u32::from(cpu.rip as u16)
        };
        Self {
            cs: cpu.cs.selector,
            eip,
            cs_default_big,
            linear_pc: cpu.cs.base.wrapping_add(u64::from(eip)),
        }
    }
}

impl fmt::Display for PostPcSite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "cs:ip={:04X}:{:04X} cs.d={} eip=0x{:08X} linear_pc=0x{:016X}",
            self.cs,
            self.eip as u16,
            u8::from(self.cs_default_big),
            self.eip,
            self.linear_pc
        )
    }
}

/// How much of the instruction stream to keep for the spin summary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PostSpinConfig {
    /// Trailing retired instructions sampled. Zero disables the summary.
    pub window: usize,
    /// Distinct program counters reported, most frequent first.
    pub hot: usize,
    /// Longest repeating cycle looked for.
    pub max_period: usize,
}

impl Default for PostSpinConfig {
    fn default() -> Self {
        Self {
            window: DEFAULT_POST_SPIN_WINDOW,
            hot: DEFAULT_POST_SPIN_HOT,
            max_period: DEFAULT_POST_SPIN_MAX_PERIOD,
        }
    }
}

impl PostSpinConfig {
    /// Keep the most recent `window` program counters (zero disables).
    pub fn with_window(window: usize) -> Self {
        Self {
            window,
            ..Self::default()
        }
    }
}

/// A repeating instruction cycle found at the end of the sampled window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PostSpinCycle {
    /// Instructions in one revolution.
    pub period: usize,
    /// Consecutive revolutions confirmed inside the window. Bounded by the
    /// window, so this is "at least this many", not the loop's trip count.
    pub repeats: u64,
    /// One revolution in execution order, ending with the most recently
    /// retired instruction — so `sites[0]` is the instruction that would have
    /// executed next, which is also [`crate::PostReport::stop_site`].
    pub sites: Vec<PostPcSite>,
}

/// What the last `window` retired instructions were doing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PostSpinSummary {
    /// Program counters retained (at most `window`).
    pub sampled: u64,
    pub window: usize,
    /// Distinct program counters among them.
    pub distinct: usize,
    /// Most frequent program counters, descending, then by address.
    ///
    /// Only counters seen more than once are listed: in straight-line code
    /// every entry would be `count=1`, which says nothing a reader needs.
    pub hot: Vec<(PostPcSite, u64)>,
    pub cycle: Option<PostSpinCycle>,
}

impl fmt::Display for PostSpinSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "  spin           sampled={} window={} distinct={} ",
            self.sampled, self.window, self.distinct
        )?;
        match &self.cycle {
            Some(cycle) => write!(f, "cycle={} repeats={}", cycle.period, cycle.repeats)?,
            None => f.write_str("cycle=none")?,
        }
        if let Some(cycle) = &self.cycle {
            for (index, site) in cycle.sites.iter().enumerate() {
                write!(f, "\n  spin-cycle     [{index}] {site}")?;
            }
        }
        for (site, count) in &self.hot {
            write!(f, "\n  spin-pc        count={count} {site}")?;
        }
        Ok(())
    }
}

/// Bounded ring of trailing program counters.
#[derive(Debug)]
pub struct PostSpinSampler {
    config: PostSpinConfig,
    sites: VecDeque<PostPcSite>,
}

impl PostSpinSampler {
    /// `None` when the configuration records nothing.
    pub fn new(config: PostSpinConfig) -> Option<Self> {
        (config.window > 0).then(|| Self {
            config,
            sites: VecDeque::with_capacity(config.window.min(1 << 16)),
        })
    }

    pub fn record(&mut self, site: PostPcSite) {
        if self.sites.len() == self.config.window {
            self.sites.pop_front();
        }
        self.sites.push_back(site);
    }

    /// Fold the window into a summary. Empty windows produce `None`, so a run
    /// that retired no instructions reports nothing rather than zeros.
    pub fn summarize(&self) -> Option<PostSpinSummary> {
        if self.sites.is_empty() {
            return None;
        }
        let samples: Vec<PostPcSite> = self.sites.iter().copied().collect();

        let mut counts: HashMap<u64, (PostPcSite, u64)> = HashMap::new();
        for site in &samples {
            let entry = counts.entry(site.linear_pc).or_insert((*site, 0));
            entry.1 += 1;
        }
        let mut hot: Vec<(PostPcSite, u64)> = counts.values().copied().collect();
        hot.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.linear_pc.cmp(&b.0.linear_pc)));
        let distinct = hot.len();
        hot.retain(|(_, count)| *count > 1);
        hot.truncate(self.config.hot);

        Some(PostSpinSummary {
            sampled: samples.len() as u64,
            window: self.config.window,
            distinct,
            hot,
            cycle: detect_cycle(&samples, self.config.max_period),
        })
    }
}

/// Smallest period `p` whose repetition covers the end of the window at least
/// [`MIN_CYCLE_REPEATS`] times.
///
/// Smallest wins so a self-jump reports period 1 rather than an arbitrary
/// multiple of itself, and the run is measured backwards from the newest
/// sample so a loop entered late in the window is still found.
fn detect_cycle(samples: &[PostPcSite], max_period: usize) -> Option<PostSpinCycle> {
    let len = samples.len();
    for period in 1..=max_period.min(len / MIN_CYCLE_REPEATS) {
        let mut matched = 0usize;
        let mut index = len;
        while index > period
            && samples[index - 1].linear_pc == samples[index - 1 - period].linear_pc
        {
            matched += 1;
            index -= 1;
        }
        let repeats = matched / period;
        if repeats + 1 >= MIN_CYCLE_REPEATS {
            return Some(PostSpinCycle {
                period,
                repeats: repeats as u64 + 1,
                sites: samples[len - period..].to_vec(),
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(linear: u64) -> PostPcSite {
        PostPcSite {
            cs: 0xF000,
            eip: linear as u32,
            cs_default_big: false,
            linear_pc: linear,
        }
    }

    #[test]
    fn a_self_jump_is_period_one() {
        let samples: Vec<PostPcSite> = std::iter::repeat_n(site(0x100), 32).collect();
        let cycle = detect_cycle(&samples, 64).expect("cycle");
        assert_eq!(cycle.period, 1);
        assert_eq!(cycle.repeats, 32);
    }

    #[test]
    fn a_three_instruction_loop_is_period_three_in_execution_order() {
        let samples: Vec<PostPcSite> = (0..30).map(|i| site(0x100 + (i % 3))).collect();
        let cycle = detect_cycle(&samples, 64).expect("cycle");
        assert_eq!(cycle.period, 3);
        let members: Vec<u64> = cycle.sites.iter().map(|s| s.linear_pc).collect();
        assert_eq!(members, vec![0x100, 0x101, 0x102]);
    }

    #[test]
    fn strictly_increasing_program_counters_are_not_a_cycle() {
        let samples: Vec<PostPcSite> = (0..64).map(|i| site(0x100 + i)).collect();
        assert!(detect_cycle(&samples, 64).is_none());
    }

    /// A loop entered part-way through the window is still found, because the
    /// run is measured backwards from the newest sample.
    #[test]
    fn a_loop_entered_late_is_still_found() {
        let mut samples: Vec<PostPcSite> = (0..40).map(|i| site(0x100 + i)).collect();
        samples.extend((0..40).map(|i| site(0x500 + (i % 2))));
        let cycle = detect_cycle(&samples, 64).expect("cycle");
        assert_eq!(cycle.period, 2);
        assert_eq!(cycle.repeats, 20);
    }

    #[test]
    fn the_hot_list_is_ordered_by_frequency() {
        let mut sampler = PostSpinSampler::new(PostSpinConfig::with_window(16)).expect("armed");
        for _ in 0..3 {
            sampler.record(site(0x200));
        }
        for _ in 0..5 {
            sampler.record(site(0x300));
        }
        let summary = sampler.summarize().expect("samples");
        assert_eq!(summary.distinct, 2);
        assert_eq!(summary.hot[0].0.linear_pc, 0x300);
        assert_eq!(summary.hot[0].1, 5);
        assert_eq!(summary.sampled, 8);
    }

    #[test]
    fn a_zero_window_records_nothing() {
        assert!(PostSpinSampler::new(PostSpinConfig::with_window(0)).is_none());
    }
}
