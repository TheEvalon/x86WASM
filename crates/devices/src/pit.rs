//! Intel 8254-compatible Programmable Interval Timer — channel 0 programming.
//!
//! Classic PC ports: counters `0x40`/`0x41`/`0x42`, control word `0x43`.
//!
//! # Spec refs
//!
//! - Intel 8254 Programmable Interval Timer datasheet — control word format
//!   (SC/RW/M/BCD), counter latch command, LSB/MSB / LSB-then-MSB access,
//!   operating modes 0–5.
//! - Classic IBM PC/AT I/O map: `0x40`–`0x43`.
//! - `docs/machine-model-pc-v1.md`, `docs/sources.md`, `plan.md` §15.3 / §21.
//!
//! # Scope (this slice)
//!
//! Channel 0 control-word programming (modes 0, 2, 3 required; other mode bits
//! stored), access-mode count load, and counter-latch read-back. Channels 1/2
//! accept control words and byte I/O but are **not** claimed as fully supported.
//!
//! # Unsupported (explicit)
//!
//! - IRQ0 pulse / wiring to PIC / `MachineBus` / `poll_external_irq`
//! - Gate input, OUT pin sampling beyond stored count
//! - Read-back command (`SC=11`) status/count latches (ignored)
//! - Channel 1 DRAM refresh and channel 2 PC speaker semantics
//! - Guest-time tick advancement driving interrupts

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
    /// 16-bit counter / divisor (0 means 65536 counts per datasheet convention).
    pub count: u16,
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

    fn apply_control(&mut self, value: u8) {
        let rw = (value >> CW_RW_SHIFT) & CW_RW_MASK;
        if rw == RW_LATCH {
            // Counter latch: freeze current count for subsequent reads.
            self.latched = Some(self.count);
            self.read_phase = BytePhase::ExpectLo;
            return;
        }

        self.control_word = value;
        self.access = AccessMode::from_rw(rw);
        self.mode = Self::decode_mode((value >> CW_MODE_SHIFT) & CW_MODE_MASK);
        self.bcd = value & CW_BCD != 0;
        self.count_loaded = false;
        self.latched = None;
        self.write_phase = match self.access {
            AccessMode::Hi => BytePhase::ExpectHi,
            _ => BytePhase::ExpectLo,
        };
        self.read_phase = match self.access {
            AccessMode::Hi => BytePhase::ExpectHi,
            _ => BytePhase::ExpectLo,
        };
    }

    fn write_data(&mut self, value: u8) {
        match self.access {
            AccessMode::Latch => {
                // Latched state does not accept count programming until a new CW.
            }
            AccessMode::Lo => {
                self.count = (self.count & 0xFF00) | u16::from(value);
                self.write_phase = BytePhase::Complete;
                self.count_loaded = true;
            }
            AccessMode::Hi => {
                self.count = (self.count & 0x00FF) | (u16::from(value) << 8);
                self.write_phase = BytePhase::Complete;
                self.count_loaded = true;
            }
            AccessMode::LoHi => match self.write_phase {
                BytePhase::ExpectLo | BytePhase::Complete => {
                    self.count = (self.count & 0xFF00) | u16::from(value);
                    self.write_phase = BytePhase::ExpectHi;
                    self.count_loaded = false;
                }
                BytePhase::ExpectHi => {
                    self.count = (self.count & 0x00FF) | (u16::from(value) << 8);
                    self.write_phase = BytePhase::Complete;
                    self.count_loaded = true;
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
}

/// 8254 PIT with three channels; channel 0 is the supported programming surface.
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

        let mut pit2 = Pit8254::new();
        pit2.port_write(PIT_CONTROL, 1, 0x36);
        pit2.port_write(PIT_CH0_DATA, 1, 0x00);
        pit2.port_write(PIT_CH0_DATA, 1, 0x10);
        pit2.reset();
        assert_eq!(pit2, Pit8254::new());
    }

    /// Spec: channel 0 modes 0 / 2 / 3 via control word M field.
    #[test]
    fn channel0_modes_0_2_3() {
        let mut pit = Pit8254::new();

        // SC=0, RW=11 (lohi), M=000 (mode 0), BCD=0 → 0x30
        pit.port_write(PIT_CONTROL, 1, 0x30);
        assert_eq!(pit.channel0().mode, 0);
        assert_eq!(pit.channel0().control_word, 0x30);

        // Mode 2 rate generator: M bits 010 → 0x34
        pit.port_write(PIT_CONTROL, 1, 0x34);
        assert_eq!(pit.channel0().mode, 2);

        // Mode 3 square wave: M bits 011 → 0x36 (classic PC IRQ0)
        pit.port_write(PIT_CONTROL, 1, 0x36);
        assert_eq!(pit.channel0().mode, 3);
        assert!(!pit.channel0().bcd);
    }

    /// Spec: LSB then MSB load of 16-bit count (access mode 11b).
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
    }

    /// Spec: RW=00 counter latch; subsequent reads return latched value.
    #[test]
    fn channel0_latch_read() {
        let mut pit = Pit8254::new();
        pit.port_write(PIT_CONTROL, 1, 0x36);
        pit.port_write(PIT_CH0_DATA, 1, 0x78);
        pit.port_write(PIT_CH0_DATA, 1, 0x56);
        assert_eq!(pit.channel0().count, 0x5678);

        // Latch command: SC=0, RW=00 → 0x00
        pit.port_write(PIT_CONTROL, 1, 0x00);
        // Mutate live count without a new full program — latch must stay.
        pit.channels[0].count = 0xABCD;

        let lo = pit.port_read(PIT_CH0_DATA, 1) as u8;
        let hi = pit.port_read(PIT_CH0_DATA, 1) as u8;
        assert_eq!(u16::from(lo) | (u16::from(hi) << 8), 0x5678);
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
