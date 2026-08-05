//! Intel 8254-compatible Programmable Interval Timer — channel 0 programming + OUT tick.
//!
//! Classic PC ports: counters `0x40`/`0x41`/`0x42`, control word `0x43`.
//! Channel 0 OUT drives ISA IRQ0 when wired by `machine-pc` (8259A master IR0).
//!
//! # Spec refs
//!
//! - Intel 8254 Programmable Interval Timer datasheet — control word format
//!   (SC/RW/M/BCD), counter latch command, Read-Back command (`SC=11`,
//!   COUNT/STATUS + CNTn select), status byte (OUT / NULL COUNT / RW / M / BCD),
//!   LSB/MSB / LSB-then-MSB access, operating modes 0–5; "Mode Definitions"
//!   for mode 0/1/2/3/4/5 OUT pin behavior and the GATE-pin operations summary.
//! - Intel 8259A — edge-triggered IR: low→high latches IRR (wired in `machine-pc`).
//! - Classic IBM PC/AT I/O map: `0x40`–`0x43`; ch0 OUT → IRQ0.
//! - `docs/machine-model-pc-v1.md`, `docs/sources.md`, `plan.md` §15.3 / §21.
//!
//! # Scope
//!
//! Channel control-word programming (operating modes 0, 1, 2, 3, 4, 5),
//! access-mode count load, counter-latch and Read-Back status/count latches,
//! counting-element (`ce`) advancement via [`Pit8254::tick_ch0`], BCD countdown
//! when the control-word BCD bit is set (four decades; written `0` → 10_000),
//! and OUT pin level / rising-edge reporting for IRQ0. Channel 2: GATE via
//! port `0x61` bit0, OUT readback on bit5, speaker-data latch bit1 (no host
//! audio), and [`Pit8254::tick_ch2`]. Channel 1 accepts control words and byte
//! I/O but is **not** fully supported.
//!
//! GATE-triggered modes 1 and 5 need a GATE rising edge to start counting, so
//! on this machine model they are only reachable on channel 2 (port `0x61`
//! bit0); channel 0/1 GATE is tied high and never has a rising edge.
//!
//! # Unsupported (explicit)
//!
//! - Channel 0/1 gate input (assumed always high)
//! - Mode 3 sub-CLK / decrement-by-two CE micro-timing (OUT uses an approximate
//!   N/2 high + N/2 low split; odd N uses the datasheet (N+1)/2 high + (N−1)/2 low
//!   asymmetry; latched CE during mode 3 is half-phase remaining, not hardware CE)
//! - Sub-CLK load delay: a count write (modes 0/2/3/4) or GATE trigger
//!   (modes 1/5) takes effect on the same model clock, not on the following
//!   CLK as on real hardware (terminal count lands one CLK early); NULL COUNT
//!   therefore clears on the same model clock the CE is loaded
//! - Channel 1 DRAM refresh; host PC-speaker audio output
//! - Host-real-time wall-clock rate (callers choose tick quantum)
//! - Port `0x61` NMI/parity/refresh toggle side effects (bits other than 0/1/5)
//! - Invalid BCD digit programming (nibbles A–F): decode treats each nibble as a
//!   weighted decade digit; hardware behavior for illegal BCD is unspecified

use crate::PortDevice;

/// Channel 0 counter data port (classic PC).
pub const PIT_CH0_DATA: u16 = 0x40;
/// Channel 1 counter data port (stubbed).
pub const PIT_CH1_DATA: u16 = 0x41;
/// Channel 2 counter data port (stubbed).
pub const PIT_CH2_DATA: u16 = 0x42;
/// Control-word / read-back port.
pub const PIT_CONTROL: u16 = 0x43;
/// System control port B (PPI) — speaker GATE2 / SPKR_EN / OUT2 readback.
pub const PORT_SYSTEM_CONTROL: u16 = 0x61;

/// Port `0x61` bit0: PIT channel 2 GATE (speaker timer).
pub const PORT61_GATE2: u8 = 1 << 0;
/// Port `0x61` bit1: speaker data enable (latched; no host audio).
pub const PORT61_SPKR_DATA: u8 = 1 << 1;
/// Port `0x61` bit5: PIT channel 2 OUT (read).
pub const PORT61_OUT2: u8 = 1 << 5;

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

/// Intel 8254 BCD mode: written count 0 encodes 10_000 clocks (four decades).
const BCD_MAX_COUNT: u32 = 10_000;

/// Decode a 16-bit BCD count (four decades) to a binary tick count.
///
/// Spec: Intel 8254 — BCD=1 selects a Binary Coded Decimal counter (4 decades).
/// Nibbles A–F are illegal on real hardware; this model still weights each
/// nibble as a decade digit (undefined case documented in the module header).
fn bcd16_to_count(v: u16) -> u32 {
    u32::from(v & 0xF)
        + 10 * u32::from((v >> 4) & 0xF)
        + 100 * u32::from((v >> 8) & 0xF)
        + 1000 * u32::from((v >> 12) & 0xF)
}

/// Encode a binary tick count (0..=9999) as a 16-bit BCD value for latched reads.
/// Values ≥ [`BCD_MAX_COUNT`] (just-loaded 0) read back as `0x0000`.
fn count_to_bcd16(n: u32) -> u16 {
    if n >= BCD_MAX_COUNT {
        return 0;
    }
    let d0 = n % 10;
    let d1 = (n / 10) % 10;
    let d2 = (n / 100) % 10;
    let d3 = (n / 1000) % 10;
    (d0 | (d1 << 4) | (d2 << 8) | (d3 << 12)) as u16
}

