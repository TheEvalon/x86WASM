//! Intel 8254-compatible Programmable Interval Timer — channel 0 programming + OUT tick.
//!
//! Classic PC ports: counters `0x40`/`0x41`/`0x42`, control word `0x43`.
//! Channel 0 OUT drives ISA IRQ0 when wired by `machine-pc` (8259A master IR0).
//!
//! # Spec refs
//!
//! - Intel 8254 Programmable Interval Timer datasheet — control word format
//!   (SC/RW/M/BCD), counter latch command, LSB/MSB / LSB-then-MSB access,
//!   operating modes 0–5; mode 0/2/3 OUT pin behavior (GATE assumed high).
//! - Intel 8259A — edge-triggered IR: low→high latches IRR (wired in `machine-pc`).
//! - Classic IBM PC/AT I/O map: `0x40`–`0x43`; ch0 OUT → IRQ0.
//! - `docs/machine-model-pc-v1.md`, `docs/sources.md`, `plan.md` §15.3 / §21.
//!
//! # Scope (this slice)
//!
//! Channel 0 control-word programming (modes 0, 2, 3 required; other mode bits
//! stored), access-mode count load, counter-latch read-back, counting-element
//! (`ce`) advancement via [`Pit8254::tick_ch0`], and OUT pin level / rising-edge
//! reporting for IRQ0. Channels 1/2 accept control words and byte I/O but are
//! **not** claimed as fully supported.
//!
//! # Unsupported (explicit)
//!
//! - Gate input (assumed always high)
//! - Modes 1 / 4 / 5 OUT/IRQ claims — programmed/stored; `tick_ch0` is a no-op
//! - Mode 3 exact 50% duty cycle (simplified: one rising OUT edge per period)
//! - BCD counting during tick (BCD flag stored; tick uses binary)
//! - Read-back command (`SC=11`) status/count latches (ignored)
//! - Channel 1 DRAM refresh and channel 2 PC speaker semantics
//! - Host-real-time wall-clock rate (callers choose tick quantum)

use crate::PortDevice;

/// Channel 0 counter data port (classic PC).
pub const PIT_CH0_DATA: u16 = 0x40;
/// Channel 1 counter data port (stubbed).
pub const PIT_CH1_DATA: u16 = 0x41;
/// Channel 2 counter data port (stubbed).
pub const PIT_CH2_DATA: u16 = 0x42;
/// Control-word / read-back port.
pub const PIT_CONTROL: u16 = 0x43;

/// Control-word SC field: select channel / latch / read-back.
const CW_SC_SHIFT: u8 = 6;
/// Control-word RW field: access mode.
const CW_RW_SHIFT: u8 = 4;
const CW_RW_MASK: u8 = 0b11;
/// Control-word M field: operating mode (bits 3:1; mode 2/3 encode with bit 3).
const CW_MODE_SHIFT: u8 = 1;
const CW_MODE_MASK: u8 = 0b111;
/// Control-word BCD bit.
const CW_BCD: u8 = 1 << 0;

/// Access mode: counter latch command (RW=00).
const RW_LATCH: u8 = 0b00;
/// Access mode: lobyte only.
const RW_LO: u8 = 0b01;
/// Access mode: hibyte only.
const RW_HI: u8 = 0b10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AccessMode {
    Latch,
    Lo,
    Hi,
    LoHi,
}

