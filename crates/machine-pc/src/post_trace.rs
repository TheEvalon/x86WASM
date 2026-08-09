//! Bounded event trace of what firmware was *doing* before a POST probe stopped.
//!
//! [`crate::Machine::probe_post`] answers "where did it die". That is enough to
//! pick the next opcode to implement and nothing else: by the time a run stops,
//! everything the firmware did on the way there is gone. This module records a
//! bounded, most-recent-first-dropped window of the platform accesses leading up
//! to the stop — port I/O, PCI configuration cycles, PAM programming, VGA
//! aperture accesses, and memory faults — so a reader can see the sequence
//! rather than only its last instruction.
//!
//! Recording is off unless [`crate::Machine::probe_post_traced`] arms it, and
//! the existing [`crate::PostReport`] output is untouched: a traced run prints
//! the same first lines byte for byte and appends clearly delimited sections.
//!
//! Nothing here changes architectural behavior. Every event is observed on the
//! path a normal run already takes.

use std::collections::VecDeque;
use std::fmt;

/// Events kept when a caller does not choose a capacity.
pub const DEFAULT_POST_TRACE_CAPACITY: usize = 256;

/// How much of a POST run to record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PostTraceConfig {
    /// Most recent events retained. Older events are dropped and counted.
    pub capacity: usize,
}

impl Default for PostTraceConfig {
    fn default() -> Self {
        Self {
            capacity: DEFAULT_POST_TRACE_CAPACITY,
        }
    }
}

impl PostTraceConfig {
    /// Keep the most recent `capacity` events.
    ///
    /// A capacity of zero records nothing but still counts what happened, which
    /// is a cheap way to ask "how much platform traffic was there".
    pub fn with_capacity(capacity: usize) -> Self {
        Self { capacity }
    }
}

/// One platform access observed during a traced POST run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostTraceEvent {
    /// `IN` from a port that is not part of PCI Mechanism #1.
    PortIn { port: u16, size: u8, value: u32 },
    /// `OUT` to a port that is not part of PCI Mechanism #1.
    PortOut { port: u16, size: u8, value: u32 },
    /// Access to CONFIG_ADDRESS (`0xCF8`–`0xCFB`), with the resulting latch.
    ///
    /// Spec: PCI Local Bus Specification Revision 3.0 §3.2.2.3.2.
    PciConfigAddress {
        write: bool,
        port: u16,
        size: u8,
        /// CONFIG_ADDRESS after the access.
        latched: u32,
    },
    /// Access to CONFIG_DATA (`0xCFC`–`0xCFF`) with the target it decoded to.
    PciConfigData {
        write: bool,
        port: u16,
        size: u8,
        value: u32,
        enabled: bool,
        bus: u8,
        device: u8,
        function: u8,
        /// Dword-aligned register number from CONFIG_ADDRESS bits 7:2.
        register: u8,
    },
    /// An i440FX PMC PAM register changed value, re-attributing its segments.
    ///
    /// Spec: Intel 440FX 82441FX (PMC) §3.2.18.
    PamProgram {
        /// `0` = PAM0 (`0x59`) … `6` = PAM6 (`0x5F`).
        index: u8,
        value: u8,
    },
    /// A CPU access the VGA device claimed inside the `0xA0000`–`0xBFFFF`
    /// display aperture.
    VgaAperture { write: bool, addr: u64, value: u8 },
    /// A CPU access that decoded to neither RAM, ROM, nor a claimed device.
    MemoryFault { write: bool, addr: u64 },
    /// A CPU write the platform discarded because the address decoded to a
    /// ROM window. The processor completes the store normally — this event is
    /// the only trace of it.
    ///
    /// Spec: PCI Local Bus Specification Revision 3.0 §3.2.2.3.4; see
    /// `docs/machine-r4-write-semantics.md` for why this is not a fault.
    RomWriteDropped { addr: u64, value: u8 },
}