/// Read-Back command (SC=11): COUNT=0 latches count of selected counters.
const RB_COUNT: u8 = 1 << 5;
/// Read-Back command (SC=11): STATUS=0 latches status of selected counters.
const RB_STATUS: u8 = 1 << 4;
/// Read-Back CNT0 / CNT1 / CNT2 select bits (1 = select).
const RB_CNT0: u8 = 1 << 1;
const RB_CNT1: u8 = 1 << 2;
const RB_CNT2: u8 = 1 << 3;

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
    /// True while the counting element is advancing (mode/GATE permitting).
    pub counting: bool,
    /// OUT is low for this model clock; next clock rises. Used by the mode 2
    /// rate pulse and the mode 4/5 one-CLK strobe (mode 3 uses half-period `ce`).
    out_low_pulse: bool,
    /// Latched count (output latch / OL) after a counter-latch or Read-Back COUNT.
    latched: Option<u16>,
    /// Latched status byte after a Read-Back with STATUS=0 (Intel 8254 Figure 11).
    status_latched: Option<u8>,
    /// NULL COUNT (status bit6): set until the last written CR is loaded into CE.
    null_count: bool,
    /// Whether a full count has been written since the last mode program.
    pub count_loaded: bool,
    /// GATE input. Ch0/ch1 assumed high; ch2 driven by port `0x61` bit0.
    pub gate: bool,
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
            out_low_pulse: false,
            latched: None,
            status_latched: None,
            null_count: true,
            count_loaded: false,
            gate: true,
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

    /// Reload value for `ce`.
    ///
    /// Spec: Intel 8254 — binary written 0 → 65_536; BCD written 0 → 10_000
    /// (four decades). Non-zero BCD counts are decoded as four BCD decades.
    fn reload_ce(&self) -> u32 {
        if self.bcd {
            if self.count == 0 {
                BCD_MAX_COUNT
            } else {
                bcd16_to_count(self.count)
            }
        } else if self.count == 0 {
            65536
        } else {
            u32::from(self.count)
        }
    }

    /// Mode 3 high-half clocks: even N → N/2; odd N → (N+1)/2.
    ///
    /// Spec: Intel 8254 Mode 3 — square wave duty approximates 50%.
    fn mode3_high_clks(&self) -> u32 {
        self.reload_ce().div_ceil(2)
    }

    /// Mode 3 low-half clocks: even N → N/2; odd N → (N−1)/2.
    fn mode3_low_clks(&self) -> u32 {
        self.reload_ce() / 2
    }

    /// Value captured by a counter-latch / Read-Back COUNT command.
    ///
    /// Spec: Intel 8254 — while counting, the output latch holds the current
    /// counting element; in BCD mode that value is reported as four decades.
    fn latch_snapshot(&self) -> u16 {
        if self.counting {
            if self.bcd {
                count_to_bcd16(self.ce)
            } else {
                (self.ce & 0xFFFF) as u16
            }
        } else {
            self.count
        }
    }

    /// Status byte format (Intel 8254 Figure 11): OUT | NULL COUNT | RW | M | BCD.
    /// Bits 5:0 match the last Mode Control Word; SC is not included.
    fn status_byte(&self) -> u8 {
        let mut s = self.control_word & 0x3F;
        if self.null_count {
            s |= 1 << 6;
        }
        if self.out_level {
            s |= 1 << 7;
        }
        s
    }

    /// Latch count into OL if not already latched (later latches ignored until read).
    fn latch_count(&mut self) {
        if self.latched.is_some() {
            return;
        }
        self.latched = Some(self.latch_snapshot());
        self.read_phase = match self.access {
            AccessMode::Hi => BytePhase::ExpectHi,
            _ => BytePhase::ExpectLo,
        };
    }

    /// Latch status if not already latched (later status latches ignored until read).
    fn latch_status(&mut self) {
        if self.status_latched.is_none() {
            self.status_latched = Some(self.status_byte());
        }
    }

    /// Apply Read-Back COUNT/STATUS latches for this channel.
    ///
    /// Spec: Intel 8254 Read-Back Command — COUNT=0 / STATUS=0 (active-low
    /// sense on those bits) latch the selected fields; unread latches are kept.
    fn apply_read_back(&mut self, latch_count: bool, latch_status: bool) {
        if latch_status {
            self.latch_status();
        }
        if latch_count {
            self.latch_count();
        }
    }

    fn apply_control(&mut self, value: u8) {
        let rw = (value >> CW_RW_SHIFT) & CW_RW_MASK;
        if rw == RW_LATCH {
            // Counter latch: freeze current CE (when counting) or programmed count.
            // Spec: subsequent latches while unread do not replace the OL.
            self.latch_count();
            return;
        }

        self.control_word = value;
        self.access = AccessMode::from_rw(rw);
        self.mode = Self::decode_mode((value >> CW_MODE_SHIFT) & CW_MODE_MASK);
        self.bcd = value & CW_BCD != 0;
        self.count_loaded = false;
        self.null_count = true;
        self.latched = None;
        self.status_latched = None;
        self.counting = false;
        self.out_low_pulse = false;
        self.ce = 0;
        // Spec: Intel 8254 "Mode Definitions" — after the control word only mode 0
        // drives OUT low; modes 1/2/3/4/5 are initially high.
        self.out_level = self.mode != 0;
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
        if matches!(self.mode, 1 | 5) {
            // Spec: Intel 8254 modes 1/5 — the count write is not a trigger; CE
            // is loaded on the GATE rising edge, and a new count written during
            // a one-shot / strobe only takes effect on the next trigger.
            if !self.counting {
                self.ce = self.reload_ce();
                self.null_count = false;
            }
            // else: CR pending until GATE; NULL COUNT stays set.
            return;
        }
        // Spec: Intel 8254 — full count load arms CE; 0 encodes 65536.
        // Mode 3: CE tracks the current half-period (start high).
        self.ce = if self.mode == 3 {
            self.mode3_high_clks()
        } else {
            self.reload_ce()
        };
        self.null_count = false;
        self.out_low_pulse = false;
        // Modes 0/2/3/4: the count write starts counting when GATE is high.
        self.counting = self.gate;
    }

    /// Update GATE. Spec: Intel 8254 GATE-pin operations summary — GATE low
    /// disables counting in modes 0/2/3/4 (and forces OUT high in modes 2/3) but
    /// has no effect in modes 1/5; a GATE rising edge reloads CE and (re)starts
    /// counting in modes 1/2/3/5 and re-enables counting in modes 0/4.
    fn set_gate(&mut self, high: bool) {
        let was = self.gate;
        if was == high {
            return;
        }
        self.gate = high;
        if !high {
            if matches!(self.mode, 2 | 3) {
                self.out_level = true;
                self.out_low_pulse = false;
            }
            // Modes 0/4: OUT unchanged; tick paused while GATE low.
            // Modes 1/5: GATE low has no effect on the in-progress count.
            return;
        }
        // Rising edge.
        if !self.count_loaded {
            return;
        }
        match self.mode {
            0 | 4 => {
                if self.ce > 0 {
                    self.counting = true;
                }
            }
            1 => {
                // Retriggerable one-shot: reload CE, OUT low until terminal count.
                self.ce = self.reload_ce();
                self.null_count = false;
                self.out_low_pulse = false;
                self.out_level = false;
                self.counting = true;
            }
            2 | 3 => {
                self.ce = if self.mode == 3 {
                    self.mode3_high_clks()
                } else {
                    self.reload_ce()
                };
                self.null_count = false;
                self.out_low_pulse = false;
                self.out_level = true;
                self.counting = true;
            }
            5 => {
                // Hardware triggered strobe: reload CE; OUT stays high until the
                // one-CLK strobe at terminal count.
                self.ce = self.reload_ce();
                self.null_count = false;
                self.out_low_pulse = false;
                self.out_level = true;
                self.counting = true;
            }
            _ => {}
        }
    }

    fn write_data(&mut self, value: u8) {
        match self.access {
            AccessMode::Latch => {
                // Latched state does not accept count programming until a new CW.
            }
            AccessMode::Lo => {
                self.count = (self.count & 0xFF00) | u16::from(value);
                self.write_phase = BytePhase::Complete;
                self.null_count = true;
                self.arm_count_loaded();
            }
            AccessMode::Hi => {
                self.count = (self.count & 0x00FF) | (u16::from(value) << 8);
                self.write_phase = BytePhase::Complete;
                self.null_count = true;
                self.arm_count_loaded();
            }
            AccessMode::LoHi => match self.write_phase {
                BytePhase::ExpectLo | BytePhase::Complete => {
                    self.count = (self.count & 0xFF00) | u16::from(value);
                    self.write_phase = BytePhase::ExpectHi;
                    self.count_loaded = false;
                    self.null_count = true;
                    // Modes 1/5: a new count never disturbs the running one-shot
                    // or strobe (Intel 8254 mode 1/5 definitions).
                    if !matches!(self.mode, 1 | 5) {
                        self.counting = false;
                    }
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
        // Spec: Intel 8254 — if both status and count are latched, the first
        // read returns status; subsequent reads return the latched count.
        if let Some(status) = self.status_latched.take() {
            return status;
        }
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
                    self.latched = None;
                    (value >> 8) as u8
                }
            },
            AccessMode::Lo => {
                self.latched = None;
                (value & 0xFF) as u8
            }
            AccessMode::Hi => {
                self.latched = None;
                (value >> 8) as u8
            }
        }
    }

    /// Advance one model CLK. Returns true if OUT had a rising edge this clock.
    ///
    /// Spec: Intel 8254 modes 0–5 OUT. Per the GATE-pin operations summary, GATE
    /// low disables counting in modes 0/2/3/4 but not in modes 1/5.
    fn tick_one(&mut self) -> bool {
        if !self.counting {
            return false;
        }
        if !self.gate && !matches!(self.mode, 1 | 5) {
            return false;
        }
        match self.mode {
            0 => self.tick_mode0(),
            1 => self.tick_mode1(),
            2 => self.tick_mode2(),
            3 => self.tick_mode3(),
            4 | 5 => self.tick_strobe(),
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

    /// Mode 1 — hardware retriggerable one-shot: OUT (already driven low by the
    /// GATE trigger in [`PitChannel::set_gate`]) returns high at terminal count.
    ///
    /// Same counting cadence as mode 0; the modes differ only in what starts the
    /// count and drives OUT low (control word vs GATE trigger).
    fn tick_mode1(&mut self) -> bool {
        self.tick_mode0()
    }

    /// Mode 2 — rate generator: at terminal, OUT low one clock then high; reload CE.
    fn tick_mode2(&mut self) -> bool {
        let prev = self.out_level;
        if self.out_low_pulse {
            // Prior clock was the one-clock OUT low pulse — rise and continue.
            self.out_level = true;
            self.out_low_pulse = false;
            return !prev && self.out_level;
        }
        if self.ce <= 1 {
            // Terminal: OUT low for one model clock; reload CE (period = N).
            self.out_level = false;
            self.out_low_pulse = true;
            self.ce = self.reload_ce();
        } else {
            self.ce -= 1;
            self.out_level = true;
        }
        !prev && self.out_level
    }

    /// Mode 3 — square wave: approximate 50% duty via high/low half-periods.
    ///
    /// Spec: Intel 8254 Mode 3 — even N → N/2 high + N/2 low; odd N → (N+1)/2 high
    /// + (N−1)/2 low. Period and BCD/binary reload use [`Self::reload_ce`].
    ///
    /// Honesty: not the hardware decrement-by-two CE micro-sequence; `ce` holds
    /// clocks remaining in the current OUT half.
    fn tick_mode3(&mut self) -> bool {
        let prev = self.out_level;
        if self.ce > 1 {
            self.ce -= 1;
            return !prev && self.out_level;
        }
        // End of current half-period: flip OUT and load the other half.
        if self.out_level {
            let low = self.mode3_low_clks();
            if low == 0 {
                // Degenerate N=1: datasheet discourages; keep OUT high.
                self.ce = self.mode3_high_clks().max(1);
            } else {
                self.out_level = false;
                self.ce = low;
            }
        } else {
            self.out_level = true;
            self.ce = self.mode3_high_clks().max(1);
        }
        !prev && self.out_level
    }

    /// Modes 4 and 5 — strobe: OUT goes low for exactly one CLK at terminal
    /// count, then returns high (the rising edge). Both are one-shot: mode 4 is
    /// re-armed by writing a new count, mode 5 by a GATE rising edge.
    fn tick_strobe(&mut self) -> bool {
        let prev = self.out_level;
        if self.out_low_pulse {
            // Prior clock was the one-clock strobe — rise and stop counting.
            self.out_level = true;
            self.out_low_pulse = false;
            self.counting = false;
            return !prev && self.out_level;
        }
        if self.ce <= 1 {
            self.ce = 0;
            self.out_level = false;
            self.out_low_pulse = true;
        } else {
            self.ce -= 1;
        }
        !prev && self.out_level
    }
}

/// 8254 PIT with three channels; ch0 IRQ0 + ch2 speaker GATE/OUT via port `0x61`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pit8254 {
    pub channels: [PitChannel; 3],
    /// Port `0x61` bits 1:0 — GATE2 + speaker data enable (no host audio).
    port61_lo: u8,
}