impl AccessMode {
    fn from_rw(rw: u8) -> Self {
        match rw & CW_RW_MASK {
            RW_LATCH => Self::Latch,
            RW_LO => Self::Lo,
            RW_HI => Self::Hi,
            _ => Self::LoHi,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BytePhase {
    /// Next data write is the low byte (or only byte for Lo mode).
    ExpectLo,
    /// Next data write is the high byte.
    ExpectHi,
    /// Count fully loaded for current access program.
    Complete,
}

/// One 8254 counter channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PitChannel {
    /// Last non-latch control word for this channel (0 after reset).
    pub control_word: u8,
    /// Operating mode 0–5 (decoded from control-word M field).
    pub mode: u8,
    /// BCD counting when true.
    pub bcd: bool,
    /// Programmed access mode (updated on non-latch control words).
    access: AccessMode,
    write_phase: BytePhase,
    read_phase: BytePhase,
    /// Programmed reload / divisor (0 means 65536 counts per datasheet convention).
    pub count: u16,
    /// Current counting element (runtime). Separated from programmed `count`.
    pub ce: u32,
    /// Channel OUT pin level (Intel 8254 OUT).
    pub out_level: bool,
    /// True while the counting element is advancing under modes 0/2/3.
    pub counting: bool,
    /// Mode 2: OUT is low for this model clock; next clock rises.
    mode2_out_low: bool,
    /// Latched value for read-back after a latch command.
    latched: Option<u16>,
    /// Whether a full count has been written since the last mode program.
    pub count_loaded: bool,
}

impl PitChannel {
    fn new() -> Self {
        Self {
            control_word: 0,
            mode: 0,
            bcd: false,
            access: AccessMode::LoHi,
            write_phase: BytePhase::ExpectLo,
            read_phase: BytePhase::ExpectLo,
            count: 0,
            ce: 0,
            out_level: false,
            counting: false,
            mode2_out_low: false,
            latched: None,
            count_loaded: false,
        }
    }

    fn reset(&mut self) {
        *self = Self::new();
    }

    fn decode_mode(m_bits: u8) -> u8 {
        // Datasheet: M2 M1 M0; modes 2 and 3 ignore M2 (encode as x10 / x11).
        match m_bits & CW_MODE_MASK {
            0b000 => 0,
            0b001 => 1,
            0b010 | 0b110 => 2,
            0b011 | 0b111 => 3,
            0b100 => 4,
            0b101 => 5,
            _ => 0,
        }
    }

    /// Reload value for `ce`: written 0 → 65536 (Intel 8254).
    fn reload_ce(&self) -> u32 {
        if self.count == 0 {
            65536
        } else {
            u32::from(self.count)
        }
    }

    /// Value captured by a counter-latch command.
    fn latch_snapshot(&self) -> u16 {
        if self.counting {
            (self.ce & 0xFFFF) as u16
        } else {
            self.count
        }
    }

    fn apply_control(&mut self, value: u8) {
        let rw = (value >> CW_RW_SHIFT) & CW_RW_MASK;
        if rw == RW_LATCH {
            // Counter latch: freeze current CE (when counting) or programmed count.
            self.latched = Some(self.latch_snapshot());
            self.read_phase = BytePhase::ExpectLo;
            return;
        }

        self.control_word = value;
        self.access = AccessMode::from_rw(rw);
        self.mode = Self::decode_mode((value >> CW_MODE_SHIFT) & CW_MODE_MASK);
        self.bcd = value & CW_BCD != 0;
        self.count_loaded = false;
        self.latched = None;
        self.counting = false;
        self.mode2_out_low = false;
        self.ce = 0;
        // Spec: Intel 8254 — after control word, mode 0 OUT low; modes 2/3 OUT high
        // (GATE high assumed). Modes 1/4/5: OUT not claimed for IRQ in this slice.
        self.out_level = matches!(self.mode, 2 | 3);
        self.write_phase = match self.access {
            AccessMode::Hi => BytePhase::ExpectHi,
            _ => BytePhase::ExpectLo,
        };
        self.read_phase = match self.access {
            AccessMode::Hi => BytePhase::ExpectHi,
            _ => BytePhase::ExpectLo,
        };
    }

    fn arm_count_loaded(&mut self) {
        self.count_loaded = true;
        // Spec: Intel 8254 — full count load arms CE; 0 encodes 65536.
        self.ce = self.reload_ce();
        self.mode2_out_low = false;
        // Modes 0/2/3: start counting (GATE always high). Modes 1/4/5: store only.
        self.counting = matches!(self.mode, 0 | 2 | 3);
    }

    fn write_data(&mut self, value: u8) {
        match self.access {
            AccessMode::Latch => {
                // Latched state does not accept count programming until a new CW.
            }
            AccessMode::Lo => {
                self.count = (self.count & 0xFF00) | u16::from(value);
                self.write_phase = BytePhase::Complete;
                self.arm_count_loaded();
            }
            AccessMode::Hi => {
                self.count = (self.count & 0x00FF) | (u16::from(value) << 8);
                self.write_phase = BytePhase::Complete;
                self.arm_count_loaded();
            }
            AccessMode::LoHi => match self.write_phase {
                BytePhase::ExpectLo | BytePhase::Complete => {
                    self.count = (self.count & 0xFF00) | u16::from(value);
                    self.write_phase = BytePhase::ExpectHi;
                    self.count_loaded = false;
                    self.counting = false;
                }
                BytePhase::ExpectHi => {
                    self.count = (self.count & 0x00FF) | (u16::from(value) << 8);
                    self.write_phase = BytePhase::Complete;
                    self.arm_count_loaded();
                }
            },
        }
    }

    fn read_data(&mut self) -> u8 {
        let value = self.latched.unwrap_or(self.count);
        match self.access {
            AccessMode::Latch | AccessMode::LoHi => match self.read_phase {
                BytePhase::ExpectLo | BytePhase::Complete => {
                    self.read_phase = BytePhase::ExpectHi;
                    (value & 0xFF) as u8
                }
                BytePhase::ExpectHi => {
                    self.read_phase = BytePhase::ExpectLo;
                    // Latch consumed after both bytes (datasheet latch read-out).
                    if self.latched.is_some() {
                        self.latched = None;
                    }
                    (value >> 8) as u8
                }
            },
            AccessMode::Lo => (value & 0xFF) as u8,
            AccessMode::Hi => (value >> 8) as u8,
        }
    }

    /// Advance one model CLK. Returns true if OUT had a rising edge this clock.
    ///
    /// Spec: Intel 8254 modes 0 / 2 / 3 OUT (GATE high). Modes 1/4/5: no-op.
    fn tick_one(&mut self) -> bool {
        if !self.counting {
            return false;
        }
        match self.mode {
            0 => self.tick_mode0(),
            2 => self.tick_mode2(),
            3 => self.tick_mode3(),
            // Modes 1/4/5: unsupported for IRQ/OUT claims in this slice.
            _ => false,
        }
    }

    /// Mode 0 — interrupt on terminal count: OUT rises at CE→0 and stays high.
    fn tick_mode0(&mut self) -> bool {
        let prev = self.out_level;
        if self.ce > 0 {
            self.ce -= 1;
        }
        if self.ce == 0 {
            self.out_level = true;
            self.counting = false;
        }
        !prev && self.out_level
    }

    /// Mode 2 — rate generator: at terminal, OUT low one clock then high; reload CE.
    fn tick_mode2(&mut self) -> bool {
        let prev = self.out_level;
        if self.mode2_out_low {
            // Prior clock was the one-clock OUT low pulse — rise and continue.
            self.out_level = true;
            self.mode2_out_low = false;
            return !prev && self.out_level;
        }
        if self.ce <= 1 {
            // Terminal: OUT low for one model clock; reload CE (period = N).
            self.out_level = false;
            self.mode2_out_low = true;
            self.ce = self.reload_ce();
        } else {
            self.ce -= 1;
            self.out_level = true;
        }
        !prev && self.out_level
    }

    /// Mode 3 — square wave (simplified): one rising OUT edge per programmed period.
    ///
    /// Honesty: exact 50% high/low duty is not modeled; we pulse OUT low for one
    /// model clock at terminal count then high (same rising-edge cadence as mode 2).
    fn tick_mode3(&mut self) -> bool {
        // Reuse mode-2 edge cadence; duty-cycle asymmetry is documented above.
        self.tick_mode2()
    }
}

/// 8254 PIT with three channels; channel 0 is the supported programming + OUT surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pit8254 {
    pub channels: [PitChannel; 3],
}

impl Pit8254 {
    pub fn new() -> Self {
        Self {
            channels: [PitChannel::new(), PitChannel::new(), PitChannel::new()],
        }
    }