impl fmt::Display for PostTraceEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PortIn { port, size, value } => {
                write!(
                    f,
                    "port   in  port=0x{port:04X} size={size} value=0x{value:08X}"
                )
            }
            Self::PortOut { port, size, value } => {
                write!(
                    f,
                    "port   out port=0x{port:04X} size={size} value=0x{value:08X}"
                )
            }
            Self::PciConfigAddress {
                write,
                port,
                size,
                latched,
            } => write!(
                f,
                "cfg-addr {} port=0x{port:04X} size={size} latched=0x{latched:08X}",
                direction(*write)
            ),
            Self::PciConfigData {
                write,
                port,
                size,
                value,
                enabled,
                bus,
                device,
                function,
                register,
            } => write!(
                f,
                "cfg-data {} port=0x{port:04X} size={size} value=0x{value:08X} \
                 target={bus:02X}:{device:02X}.{function} reg=0x{register:02X} en={}",
                direction(*write),
                u8::from(*enabled)
            ),
            Self::PamProgram { index, value } => {
                write!(f, "pam        PAM{index} value=0x{value:02X}")
            }
            Self::VgaAperture { write, addr, value } => write!(
                f,
                "vga-mmio {} addr=0x{addr:08X} value=0x{value:02X}",
                direction(*write)
            ),
            Self::MemoryFault { write, addr } => {
                write!(f, "mem-fault {} addr=0x{addr:016X}", direction(*write))
            }
            Self::RomWriteDropped { addr, value } => {
                write!(
                    f,
                    "rom-write wr addr=0x{addr:016X} value=0x{value:02X} dropped"
                )
            }
        }
    }
}

fn direction(write: bool) -> &'static str {
    if write {
        "wr"
    } else {
        "rd"
    }
}

/// A bounded ring of the most recent [`PostTraceEvent`]s.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PostTrace {
    capacity: usize,
    /// Retained events, each with its sequence number in the full stream.
    events: VecDeque<(u64, PostTraceEvent)>,
    /// Events observed, including those dropped to stay inside `capacity`.
    total: u64,
}

impl PostTrace {
    pub fn new(config: PostTraceConfig) -> Self {
        Self {
            capacity: config.capacity,
            events: VecDeque::new(),
            total: 0,
        }
    }

    pub fn record(&mut self, event: PostTraceEvent) {
        self.total = self.total.saturating_add(1);
        if self.capacity == 0 {
            return;
        }
        if self.events.len() == self.capacity {
            self.events.pop_front();
        }
        self.events.push_back((self.total - 1, event));
    }

    /// Retained events, oldest first, each with its sequence number.
    pub fn events(&self) -> impl Iterator<Item = (u64, PostTraceEvent)> + '_ {
        self.events.iter().copied()
    }

    /// Events observed, including dropped ones.
    pub fn total(&self) -> u64 {
        self.total
    }

    /// Events dropped to stay inside the configured capacity.
    pub fn dropped(&self) -> u64 {
        self.total - self.events.len() as u64
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Count of retained events matching a predicate — the shape a test wants.
    pub fn count_matching(&self, mut pred: impl FnMut(&PostTraceEvent) -> bool) -> usize {
        self.events.iter().filter(|(_, e)| pred(e)).count()
    }
}

impl fmt::Display for PostTrace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "post-trace: events={} kept={} dropped={} capacity={}",
            self.total,
            self.events.len(),
            self.dropped(),
            self.capacity
        )?;
        for (seq, event) in &self.events {
            write!(f, "\n  [{seq:6}] {event}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_keeps_the_most_recent_events_and_counts_the_rest() {
        let mut trace = PostTrace::new(PostTraceConfig::with_capacity(2));
        for port in 0..5u16 {
            trace.record(PostTraceEvent::PortOut {
                port,
                size: 1,
                value: u32::from(port),
            });
        }

        assert_eq!(trace.total(), 5);
        assert_eq!(trace.len(), 2);
        assert_eq!(trace.dropped(), 3);
        let kept: Vec<(u64, PostTraceEvent)> = trace.events().collect();
        assert_eq!(kept[0].0, 3);
        assert_eq!(
            kept[1].1,
            PostTraceEvent::PortOut {
                port: 4,
                size: 1,
                value: 4
            }
        );
    }

    #[test]
    fn zero_capacity_counts_without_retaining() {
        let mut trace = PostTrace::new(PostTraceConfig::with_capacity(0));
        trace.record(PostTraceEvent::MemoryFault {
            write: false,
            addr: 0xDEAD,
        });
        assert_eq!(trace.total(), 1);
        assert_eq!(trace.dropped(), 1);
        assert!(trace.is_empty());
    }

    #[test]
    fn event_lines_name_the_access() {
        let line = PostTraceEvent::PciConfigData {
            write: true,
            port: 0x0CFD,
            size: 1,
            value: 0x30,
            enabled: true,
            bus: 0,
            device: 0,
            function: 0,
            register: 0x58,
        }
        .to_string();
        assert!(line.starts_with("cfg-data wr port=0x0CFD"), "{line}");
        assert!(line.contains("target=00:00.0 reg=0x58 en=1"), "{line}");
    }
}