impl Pit8254 {
    pub fn new() -> Self {
        let mut s = Self {
            channels: [PitChannel::new(), PitChannel::new(), PitChannel::new()],
            port61_lo: 0,
        };
        // Ch2 GATE follows port 0x61 bit0 (cleared at reset → GATE low).
        s.channels[2].gate = false;
        s
    }

    pub fn reset(&mut self) {
        for ch in &mut self.channels {
            ch.reset();
        }
        self.port61_lo = 0;
        self.channels[2].gate = false;
    }

    pub fn channel0(&self) -> &PitChannel {
        &self.channels[0]
    }

    pub fn channel2(&self) -> &PitChannel {
        &self.channels[2]
    }

    /// Channel 0 OUT pin level (Intel 8254 OUT → PC IRQ0 when wired).
    pub fn out_ch0(&self) -> bool {
        self.channels[0].out_level
    }

    /// Channel 2 OUT pin level (PC speaker timer; read via port `0x61` bit5).
    pub fn out_ch2(&self) -> bool {
        self.channels[2].out_level
    }

    /// Speaker data enable latch (port `0x61` bit1). No host audio side effect.
    pub fn speaker_data_enabled(&self) -> bool {
        self.port61_lo & PORT61_SPKR_DATA != 0
    }

    /// Set channel 2 GATE (also updated by [`Pit8254::port61_write`]).
    pub fn set_gate_ch2(&mut self, high: bool) {
        self.channels[2].set_gate(high);
        if high {
            self.port61_lo |= PORT61_GATE2;
        } else {
            self.port61_lo &= !PORT61_GATE2;
        }
    }