    pub fn reset(&mut self) {
        for ch in &mut self.channels {
            ch.reset();
        }
    }

    pub fn channel0(&self) -> &PitChannel {
        &self.channels[0]
    }

    /// Channel 0 OUT pin level (Intel 8254 OUT → PC IRQ0 when wired).
    pub fn out_ch0(&self) -> bool {
        self.channels[0].out_level
    }

    /// Advance channel 0 by `clocks` model ticks.
    ///
    /// Returns `true` if OUT had at least one rising edge during the quantum
    /// (useful for 8259A edge IR latching). Guest wall-clock is not host-real-time.
    pub fn tick_ch0(&mut self, clocks: u64) -> bool {
        let mut rising = false;
        for _ in 0..clocks {
            if self.channels[0].tick_one() {
                rising = true;
            }
        }
        rising
    }
}

impl Default for Pit8254 {
    fn default() -> Self {
        Self::new()
    }
}

impl PortDevice for Pit8254 {
    fn port_read(&mut self, port: u16, _size: u8) -> u32 {
        match port {
            PIT_CH0_DATA => u32::from(self.channels[0].read_data()),
            PIT_CH1_DATA => u32::from(self.channels[1].read_data()),
            PIT_CH2_DATA => u32::from(self.channels[2].read_data()),
            PIT_CONTROL => 0xFF, // control port is write-only on real hardware
            _ => 0xFFFF_FFFF,
        }
    }

    fn port_write(&mut self, port: u16, _size: u8, value: u32) {
        let v = value as u8;
        match port {
            PIT_CH0_DATA => self.channels[0].write_data(v),
            PIT_CH1_DATA => self.channels[1].write_data(v),
            PIT_CH2_DATA => self.channels[2].write_data(v),
            PIT_CONTROL => {
                let sc = (v >> CW_SC_SHIFT) & 0b11;
                if sc == 0b11 {
                    // Read-back command — unsupported in this slice.
                    return;
                }
                self.channels[sc as usize].apply_control(v);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spec: cold reset — counts unloaded, mode 0 defaults (Intel 8254).
    #[test]
    fn reset_state() {
        let pit = Pit8254::new();
        assert_eq!(pit.channel0().count, 0);
        assert!(!pit.channel0().count_loaded);
        assert_eq!(pit.channel0().mode, 0);
        assert!(!pit.channel0().bcd);
        assert_eq!(pit.channel0().control_word, 0);
        assert!(!pit.out_ch0());
        assert_eq!(pit.channel0().ce, 0);
        assert!(!pit.channel0().counting);

        let mut pit2 = Pit8254::new();
        pit2.port_write(PIT_CONTROL, 1, 0x36);
        pit2.port_write(PIT_CH0_DATA, 1, 0x00);
        pit2.port_write(PIT_CH0_DATA, 1, 0x10);
        assert!(pit2.out_ch0());
        assert!(pit2.channel0().counting);
        pit2.reset();
        assert_eq!(pit2, Pit8254::new());
        assert!(!pit2.out_ch0());
        assert_eq!(pit2.channel0().ce, 0);
        assert!(!pit2.channel0().counting);
    }

    /// Spec: channel 0 modes 0 / 2 / 3 via control word M field; OUT init levels.
    #[test]
    fn channel0_modes_0_2_3() {
        let mut pit = Pit8254::new();

        // SC=0, RW=11 (lohi), M=000 (mode 0), BCD=0 → 0x30
        pit.port_write(PIT_CONTROL, 1, 0x30);
        assert_eq!(pit.channel0().mode, 0);
        assert_eq!(pit.channel0().control_word, 0x30);
        assert!(!pit.out_ch0()); // mode 0: OUT low after CW

        // Mode 2 rate generator: M bits 010 → 0x34
        pit.port_write(PIT_CONTROL, 1, 0x34);
        assert_eq!(pit.channel0().mode, 2);
        assert!(pit.out_ch0()); // modes 2/3: OUT high after CW

        // Mode 3 square wave: M bits 011 → 0x36 (classic PC IRQ0)
        pit.port_write(PIT_CONTROL, 1, 0x36);
        assert_eq!(pit.channel0().mode, 3);
        assert!(!pit.channel0().bcd);
        assert!(pit.out_ch0());
    }

    /// Spec: LSB then MSB load of 16-bit count (access mode 11b); arms CE.
    #[test]
    fn channel0_lohi_count_load() {
        let mut pit = Pit8254::new();
        pit.port_write(PIT_CONTROL, 1, 0x36); // ch0, lohi, mode 3
        assert!(!pit.channel0().count_loaded);
        pit.port_write(PIT_CH0_DATA, 1, 0x34); // low
        assert!(!pit.channel0().count_loaded);
        assert_eq!(pit.channel0().count & 0xFF, 0x34);
        pit.port_write(PIT_CH0_DATA, 1, 0x12); // high
        assert!(pit.channel0().count_loaded);
        assert_eq!(pit.channel0().count, 0x1234);
        assert_eq!(pit.channel0().ce, 0x1234);
        assert!(pit.channel0().counting);
    }

    /// Spec: RW=00 counter latch; subsequent reads return latched value.
    #[test]
    fn channel0_latch_read() {
        let mut pit = Pit8254::new();
        pit.port_write(PIT_CONTROL, 1, 0x36);
        pit.port_write(PIT_CH0_DATA, 1, 0x78);
        pit.port_write(PIT_CH0_DATA, 1, 0x56);
        assert_eq!(pit.channel0().count, 0x5678);

        // Latch command: SC=0, RW=00 → 0x00 — captures CE while counting.
        pit.port_write(PIT_CONTROL, 1, 0x00);
        // Mutate live CE without a new full program — latch must stay.
        pit.channels[0].ce = 0xABCD;

        let lo = pit.port_read(PIT_CH0_DATA, 1) as u8;
        let hi = pit.port_read(PIT_CH0_DATA, 1) as u8;
        assert_eq!(u16::from(lo) | (u16::from(hi) << 8), 0x5678);
    }

    /// Spec: Intel 8254 mode 0 — after count N, OUT rises once and stays high.
    #[test]
    fn mode0_tick_out_rises_once() {
        let mut pit = Pit8254::new();
        pit.port_write(PIT_CONTROL, 1, 0x30); // ch0 lohi mode 0
        assert!(!pit.out_ch0());
        pit.port_write(PIT_CH0_DATA, 1, 0x05);
        pit.port_write(PIT_CH0_DATA, 1, 0x00); // count = 5
        assert!(!pit.out_ch0());
        assert!(pit.channel0().counting);

        assert!(!pit.tick_ch0(4));
        assert!(!pit.out_ch0());
        assert!(pit.tick_ch0(1)); // 5th clock: terminal → OUT rising edge
        assert!(pit.out_ch0());
        assert!(!pit.channel0().counting);
        // Stays high; no further rising edges.
        assert!(!pit.tick_ch0(10));
        assert!(pit.out_ch0());
    }

    /// Spec: Intel 8254 mode 2 — rate generator OUT low one clock then high (rising).
    #[test]
    fn mode2_tick_rising_edge_per_period() {
        let mut pit = Pit8254::new();
        pit.port_write(PIT_CONTROL, 1, 0x34); // ch0 lohi mode 2
        pit.port_write(PIT_CH0_DATA, 1, 0x03);
        pit.port_write(PIT_CH0_DATA, 1, 0x00); // count = 3
        assert!(pit.out_ch0());

        // Period 3: after enough clocks, at least one rising edge.
        let rising = pit.tick_ch0(4);
        assert!(rising);
        assert!(pit.out_ch0());
    }

    /// Spec: Intel 8254 mode 3 — square wave; one rising OUT edge per period (simplified duty).
    #[test]
    fn mode3_tick_rising_edge_per_period() {
        let mut pit = Pit8254::new();
        pit.port_write(PIT_CONTROL, 1, 0x36); // ch0 lohi mode 3
        pit.port_write(PIT_CH0_DATA, 1, 0x04);
        pit.port_write(PIT_CH0_DATA, 1, 0x00); // count = 4
        assert!(pit.out_ch0());

        let rising = pit.tick_ch0(5);
        assert!(rising);
        assert!(pit.out_ch0());
    }

    #[test]
    fn bcd_flag_stored() {
        let mut pit = Pit8254::new();
        // Mode 3 lohi + BCD → 0x37
        pit.port_write(PIT_CONTROL, 1, 0x37);
        assert!(pit.channel0().bcd);
        assert_eq!(pit.channel0().mode, 3);
    }

    #[test]
    fn read_back_command_ignored() {
        let mut pit = Pit8254::new();
        pit.port_write(PIT_CONTROL, 1, 0x36);
        pit.port_write(PIT_CH0_DATA, 1, 0x00);
        pit.port_write(PIT_CH0_DATA, 1, 0x10);
        let before = pit.channel0().clone();
        // SC=11 read-back — unsupported; must not alter channel 0 program.
        pit.port_write(PIT_CONTROL, 1, 0xC2);
        assert_eq!(pit.channel0(), &before);
    }

    #[test]
    fn channels_1_and_2_accept_but_undocumented_as_full() {
        // Honest stub: control + data accepted; no IRQ/speaker claims.
        let mut pit = Pit8254::new();
        pit.port_write(PIT_CONTROL, 1, 0x76); // ch1 mode 3 lohi
        pit.port_write(PIT_CH1_DATA, 1, 0x01);
        pit.port_write(PIT_CH1_DATA, 1, 0x00);
        assert!(pit.channels[1].count_loaded);
        assert_eq!(pit.channels[1].count, 0x0001);

        pit.port_write(PIT_CONTROL, 1, 0xB6); // ch2 mode 3 lohi
        pit.port_write(PIT_CH2_DATA, 1, 0xFF);
        pit.port_write(PIT_CH2_DATA, 1, 0x00);
        assert_eq!(pit.channels[2].count, 0x00FF);
    }

    #[test]
    fn state_clone_equality_round_trip() {
        let mut pit = Pit8254::new();
        pit.port_write(PIT_CONTROL, 1, 0x36);
        pit.port_write(PIT_CH0_DATA, 1, 0x00);
        pit.port_write(PIT_CH0_DATA, 1, 0x04);
        let cloned = pit.clone();
        assert_eq!(pit, cloned);
        assert_eq!(cloned.channel0().count, 0x0400);
        assert_eq!(cloned.channel0().mode, 3);
    }

    #[test]
    fn unrelated_ports_ignored() {
        let mut pit = Pit8254::new();
        pit.port_write(0x20, 1, 0x36);
        assert_eq!(pit.channel0().control_word, 0);
        assert_eq!(pit.port_read(0x20, 1), 0xFFFF_FFFF);
    }
}