    /// Read system control port B subset: bits 1:0 latched + bit5 = ch2 OUT.
    ///
    /// Spec: IBM PC/AT PPI port B — bit0 GATE2, bit1 speaker data, bit5 OUT2.
    pub fn port61_read(&self) -> u8 {
        let mut v = self.port61_lo & (PORT61_GATE2 | PORT61_SPKR_DATA);
        if self.out_ch2() {
            v |= PORT61_OUT2;
        }
        v
    }

    /// Write system control port B subset (bits 1:0). Updates ch2 GATE.
    pub fn port61_write(&mut self, value: u8) {
        self.port61_lo = value & (PORT61_GATE2 | PORT61_SPKR_DATA);
        self.channels[2].set_gate(self.port61_lo & PORT61_GATE2 != 0);
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

    /// Advance channel 2 by `clocks` model ticks (GATE-gated).
    ///
    /// Returns `true` if OUT had at least one rising edge during the quantum.
    pub fn tick_ch2(&mut self, clocks: u64) -> bool {
        let mut rising = false;
        for _ in 0..clocks {
            if self.channels[2].tick_one() {
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
                    // Spec: Intel 8254 Read-Back Command format —
                    // D7:D6=11, COUNT(D5)=0 latch count, STATUS(D4)=0 latch status,
                    // CNT2/CNT1/CNT0 (D3:D1)=1 select, D0 reserved.
                    let latch_count = v & RB_COUNT == 0;
                    let latch_status = v & RB_STATUS == 0;
                    if !latch_count && !latch_status {
                        return;
                    }
                    if v & RB_CNT0 != 0 {
                        self.channels[0].apply_read_back(latch_count, latch_status);
                    }
                    if v & RB_CNT1 != 0 {
                        self.channels[1].apply_read_back(latch_count, latch_status);
                    }
                    if v & RB_CNT2 != 0 {
                        self.channels[2].apply_read_back(latch_count, latch_status);
                    }
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
        // Mode 3: CE starts as high-half clocks (even N → N/2).
        assert_eq!(pit.channel0().ce, 0x1234 / 2);
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
        // Mode 3 even N: CE is high-half remaining at load.
        let ce_at_load = pit.channel0().ce;
        assert_eq!(ce_at_load, 0x5678_u32 / 2);

        // Latch command: SC=0, RW=00 → 0x00 — captures CE while counting.
        pit.port_write(PIT_CONTROL, 1, 0x00);
        // Mutate live CE without a new full program — latch must stay.
        pit.channels[0].ce = 0xABCD;

        let lo = pit.port_read(PIT_CH0_DATA, 1) as u8;
        let hi = pit.port_read(PIT_CH0_DATA, 1) as u8;
        assert_eq!(u16::from(lo) | (u16::from(hi) << 8), ce_at_load as u16);
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

    /// Spec: Intel 8254 mode 3 — square wave; one rising OUT edge per period (~50% duty).
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

    /// Spec: Intel 8254 mode 3 — even N: OUT high N/2 then low N/2 (approx 50% duty).
    /// Observes OUT after each model CLK for N=4 over two periods.
    #[test]
    fn mode3_even_count_approx_50_percent_duty() {
        let mut pit = Pit8254::new();
        pit.port_write(PIT_CONTROL, 1, 0x36); // ch0 lohi mode 3
        pit.port_write(PIT_CH0_DATA, 1, 0x04);
        pit.port_write(PIT_CH0_DATA, 1, 0x00); // count = 4
        assert!(pit.out_ch0());

        // Period 1: HHLL; rising edge into period 2 on clock 4.
        assert!(!pit.tick_ch0(1));
        assert!(pit.out_ch0());
        assert!(!pit.tick_ch0(1));
        assert!(!pit.out_ch0()); // end of high half → low
        assert!(!pit.tick_ch0(1));
        assert!(!pit.out_ch0());
        assert!(pit.tick_ch0(1)); // rising at end of low half
        assert!(pit.out_ch0());

        // Period 2: same HHLL pattern.
        assert!(!pit.tick_ch0(1));
        assert!(pit.out_ch0());
        assert!(!pit.tick_ch0(1));
        assert!(!pit.out_ch0());
        assert!(!pit.tick_ch0(1));
        assert!(!pit.out_ch0());
        assert!(pit.tick_ch0(1));
        assert!(pit.out_ch0());
    }

    /// Spec: Intel 8254 mode 3 — odd N: OUT high (N+1)/2 then low (N−1)/2 (asymmetric).
    #[test]
    fn mode3_odd_count_asymmetric_duty() {
        let mut pit = Pit8254::new();
        pit.port_write(PIT_CONTROL, 1, 0x36);
        pit.port_write(PIT_CH0_DATA, 1, 0x05);
        pit.port_write(PIT_CH0_DATA, 1, 0x00); // count = 5
        assert!(pit.out_ch0());

        // High for 3, low for 2; rising on clock 5.
        assert!(!pit.tick_ch0(1));
        assert!(pit.out_ch0());
        assert!(!pit.tick_ch0(1));
        assert!(pit.out_ch0());
        assert!(!pit.tick_ch0(1));
        assert!(!pit.out_ch0()); // after 3 high → low
        assert!(!pit.tick_ch0(1));
        assert!(!pit.out_ch0());
        assert!(pit.tick_ch0(1)); // rising after 2 low
        assert!(pit.out_ch0());
    }

    /// Spec: Intel 8254 mode 3 + BCD — even BCD count uses same N/2 high / N/2 low split
    /// (period from BCD decades via [`PitChannel::reload_ce`], not binary).
    #[test]
    fn mode3_bcd_even_count_approx_50_percent_duty() {
        let mut pit = Pit8254::new();
        // ch0, lohi, mode 3, BCD → 0x37; BCD count 0x0004 → 4 clocks.
        pit.port_write(PIT_CONTROL, 1, 0x37);
        pit.port_write(PIT_CH0_DATA, 1, 0x04);
        pit.port_write(PIT_CH0_DATA, 1, 0x00);
        assert!(pit.channel0().bcd);
        assert_eq!(pit.channel0().ce, 2); // high half of BCD period 4
        assert!(pit.out_ch0());

        assert!(!pit.tick_ch0(1));
        assert!(pit.out_ch0());
        assert!(!pit.tick_ch0(1));
        assert!(!pit.out_ch0());
        assert!(!pit.tick_ch0(1));
        assert!(!pit.out_ch0());
        assert!(pit.tick_ch0(1));
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

    /// Spec: Intel 8254 control-word BCD bit — four-decade BCD counter.
    /// Written count `0x0100` means 100 clocks (not binary 256). Mode 0 OUT
    /// rises at terminal count under BCD semantics.
    #[test]
    fn bcd_mode0_terminal_uses_bcd_period_not_binary() {
        let mut bcd = Pit8254::new();
        // ch0, lohi, mode 0, BCD → 0x31
        bcd.port_write(PIT_CONTROL, 1, 0x31);
        bcd.port_write(PIT_CH0_DATA, 1, 0x00);
        bcd.port_write(PIT_CH0_DATA, 1, 0x01); // BCD 100
        assert!(bcd.channel0().bcd);
        assert_eq!(bcd.channel0().ce, 100);
        assert!(!bcd.tick_ch0(99));
        assert!(!bcd.out_ch0());
        assert!(bcd.tick_ch0(1)); // 100th clock → OUT rising
        assert!(bcd.out_ch0());
        assert!(!bcd.channel0().counting);

        let mut bin = Pit8254::new();
        bin.port_write(PIT_CONTROL, 1, 0x30); // same bytes, binary
        bin.port_write(PIT_CH0_DATA, 1, 0x00);
        bin.port_write(PIT_CH0_DATA, 1, 0x01); // binary 256
        assert_eq!(bin.channel0().ce, 0x0100);
        assert!(!bin.tick_ch0(100)); // still counting after BCD's terminal
        assert!(!bin.out_ch0());
        assert!(bin.channel0().counting);
    }

    /// Spec: Intel 8254 BCD countdown — decade borrow: after one CLK from
    /// BCD `0x0100`, latched CE reads `0x0099` (not binary `0x00FF`).
    #[test]
    fn bcd_tick_latch_shows_decade_borrow() {
        let mut pit = Pit8254::new();
        pit.port_write(PIT_CONTROL, 1, 0x31); // ch0 lohi mode 0 BCD
        pit.port_write(PIT_CH0_DATA, 1, 0x00);
        pit.port_write(PIT_CH0_DATA, 1, 0x01); // BCD 100
        assert!(!pit.tick_ch0(1));
        assert_eq!(pit.channel0().ce, 99);
        // Counter latch SC=0 RW=00 → 0x00
        pit.port_write(PIT_CONTROL, 1, 0x00);
        let lo = pit.port_read(PIT_CH0_DATA, 1) as u8;
        let hi = pit.port_read(PIT_CH0_DATA, 1) as u8;
        assert_eq!(u16::from(lo) | (u16::from(hi) << 8), 0x0099);
    }

    /// Spec: Intel 8254 — initial count 0 in BCD mode means 10_000 clocks;
    /// first CLK yields latched `0x9999`.
    #[test]
    fn bcd_count_zero_means_10000() {
        let mut pit = Pit8254::new();
        pit.port_write(PIT_CONTROL, 1, 0x31);
        pit.port_write(PIT_CH0_DATA, 1, 0x00);
        pit.port_write(PIT_CH0_DATA, 1, 0x00); // BCD 0 → 10000
        assert_eq!(pit.channel0().ce, 10_000);
        assert!(!pit.tick_ch0(1));
        assert_eq!(pit.channel0().ce, 9999);
        pit.port_write(PIT_CONTROL, 1, 0x00);
        let lo = pit.port_read(PIT_CH0_DATA, 1) as u8;
        let hi = pit.port_read(PIT_CH0_DATA, 1) as u8;
        assert_eq!(u16::from(lo) | (u16::from(hi) << 8), 0x9999);
        // Remaining 9999 clocks → OUT rising at total 10000.
        assert!(!pit.tick_ch0(9998));
        assert!(!pit.out_ch0());
        assert!(pit.tick_ch0(1));
        assert!(pit.out_ch0());
    }

    /// Spec: Intel 8254 mode 2 + BCD — period follows BCD value (`0x0020` = 20),
    /// not binary 32; reload at terminal uses the same BCD interpretation.
    #[test]
    fn bcd_mode2_period_uses_bcd_value() {
        let mut pit = Pit8254::new();
        // ch0, lohi, mode 2, BCD → 0x35
        pit.port_write(PIT_CONTROL, 1, 0x35);
        pit.port_write(PIT_CH0_DATA, 1, 0x20);
        pit.port_write(PIT_CH0_DATA, 1, 0x00); // BCD 20
        assert_eq!(pit.channel0().ce, 20);
        // Mode 2: rising edge after one low-pulse clock following terminal.
        assert!(pit.tick_ch0(21));
        assert!(pit.out_ch0());
        // Reloaded to BCD period again.
        assert_eq!(pit.channel0().ce, 20);
    }

    /// Spec: ch2 BCD mode 0 with GATE high; Read-Back status keeps BCD=1 after
    /// tick; reset clears BCD programming.
    #[test]
    fn bcd_ch2_status_readback_and_reset() {
        let mut pit = Pit8254::new();
        pit.port61_write(PORT61_GATE2);
        // ch2, lohi, mode 0, BCD → 0xB1
        pit.port_write(PIT_CONTROL, 1, 0xB1);
        pit.port_write(PIT_CH2_DATA, 1, 0x05);
        pit.port_write(PIT_CH2_DATA, 1, 0x00); // BCD 5
        assert!(pit.channel2().bcd);
        assert_eq!(pit.channel2().ce, 5);
        assert!(!pit.tick_ch2(4));
        // Read-back status CNT2: SC=11 COUNT=1 STATUS=0 CNT2=1 → 0xE8
        pit.port_write(PIT_CONTROL, 1, 0xE8);
        let status = pit.port_read(PIT_CH2_DATA, 1) as u8;
        // OUT=0, NULL_COUNT=0, RW=11, M=000, BCD=1 → 0x31
        assert_eq!(status, 0x31);
        assert!(pit.tick_ch2(1));
        assert!(pit.out_ch2());

        pit.reset();
        assert!(!pit.channel2().bcd);
        assert_eq!(pit.channel2().ce, 0);
        assert!(!pit.channel2().counting);
    }

    /// Spec: Intel 8254 Read-Back Command (SC=11) — STATUS=0 latches status;
    /// first data-port read returns status: OUT | NULL_COUNT | RW | M | BCD
    /// (datasheet Figure 11 / status byte).
    #[test]
    fn read_back_status_latch_ch0() {
        let mut pit = Pit8254::new();
        // ch0, lohi, mode 3, binary → control 0x36; OUT high after CW, null count set.
        pit.port_write(PIT_CONTROL, 1, 0x36);
        // Read-back status only for CNT0: SC=11 COUNT=1 STATUS=0 CNT0=1 → 0xE2.
        pit.port_write(PIT_CONTROL, 1, 0xE2);
        let status = pit.port_read(PIT_CH0_DATA, 1) as u8;
        // OUT=1, NULL_COUNT=1, RW=11, M=011, BCD=0 → 0xF6.
        assert_eq!(status, 0xF6);
        assert_eq!(pit.channel0().mode, 3);
        assert!(!pit.channel0().count_loaded);

        // After count load, NULL_COUNT clears; OUT still high (mode 3).
        pit.port_write(PIT_CH0_DATA, 1, 0x00);
        pit.port_write(PIT_CH0_DATA, 1, 0x10);
        pit.port_write(PIT_CONTROL, 1, 0xE2);
        let status = pit.port_read(PIT_CH0_DATA, 1) as u8;
        // OUT=1, NULL_COUNT=0, RW/M/BCD from 0x36 → 0xB6.
        assert_eq!(status, 0xB6);
        assert!(pit.out_ch0());
    }

    /// Spec: Intel 8254 Read-Back — COUNT=0 latches CE like a counter-latch;
    /// subsequent unread latches of the same OL are ignored.
    #[test]
    fn read_back_count_latch_ch0() {
        let mut pit = Pit8254::new();
        pit.port_write(PIT_CONTROL, 1, 0x36); // ch0 lohi mode 3
        pit.port_write(PIT_CH0_DATA, 1, 0x00);
        pit.port_write(PIT_CH0_DATA, 1, 0x10); // count = 0x1000
        assert!(pit.channel0().counting);
        // Drive CE down a few clocks without changing programmed count.
        let _ = pit.tick_ch0(3);
        let ce_now = pit.channel0().ce;
        assert!(ce_now > 0 && ce_now < 0x1000);

        // Read-back count only CNT0: SC=11 COUNT=0 STATUS=1 CNT0=1 → 0xD2.
        pit.port_write(PIT_CONTROL, 1, 0xD2);
        let _ = pit.tick_ch0(5); // live CE advances; latched OL must not.
        let lo = pit.port_read(PIT_CH0_DATA, 1) as u8;
        let hi = pit.port_read(PIT_CH0_DATA, 1) as u8;
        assert_eq!(u16::from(lo) | (u16::from(hi) << 8), ce_now as u16);

        // Second unread count latch ignored: latch again, advance, re-latch → first OL kept.
        pit.port_write(PIT_CONTROL, 1, 0xD2);
        let first = pit.channel0().ce;
        let _ = pit.tick_ch0(7);
        pit.port_write(PIT_CONTROL, 1, 0xD2); // ignored while unread
        let lo = pit.port_read(PIT_CH0_DATA, 1) as u8;
        let hi = pit.port_read(PIT_CH0_DATA, 1) as u8;
        assert_eq!(u16::from(lo) | (u16::from(hi) << 8), first as u16);
    }

    /// Spec: Intel 8254 — when both count and status are latched, the first
    /// counter read returns status; the next one/two reads return the count.
    #[test]
    fn read_back_status_then_count_order() {
        let mut pit = Pit8254::new();
        pit.port_write(PIT_CONTROL, 1, 0x30); // ch0 lohi mode 0 (OUT low after CW)
        pit.port_write(PIT_CH0_DATA, 1, 0x04);
        pit.port_write(PIT_CH0_DATA, 1, 0x00); // count = 4; counting, OUT still low
        assert!(pit.channel0().counting);
        assert!(!pit.out_ch0());

        // Both count+status CNT0: SC=11 COUNT=0 STATUS=0 CNT0=1 → 0xC2.
        pit.port_write(PIT_CONTROL, 1, 0xC2);
        let status = pit.port_read(PIT_CH0_DATA, 1) as u8;
        // OUT=0, NULL_COUNT=0, RW=11, M=000, BCD=0 → 0x30.
        assert_eq!(status, 0x30);
        let lo = pit.port_read(PIT_CH0_DATA, 1) as u8;
        let hi = pit.port_read(PIT_CH0_DATA, 1) as u8;
        assert_eq!(u16::from(lo) | (u16::from(hi) << 8), 4);
    }

    /// Spec: Intel 8254 Read-Back may select multiple counters in one command.
    #[test]
    fn read_back_multi_channel_status() {
        let mut pit = Pit8254::new();
        pit.port_write(PIT_CONTROL, 1, 0x36); // ch0 mode 3 lohi
        pit.port_write(PIT_CH0_DATA, 1, 0xFF);
        pit.port_write(PIT_CH0_DATA, 1, 0xFF);
        pit.port_write(PIT_CONTROL, 1, 0xB0); // ch2 mode 0 lohi
                                              // Status of CNT2+CNT0: SC=11 COUNT=1 STATUS=0 CNT2=1 CNT0=1 → 0xEA.
        pit.port_write(PIT_CONTROL, 1, 0xEA);
        let st0 = pit.port_read(PIT_CH0_DATA, 1) as u8;
        let st2 = pit.port_read(PIT_CH2_DATA, 1) as u8;
        // ch0: OUT=1 NULL=0 + 0x36 → 0xB6; ch2: OUT=0 NULL=1 + 0x30 → 0x70.
        assert_eq!(st0, 0xB6);
        assert_eq!(st2, 0x70);
        assert_eq!(pit.channel0().mode, 3);
        assert_eq!(pit.channel2().mode, 0);
    }

    /// Spec: Read-Back must not disturb programmed mode / access / reload.
    #[test]
    fn read_back_preserves_mode_programming() {
        let mut pit = Pit8254::new();
        pit.port_write(PIT_CONTROL, 1, 0x34); // ch0 lohi mode 2
        pit.port_write(PIT_CH0_DATA, 1, 0x10);
        pit.port_write(PIT_CH0_DATA, 1, 0x00);
        let mode = pit.channel0().mode;
        let count = pit.channel0().count;
        let cw = pit.channel0().control_word;
        pit.port_write(PIT_CONTROL, 1, 0xC2); // latch both on ch0
        let _ = pit.port_read(PIT_CH0_DATA, 1); // status
        let _ = pit.port_read(PIT_CH0_DATA, 1); // count lo
        let _ = pit.port_read(PIT_CH0_DATA, 1); // count hi
        assert_eq!(pit.channel0().mode, mode);
        assert_eq!(pit.channel0().count, count);
        assert_eq!(pit.channel0().control_word, cw);
        assert!(pit.channel0().counting);
    }

    #[test]
    fn channel1_accept_but_undocumented_as_full() {
        // Honest stub: ch1 control + data accepted; no DRAM-refresh claims.
        let mut pit = Pit8254::new();
        pit.port_write(PIT_CONTROL, 1, 0x76); // ch1 mode 3 lohi
        pit.port_write(PIT_CH1_DATA, 1, 0x01);
        pit.port_write(PIT_CH1_DATA, 1, 0x00);
        assert!(pit.channels[1].count_loaded);
        assert_eq!(pit.channels[1].count, 0x0001);
    }

    /// Spec: IBM PC/AT port 0x61 — bit0 GATE2, bit1 speaker data, bit5 OUT2.
    #[test]
    fn port61_gate_speaker_and_out2_readback() {
        let mut pit = Pit8254::new();
        assert_eq!(pit.port61_read() & (PORT61_GATE2 | PORT61_SPKR_DATA), 0);
        assert!(!pit.channel2().gate);

        // Program ch2 mode 0, count=3; GATE still low → not counting.
        pit.port_write(PIT_CONTROL, 1, 0xB0); // ch2 lohi mode 0
        pit.port_write(PIT_CH2_DATA, 1, 0x03);
        pit.port_write(PIT_CH2_DATA, 1, 0x00);
        assert!(!pit.channel2().counting);
        assert!(!pit.out_ch2());
        assert!(!pit.tick_ch2(10));
        assert!(!pit.out_ch2());

        // Enable GATE2 + speaker data.
        pit.port61_write(PORT61_GATE2 | PORT61_SPKR_DATA);
        assert!(pit.speaker_data_enabled());
        assert!(pit.channel2().gate);
        assert!(pit.channel2().counting);
        assert_eq!(pit.port61_read() & (PORT61_GATE2 | PORT61_SPKR_DATA), 0x03);

        assert!(!pit.tick_ch2(2));
        assert!(!pit.out_ch2());
        assert!(pit.tick_ch2(1)); // 3rd clock → OUT rising
        assert!(pit.out_ch2());
        assert_ne!(pit.port61_read() & PORT61_OUT2, 0);

        // GATE low pauses; OUT2 stays high (mode 0).
        pit.port61_write(PORT61_SPKR_DATA);
        assert!(!pit.channel2().gate);
        assert!(pit.out_ch2());
        assert_ne!(pit.port61_read() & PORT61_OUT2, 0);
        assert_eq!(pit.port61_read() & PORT61_GATE2, 0);
    }

    /// Spec: Intel 8254 mode 2 — GATE low forces OUT high and stops counting.
    #[test]
    fn ch2_mode2_gate_low_forces_out_high() {
        let mut pit = Pit8254::new();
        pit.port61_write(PORT61_GATE2);
        pit.port_write(PIT_CONTROL, 1, 0xB4); // ch2 lohi mode 2
        pit.port_write(PIT_CH2_DATA, 1, 0x04);
        pit.port_write(PIT_CH2_DATA, 1, 0x00);
        assert!(pit.out_ch2());
        assert!(pit.channel2().counting);

        // Advance near terminal so OUT may go low.
        let _ = pit.tick_ch2(4);
        pit.port61_write(0); // GATE low
        assert!(!pit.channel2().gate);
        assert!(pit.out_ch2()); // forced high
        assert!(!pit.tick_ch2(10));
    }

    /// Spec: Intel 8254 datasheet "Mode Definitions" — Mode 4 (Software
    /// Triggered Strobe): OUT is high after the control word; writing the count
    /// starts counting; OUT is low for one CLK at terminal count then high.
    #[test]
    fn mode4_strobe_pulses_out_low_for_one_clock() {
        let mut pit = Pit8254::new();
        pit.port_write(PIT_CONTROL, 1, 0x38); // ch0 lohi mode 4
        assert_eq!(pit.channel0().mode, 4);
        assert!(pit.out_ch0()); // mode 4: OUT high after CW
        pit.port_write(PIT_CH0_DATA, 1, 0x05);
        pit.port_write(PIT_CH0_DATA, 1, 0x00); // count = 5 → starts counting
        assert!(pit.channel0().counting);
        assert!(pit.out_ch0());

        // N-1 clocks: OUT stays high, no edges.
        for _ in 0..4 {
            assert!(!pit.tick_ch0(1));
            assert!(pit.out_ch0());
        }
        // Terminal count: OUT low for exactly one model clock (no rise yet).
        assert!(!pit.tick_ch0(1));
        assert!(!pit.out_ch0());
        // Strobe ends: OUT rises, reported exactly once.
        assert!(pit.tick_ch0(1));
        assert!(pit.out_ch0());
        assert!(!pit.tick_ch0(20));
        assert!(pit.out_ch0());
    }

    /// Spec: Intel 8254 Mode 4 — one-shot: no further strobes until a new count
    /// is written (writing a new count re-arms without a new control word).
    #[test]
    fn mode4_one_shot_rearms_on_new_count() {
        let mut pit = Pit8254::new();
        pit.port_write(PIT_CONTROL, 1, 0x38); // ch0 lohi mode 4
        pit.port_write(PIT_CH0_DATA, 1, 0x02);
        pit.port_write(PIT_CH0_DATA, 1, 0x00); // count = 2
        assert!(pit.tick_ch0(3)); // low on clock 2, rises on clock 3
        assert!(pit.out_ch0());
        assert!(!pit.channel0().counting);
        assert!(!pit.tick_ch0(10)); // one-shot: no repeat
        assert!(pit.out_ch0());

        pit.port_write(PIT_CH0_DATA, 1, 0x02);
        pit.port_write(PIT_CH0_DATA, 1, 0x00); // re-arm, count = 2
        assert!(pit.channel0().counting);
        assert!(!pit.tick_ch0(2));
        assert!(!pit.out_ch0());
        assert!(pit.tick_ch0(1));
        assert!(pit.out_ch0());
    }

    /// Spec: Intel 8254 GATE-pin operations summary — Mode 4: GATE low disables
    /// counting, GATE high enables it (OUT is unaffected by GATE).
    #[test]
    fn ch2_mode4_gate_low_disables_counting() {
        let mut pit = Pit8254::new();
        pit.port61_write(PORT61_GATE2);
        pit.port_write(PIT_CONTROL, 1, 0xB8); // ch2 lohi mode 4
        assert!(pit.out_ch2());
        pit.port_write(PIT_CH2_DATA, 1, 0x04);
        pit.port_write(PIT_CH2_DATA, 1, 0x00); // count = 4
        assert!(pit.channel2().counting);

        assert!(!pit.tick_ch2(2)); // CE 4 → 2
        assert_eq!(pit.channels[2].ce, 2);

        pit.port61_write(0); // GATE low → counting disabled
        assert!(!pit.tick_ch2(50));
        assert_eq!(pit.channels[2].ce, 2);
        assert!(pit.out_ch2()); // GATE does not move OUT in mode 4

        pit.port61_write(PORT61_GATE2); // GATE high → resume from CE (no reload)
        assert!(!pit.tick_ch2(1));
        assert!(!pit.tick_ch2(1)); // terminal count → OUT low one clock
        assert!(!pit.out_ch2());
        assert!(pit.tick_ch2(1));
        assert!(pit.out_ch2());
    }

    /// Spec: Intel 8254 "Mode Definitions" — Mode 5 (Hardware Triggered
    /// Strobe): OUT high after the control word; counting starts on the GATE
    /// rising edge (not on the count write); OUT low one CLK at terminal count.
    #[test]
    fn ch2_mode5_starts_on_gate_rising_edge() {
        let mut pit = Pit8254::new();
        pit.port_write(PIT_CONTROL, 1, 0xBA); // ch2 lohi mode 5 (GATE2 low at reset)
        assert_eq!(pit.channel2().mode, 5);
        assert!(pit.out_ch2()); // mode 5: OUT high after CW
        pit.port_write(PIT_CH2_DATA, 1, 0x03);
        pit.port_write(PIT_CH2_DATA, 1, 0x00); // count = 3 — not a trigger
        assert!(!pit.channel2().counting);
        assert!(!pit.tick_ch2(10));
        assert!(pit.out_ch2());

        pit.port61_write(PORT61_GATE2); // GATE rising edge triggers
        assert!(pit.channel2().counting);
        assert!(pit.out_ch2());
        assert!(!pit.tick_ch2(2));
        assert!(pit.out_ch2());
        assert!(!pit.tick_ch2(1)); // terminal count → OUT low one clock
        assert!(!pit.out_ch2());
        assert!(pit.tick_ch2(1)); // strobe ends → single rising edge
        assert!(pit.out_ch2());
        assert!(!pit.tick_ch2(10)); // needs a new trigger
    }

    /// Spec: Intel 8254 Mode 5 — retriggerable: a GATE rising edge reloads the
    /// counter; GATE low alone does not disable counting.
    #[test]
    fn ch2_mode5_gate_rising_edge_retriggers() {
        let mut pit = Pit8254::new();
        pit.port_write(PIT_CONTROL, 1, 0xBA); // ch2 lohi mode 5
        pit.port_write(PIT_CH2_DATA, 1, 0x04);
        pit.port_write(PIT_CH2_DATA, 1, 0x00); // count = 4
        pit.port61_write(PORT61_GATE2);
        assert!(!pit.tick_ch2(2)); // CE 4 → 2
        assert_eq!(pit.channels[2].ce, 2);

        pit.port61_write(0); // mode 5: GATE low does not disable counting
        assert!(!pit.tick_ch2(1));
        assert_eq!(pit.channels[2].ce, 1);

        pit.port61_write(PORT61_GATE2); // retrigger → full count again
        assert_eq!(pit.channels[2].ce, 4);
        assert!(!pit.tick_ch2(3)); // would have strobed at CE 1 without reload
        assert!(pit.out_ch2());
        assert!(!pit.tick_ch2(1)); // terminal count → OUT low
        assert!(!pit.out_ch2());
        assert!(pit.tick_ch2(1));
        assert!(pit.out_ch2());
    }

    /// Spec: Intel 8254 "Mode Definitions" — Mode 1 (Hardware Retriggerable
    /// One-Shot): OUT high after the control word, low on the GATE trigger,
    /// low until terminal count, then high.
    #[test]
    fn ch2_mode1_one_shot_low_until_terminal_count() {
        let mut pit = Pit8254::new();
        pit.port_write(PIT_CONTROL, 1, 0xB2); // ch2 lohi mode 1
        assert_eq!(pit.channel2().mode, 1);
        assert!(pit.out_ch2()); // mode 1: OUT high after CW
        pit.port_write(PIT_CH2_DATA, 1, 0x03);
        pit.port_write(PIT_CH2_DATA, 1, 0x00); // count = 3 — not a trigger
        assert!(!pit.channel2().counting);
        assert!(!pit.tick_ch2(10));
        assert!(pit.out_ch2());

        pit.port61_write(PORT61_GATE2); // trigger → OUT low
        assert!(pit.channel2().counting);
        assert!(!pit.out_ch2());
        assert!(!pit.tick_ch2(2));
        assert!(!pit.out_ch2());
        assert!(pit.tick_ch2(1)); // terminal count → single rising edge
        assert!(pit.out_ch2());
        assert!(!pit.tick_ch2(10));
        assert!(pit.out_ch2());
    }

    /// Spec: Intel 8254 Mode 1 — retriggerable: a GATE rising edge while OUT is
    /// low reloads the counter and restarts the low period.
    #[test]
    fn ch2_mode1_gate_rising_edge_restarts_one_shot() {
        let mut pit = Pit8254::new();
        pit.port_write(PIT_CONTROL, 1, 0xB2); // ch2 lohi mode 1
        pit.port_write(PIT_CH2_DATA, 1, 0x04);
        pit.port_write(PIT_CH2_DATA, 1, 0x00); // count = 4
        pit.port61_write(PORT61_GATE2);
        assert!(!pit.tick_ch2(2)); // CE 4 → 2, OUT still low
        assert!(!pit.out_ch2());

        pit.port61_write(0);
        pit.port61_write(PORT61_GATE2); // retrigger mid one-shot
        assert!(!pit.out_ch2());
        assert_eq!(pit.channels[2].ce, 4);
        assert!(!pit.tick_ch2(3)); // full count from the retrigger
        assert!(!pit.out_ch2());
        assert!(pit.tick_ch2(1));
        assert!(pit.out_ch2());
    }

    /// Spec: Intel 8254 Mode 1 — writing a new count does not affect the
    /// in-progress one-shot; it is loaded on the next trigger.
    #[test]
    fn ch2_mode1_new_count_applies_on_next_trigger() {
        let mut pit = Pit8254::new();
        pit.port_write(PIT_CONTROL, 1, 0xB2); // ch2 lohi mode 1
        pit.port_write(PIT_CH2_DATA, 1, 0x02);
        pit.port_write(PIT_CH2_DATA, 1, 0x00); // count = 2
        pit.port61_write(PORT61_GATE2); // trigger → CE = 2

        pit.port_write(PIT_CH2_DATA, 1, 0x05);
        pit.port_write(PIT_CH2_DATA, 1, 0x00); // new count = 5 mid one-shot
        assert_eq!(pit.channels[2].ce, 2);
        assert!(!pit.out_ch2());
        assert!(!pit.tick_ch2(1));
        assert!(pit.tick_ch2(1)); // original count of 2 finishes the one-shot
        assert!(pit.out_ch2());

        pit.port61_write(0);
        pit.port61_write(PORT61_GATE2); // next trigger loads the new count
        assert_eq!(pit.channels[2].ce, 5);
        assert!(!pit.out_ch2());
        assert!(!pit.tick_ch2(4));
        assert!(pit.tick_ch2(1));
        assert!(pit.out_ch2());
    }

    #[test]
    fn reset_clears_port61_and_ch2_gate() {
        let mut pit = Pit8254::new();
        pit.port61_write(PORT61_GATE2 | PORT61_SPKR_DATA);
        pit.port_write(PIT_CONTROL, 1, 0xB6);
        pit.port_write(PIT_CH2_DATA, 1, 0x10);
        pit.port_write(PIT_CH2_DATA, 1, 0x00);
        pit.reset();
        assert_eq!(pit, Pit8254::new());
        assert!(!pit.channel2().gate);
        assert_eq!(pit.port61_read(), 0);
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
