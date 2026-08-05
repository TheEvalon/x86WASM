//! MC146818-compatible CMOS / RTC register file (ports `0x70` / `0x71`).
//!
//! # Spec refs
//!
//! - Motorola MC146818A Real Time Clock Plus RAM datasheet — address/data
//!   multiplexing, register map 0x00–0x0D (time + calendar + status A–D),
//!   "Time, Calendar, and Alarm Locations", update-cycle time/calendar
//!   increment with automatic leap-year compensation, status A UIP, status B
//!   SET/DM/24-12 + PIE/AIE/UIE, status C PF/AF/UF/IRQF (read-to-clear), IRQ pin.
//! - IBM PC/AT Technical Reference — CMOS index port `0x70` (bit7 = NMI mask),
//!   data port `0x71`; RTC IRQ → ISA IRQ8 (8259A slave IR0); BCD century at
//!   index `0x32` (later standardized as the ACPI FADT `CENTURY` index byte);
//!   CMOS map index `0x0F` shutdown status / reset code (ordinary CMOS RAM,
//!   not MC146818 status A–D; see [`REG_SHUTDOWN`]).
//! - Ralf Brown's Interrupt List — CMOS 0Fh "Shutdown Status Byte" / reset-code
//!   values used by POST after CPU reset (SeaBIOS soft-reset).
//! - `docs/machine-model-pc-v1.md`, `plan.md` §15.3 RTC.
//!
//! # Scope (this slice)
//!
//! 128-byte register bank with index/data port access, NMI-mask bit tracking
//! (port `0x70` bit7), status B PIE/AIE/UIE subset, model `tick` that sets
//! PF/UF (and AF on alarm match), IRQF → IRQ line for MachineBus → DualPic
//! IRQ8, plus a `tick_second` update cycle (Status A UIP approximate high window
//! of [`UIP_WINDOW_PERIODS`] `tick` periods + the full calendar cascade: seconds →
//! minutes → hours → date of month → month → year → century `0x32`, with
//! day-of-week 1–7 and Gregorian leap years). Status B `DM`
//! (bit 2) selects BCD (`DM=0`, reset default) or binary (`DM=1`) encoding for
//! the time/calendar registers during that cascade; Status B `24/12` (bit 1)
//! selects 24-hour hours `0–23` (set; reset default) or 12-hour hours `1–12`
//! with AM/PM in bit7 of the hours byte (clear); toggling that bit converts
//! the current hours and hours-alarm encodings (don't-care alarm `C0h`–`FFh`
//! unchanged). Alarm registers (`0x01` / `0x03` / `0x05`) match by byte
//! equality against current time (works for BCD, binary, and 12-hour AM/PM
//! bit7); values `C0h`–`FFh` are don't-care.
//! AF sets on match regardless of AIE; AIE gates IRQF only.
//! Index-port bit7 is readable/writable; [`CmosRtc::nmi_masked`] and
//! `Machine::nmi_delivery_enabled` / `Machine::inject_nmi` gate CPU `#NMI`.
//! IBM PC/AT CMOS index [`REG_SHUTDOWN`] (`0x0F`) is ordinary R/W CMOS RAM
//! (store/readback) used as the POST shutdown / reset code; SeaBIOS writes it
//! before a soft CPU reset. This model preserves that byte across
//! [`CmosRtc::reset`] (battery-backed); other general CMOS config bytes are
//! still cleared by model reset.
//!
//! # Model note: invalid calendar state
//!
//! Reset leaves the time/calendar registers zeroed, so month `0x00` / date
//! `0x00` / weekday `0x00` are reachable but are not valid dates. The cascade is
//! total and never panics or wraps arithmetically: see [`FALLBACK_MONTH_DAYS`]
//! for the documented fallback month length and resynchronization rules.
//! Silicon treats changing `24/12` without reinitializing the hour locations as
//! undefined; this model **converts** the current hours and hours-alarm bytes
//! between 24-hour (`0–23`) and 12-hour (`1–12` + AM/PM bit7) encodings when
//! Status B bit 1 flips (respecting `DM` BCD/binary; alarm don't-care `C0h`–`FFh`
//! is left unchanged). Cascade increments use the format selected by the bit.
//!
//! # Unsupported (explicit)
//!
//! - Host wall-clock sync / NTP-style host time
//! - Full NMI nesting / SMRAM/SMI / post-delivery NMI blocking window
//!   (`Machine::inject_nmi` + interpreter vector-2 stub covers the pin path)
//! - Exact crystal divider / UIP pulse timing (approximate
//!   [`UIP_WINDOW_PERIODS`]-tick hold after `tick_second` only; not µs-accurate)
//! - ACPI extended CMOS beyond 128 bytes
//! - Square-wave output (SQWE)
//! - Full battery-backed preserve of all non-volatile CMOS (only shutdown
//!   `0x0F` survives [`CmosRtc::reset`] today; POST action on the code is
//!   firmware/Machine, not this device)

use crate::PortDevice;

/// CMOS index / NMI-mask port (classic PC).
pub const CMOS_INDEX: u16 = 0x70;
/// CMOS data port (classic PC).
pub const CMOS_DATA: u16 = 0x71;

/// Status Register A.
pub const REG_STATUS_A: u8 = 0x0A;
/// Status Register B.
pub const REG_STATUS_B: u8 = 0x0B;
/// Status Register C (IRQ flags; read-to-clear on real hardware).
pub const REG_STATUS_C: u8 = 0x0C;
/// Status Register D (valid RAM / battery).
pub const REG_STATUS_D: u8 = 0x0D;

/// IBM PC/AT CMOS shutdown status / reset code (index `0x0F`).
///
/// Spec: IBM PC/AT Technical Reference CMOS map + RBIL CMOS 0Fh — ordinary
/// battery CMOS RAM (not MC146818 status A–D). BIOS POST reads this byte after
/// CPU reset to choose cold POST vs resume (SeaBIOS soft-reset writes
/// [`SHUTDOWN_JMP`] / similar before pulse-reset). Store/readback via
/// `0x70`/`0x71`; preserved across [`CmosRtc::reset`].
///
/// Documented reset-code values (RBIL Table C006 / PC AT):
/// - `00h` — soft reset or unexpected shutdown ([`SHUTDOWN_SOFT_OR_UNEXPECTED`])
/// - `01h` — after memory size check
/// - `02h` — after successful memory test
/// - `03h` — after failed memory test
/// - `04h` — INT 19h reboot ([`SHUTDOWN_INT19`])
/// - `05h` — EOI keyboard + jump via BDA `40:67` ([`SHUTDOWN_JMP_WITH_EOI`])
/// - `06h`–`08h` — protected-mode test / POST return paths
/// - `09h` — INT 15h/87h block move ([`SHUTDOWN_BLOCK_MOVE`])
/// - `0Ah` — jump via BDA `40:67` ([`SHUTDOWN_JMP`]; common SeaBIOS soft-reset)
/// - `0Bh` / `0Ch` — resume via IRET / RETF via `40:67`
/// - other — treated as power-on reset by classic POST
pub const REG_SHUTDOWN: u8 = 0x0F;

/// Shutdown status `00h`: soft reset or unexpected shutdown.
pub const SHUTDOWN_SOFT_OR_UNEXPECTED: u8 = 0x00;
/// Shutdown status `04h`: INT 19h reboot request.
pub const SHUTDOWN_INT19: u8 = 0x04;
/// Shutdown status `05h`: flush keyboard (EOI) and jump via BDA `40:67`.
pub const SHUTDOWN_JMP_WITH_EOI: u8 = 0x05;
/// Shutdown status `09h`: INT 15h/87h block-move return.
pub const SHUTDOWN_BLOCK_MOVE: u8 = 0x09;
/// Shutdown status `0Ah`: jump via BDA `40:67` (SeaBIOS soft-reset code).
pub const SHUTDOWN_JMP: u8 = 0x0A;

/// Seconds / minutes / hours (time).
const REG_SEC: u8 = 0x00;
const REG_MIN: u8 = 0x02;
const REG_HOUR: u8 = 0x04;
/// Alarm seconds / minutes / hours.
const REG_SEC_ALARM: u8 = 0x01;
const REG_MIN_ALARM: u8 = 0x03;
const REG_HOUR_ALARM: u8 = 0x05;
/// Calendar: day of week (1 = Sunday), day of month, month, year.
const REG_DAY_OF_WEEK: u8 = 0x06;
const REG_DAY_OF_MONTH: u8 = 0x07;
const REG_MONTH: u8 = 0x08;
const REG_YEAR: u8 = 0x09;
/// Century register.
///
/// Spec: not part of the base MC146818 register file (0x00–0x0D). PC/AT CMOS
/// convention places the BCD century at index `0x32`, which ACPI later
/// standardized as the FADT `CENTURY` index byte.
const REG_CENTURY: u8 = 0x32;

/// Index port bit7: NMI disable when set (PC/AT).
const NMI_DISABLE: u8 = 1 << 7;
const INDEX_MASK: u8 = 0x7F;

/// Status A: UIP (Update In Progress) — hardware-driven, read-only to guest.
/// Spec: MC146818 Status Register A bit7.
pub const STATUS_A_UIP: u8 = 1 << 7;

/// Approximate Status A UIP high window after [`CmosRtc::tick_second`], in model
/// periodic quanta ([`CmosRtc::tick`] periods).
///
/// Spec (MC146818): UIP rises ~244 µs before the update and stays high for the
/// ~1984 µs update cycle. This model sets UIP at the start of `tick_second`,
/// advances the calendar and latches UF immediately, then leaves UIP set for
/// this many subsequent `tick` periods before clearing it — an order-of-magnitude
/// match at the common RS=0110 / 1024 Hz rate (~976 µs/period), not an exact
/// crystal-timed pulse.
pub const UIP_WINDOW_PERIODS: u8 = 2;

/// Status B: SET (inhibit update), PIE, AIE, UIE, DM (binary data mode), 24/12.
///
/// Spec: MC146818 Status Register B — bit 2 (`DM`) selects calendar encoding:
/// 0 = BCD (reset default), 1 = binary. Bit 1 (`24/12`) selects hour format:
/// 1 = 24-hour `0–23` (reset default), 0 = 12-hour `1–12` with AM/PM.
/// See also OSDev CMOS / IBM PC AT RTC.
pub const STB_SET: u8 = 1 << 7;
pub const STB_PIE: u8 = 1 << 6;
pub const STB_AIE: u8 = 1 << 5;
pub const STB_UIE: u8 = 1 << 4;
pub const STB_DM: u8 = 1 << 2;
/// Status B bit 1: 1 = 24-hour mode, 0 = 12-hour mode.
pub const STB_24_12: u8 = 1 << 1;
/// Hours register bit7 in 12-hour mode: 1 = PM, 0 = AM.
///
/// Spec: MC146818 — "when the 12-hour format is selected the high order bit of
/// the hours byte represents PM when it is a 1".
pub const HOUR_PM: u8 = 1 << 7;

/// Status C: IRQF, PF, AF, UF (bits 3:0 reserved 0).
pub const STC_IRQF: u8 = 1 << 7;
pub const STC_PF: u8 = 1 << 6;
pub const STC_AF: u8 = 1 << 5;
pub const STC_UF: u8 = 1 << 4;

/// Default Status A: UIP=0, divider=010 (32.768 kHz), rate=0110 (1024 Hz).
/// Common AT POST default.
const DEFAULT_STATUS_A: u8 = 0x26;
/// Default Status B: 24-hour (`STB_24_12`); DM/binary cleared → BCD; PIE/AIE/UIE off.
const DEFAULT_STATUS_B: u8 = STB_24_12;
/// Default Status C: no IRQ flags pending.
const DEFAULT_STATUS_C: u8 = 0x00;
/// Default Status D: VRT=1 (valid RAM and time / battery OK).
const DEFAULT_STATUS_D: u8 = 0x80;

/// Status A RS field mask (bits 3:0); 0 = periodic interrupt disabled.
const STATUS_A_RS_MASK: u8 = 0x0F;

/// Day-of-week counter modulus (MC146818: 1 = Sunday … 7 = Saturday).
const DAYS_PER_WEEK: u8 = 7;
/// Month counter modulus (MC146818: 01 = January … 12 = December).
const MONTHS_PER_YEAR: u8 = 12;
/// Month length used when the month register does not hold BCD `0x01`–`0x12`.
///
/// Model note (not MC146818 spec): reset zeroes the time/calendar registers, so
/// month `0x00` and date `0x00` are reachable states that are not valid dates.
/// The update cycle stays total and non-panicking by treating an unrecognized
/// month as the longest possible month, so no valid date ever wraps early; a
/// date past that length wraps to 01 and steps the month, an unrecognized month
/// steps to January without a year carry, and an unrecognized day-of-week resets
/// to 1. Guests (SeaBIOS) program a valid date before relying on the cascade.
const FALLBACK_MONTH_DAYS: u8 = 31;

/// 128-byte CMOS/RTC image with index+data port access.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CmosRtc {
    /// Register file (0x00–0x7F).
    pub ram: [u8; 128],
    /// Last index written (low 7 bits); bit7 tracks NMI disable separately.
    index: u8,
    /// NMI-disable latch from index-port bit7 (PC/AT: 1 = NMI masked).
    ///
    /// Spec: IBM PC/AT — writing `0x70` bit7 disables NMI; this stub stores the
    /// bit and exposes it via [`Self::nmi_masked`] for `Machine::inject_nmi`.
    pub nmi_disabled: bool,
    /// Remaining [`CmosRtc::tick`] periods while Status A UIP stays high after an
    /// update cycle. Zero when UIP is clear (or held only by a test begin).
    uip_hold_periods: u8,
}

impl CmosRtc {
    pub fn new() -> Self {
        let mut s = Self {
            ram: [0; 128],
            index: 0,
            nmi_disabled: false,
            uip_hold_periods: 0,
        };
        s.apply_reset_defaults();
        s
    }

    fn apply_reset_defaults(&mut self) {
        // Spec: IBM PC/AT — CMOS `0x0F` is battery-backed; SeaBIOS soft-reset
        // relies on the shutdown code surviving CPU/device reset.
        let shutdown = self.ram[REG_SHUTDOWN as usize];
        self.ram = [0; 128];
        self.ram[REG_STATUS_A as usize] = DEFAULT_STATUS_A;
        self.ram[REG_STATUS_B as usize] = DEFAULT_STATUS_B;
        self.ram[REG_STATUS_C as usize] = DEFAULT_STATUS_C;
        self.ram[REG_STATUS_D as usize] = DEFAULT_STATUS_D;
        self.ram[REG_SHUTDOWN as usize] = shutdown;
        self.index = 0;
        self.nmi_disabled = false;
        self.uip_hold_periods = 0;
    }

    pub fn reset(&mut self) {
        self.apply_reset_defaults();
    }

    pub fn selected_index(&self) -> u8 {
        self.index & INDEX_MASK
    }

    /// True when port `0x70` bit7 last wrote NMI disable.
    ///
    /// Spec: IBM PC/AT Technical Reference — CMOS index bit7 masks NMI.
    pub fn nmi_masked(&self) -> bool {
        self.nmi_disabled
    }

    /// IBM PC/AT CMOS shutdown status byte (index [`REG_SHUTDOWN`] / `0x0F`).
    ///
    /// Spec: RBIL CMOS 0Fh — POST reset code. Ordinary R/W CMOS RAM; see
    /// [`REG_SHUTDOWN`] for value meanings. Exposed for a future Machine soft-
    /// reset path without requiring MachineBus port I/O.
    pub fn shutdown_status(&self) -> u8 {
        self.ram[REG_SHUTDOWN as usize]
    }

    pub fn read_reg(&self, index: u8) -> u8 {
        self.ram[(index & INDEX_MASK) as usize]
    }

    pub fn write_reg(&mut self, index: u8, value: u8) {
        let idx = index & INDEX_MASK;
        if idx == REG_STATUS_C {
            return;
        }
        if idx == REG_STATUS_B {
            let old_b = self.ram[REG_STATUS_B as usize];
            self.ram[REG_STATUS_B as usize] = value;
            // Spec: MC146818 — "The 24/12 bit cannot be changed without
            // reinitializing the hour locations." Silicon leaves conversion to
            // software; this model converts hours + hours-alarm so a Status B
            // toggle keeps a coherent wall time (DM BCD/binary respected;
            // alarm don't-care C0h–FFh unchanged).
            if (old_b ^ value) & STB_24_12 != 0 {
                let binary = value & STB_DM != 0;
                let to_24 = value & STB_24_12 != 0;
                self.ram[REG_HOUR as usize] =
                    convert_hour_format(self.ram[REG_HOUR as usize], binary, to_24);
                let alarm = self.ram[REG_HOUR_ALARM as usize];
                if alarm < 0xC0 {
                    self.ram[REG_HOUR_ALARM as usize] = convert_hour_format(alarm, binary, to_24);
                }
            }
            self.recompute_irqf();
            return;
        }
        self.ram[idx as usize] = Self::mask_status_a_write(idx, value, self.ram[idx as usize]);
    }

    /// Spec: Status A UIP (bit7) is read-only; guest writes must not sticky-set it.
    /// Preserve the current hardware UIP while accepting divider/RS bits.
    fn mask_status_a_write(idx: u8, value: u8, current: u8) -> u8 {
        if idx == REG_STATUS_A {
            (value & !STATUS_A_UIP) | (current & STATUS_A_UIP)
        } else {
            value
        }
    }

    /// RTC IRQ pin level (MC146818 IRQ); true when status C IRQF is set.
    ///
    /// Spec: IRQ is asserted while IRQF=1; reading status C clears flags / pin.
    pub fn irq_line(&self) -> bool {
        self.ram[REG_STATUS_C as usize] & STC_IRQF != 0
    }

    /// Advance `periods` model periodic quanta (not host-real-time).
    ///
    /// Spec (MC146818): when RS≠0, each period sets PF; when SET=0 each period
    /// also sets UF (update-ended colocated with the quantum — honesty note:
    /// calendar fields are not advanced here; use [`Self::tick_second`] for the
    /// UIP + calendar update cycle). Alarm match sets AF (don't-care `C0h`–`FFh`);
    /// AIE gates IRQF only. Each period also decays an outstanding UIP hold from
    /// [`Self::tick_second`] (see [`UIP_WINDOW_PERIODS`]).
    /// IRQF = (PF∧PIE) ∨ (AF∧AIE) ∨ (UF∧UIE). Returns true on IRQ pin rising edge.
    pub fn tick(&mut self, periods: u64) -> bool {
        if periods == 0 {
            return false;
        }
        let prev = self.irq_line();
        let status_a = self.ram[REG_STATUS_A as usize];
        let status_b = self.ram[REG_STATUS_B as usize];
        let rs = status_a & STATUS_A_RS_MASK;
        let set_inhibits_update = status_b & STB_SET != 0;

        for _ in 0..periods {
            if rs != 0 {
                self.ram[REG_STATUS_C as usize] |= STC_PF;
            }
            if !set_inhibits_update {
                self.ram[REG_STATUS_C as usize] |= STC_UF;
            }
            self.maybe_set_alarm_flag();
            self.decay_uip_window();
        }
        self.recompute_irqf();
        !prev && self.irq_line()
    }

    /// One second update cycle: UIP → calendar advance → UF → UIP hold window.
    ///
    /// Spec (MC146818): when Status B SET=0, the chip runs an update cycle each
    /// second; Status A UIP is set while the cycle runs and cleared when done;
    /// UF is set at update-ended. SET=1 inhibits the cycle (no UIP/UF/advance).
    /// Status B `DM` selects BCD (`0`) or binary (`1`) field encoding.
    ///
    /// Model: UIP remains set after return for [`UIP_WINDOW_PERIODS`] subsequent
    /// [`Self::tick`] periods (approximate pulse; not crystal-timed). UF and
    /// alarm match still latch at calendar update-ended inside this call.
    /// Returns true on IRQ pin rising edge (e.g. UIE∧UF).
    pub fn tick_second(&mut self) -> bool {
        if !self.begin_update_cycle() {
            return false;
        }
        self.advance_calendar();
        self.finish_update_cycle_with_uip_window()
    }

    /// Spec: MC146818 Status B bit 2 (`DM`) — 1 = binary calendar, 0 = BCD.
    fn data_mode_binary(&self) -> bool {
        self.ram[REG_STATUS_B as usize] & STB_DM != 0
    }

    /// Spec: MC146818 Status B bit 1 (`24/12`) — 1 = 24-hour, 0 = 12-hour.
    fn hour_mode_24(&self) -> bool {
        self.ram[REG_STATUS_B as usize] & STB_24_12 != 0
    }

    /// Observability helper: set UIP without finishing the cycle.
    ///
    /// Returns false when Status B SET inhibits the update (UIP left clear).
    pub fn begin_update_for_test(&mut self) -> bool {
        self.begin_update_cycle()
    }

    /// Observability helper: clear UIP, set UF, recompute IRQF (no calendar advance).
    pub fn end_update_for_test(&mut self) -> bool {
        self.finish_update_cycle()
    }

    fn begin_update_cycle(&mut self) -> bool {
        if self.ram[REG_STATUS_B as usize] & STB_SET != 0 {
            return false;
        }
        self.ram[REG_STATUS_A as usize] |= STATUS_A_UIP;
        true
    }

    /// Clear UIP immediately, set UF, recompute IRQF (test helper / abort path).
    fn finish_update_cycle(&mut self) -> bool {
        self.uip_hold_periods = 0;
        let prev = self.irq_line();
        self.ram[REG_STATUS_A as usize] &= !STATUS_A_UIP;
        self.ram[REG_STATUS_C as usize] |= STC_UF;
        self.maybe_set_alarm_flag();
        self.recompute_irqf();
        !prev && self.irq_line()
    }

    /// End of `tick_second`: latch UF/alarm while leaving UIP set for the
    /// approximate [`UIP_WINDOW_PERIODS`] hold (cleared by later [`Self::tick`]).
    fn finish_update_cycle_with_uip_window(&mut self) -> bool {
        let prev = self.irq_line();
        // UIP already set by `begin_update_cycle`.
        self.uip_hold_periods = UIP_WINDOW_PERIODS;
        self.ram[REG_STATUS_C as usize] |= STC_UF;
        self.maybe_set_alarm_flag();
        self.recompute_irqf();
        !prev && self.irq_line()
    }

    /// Decrement the post-`tick_second` UIP hold; clear Status A UIP at zero.
    fn decay_uip_window(&mut self) {
        if self.uip_hold_periods == 0 {
            return;
        }
        self.uip_hold_periods -= 1;
        if self.uip_hold_periods == 0 {
            self.ram[REG_STATUS_A as usize] &= !STATUS_A_UIP;
        }
    }

    /// Full time + calendar advance for one update cycle.
    ///
    /// Spec (MC146818, "Time, Calendar, and Alarm Locations" + update cycle;
    /// Status B `DM` / `24/12`): seconds 59→00 carry into minutes, minutes
    /// 59→00 into hours; hours advance in 24-hour (`0–23`) or 12-hour
    /// (`1–12` with AM/PM bit7) form per Status B bit 1 and carry into the
    /// date of month at midnight; the date rolls per month length (with
    /// automatic leap-year compensation for February), month 12→01 carries
    /// into the year, and the day-of-week counter advances on every date
    /// rollover. Field storage is BCD when `DM=0` and binary when `DM=1`.
    fn advance_calendar(&mut self) {
        let binary = self.data_mode_binary();
        let (sec, carry_min) = field_inc_mod(self.ram[REG_SEC as usize], 59, binary);
        self.ram[REG_SEC as usize] = sec;
        if !carry_min {
            return;
        }
        let (min, carry_hour) = field_inc_mod(self.ram[REG_MIN as usize], 59, binary);
        self.ram[REG_MIN as usize] = min;
        if !carry_hour {
            return;
        }
        let mode_24 = self.hour_mode_24();
        let carry_day = advance_hour_reg(&mut self.ram[REG_HOUR as usize], binary, mode_24);
        if !carry_day {
            return;
        }
        self.advance_day_of_week();
        self.advance_date();
    }

    /// Day-of-week counter (1 = Sunday … 7 = Saturday), wrapping 7 → 1.
    ///
    /// Spec: MC146818 day-of-week register 0x06 counts 1–7 independently of the
    /// date arithmetic. Any other stored value is not a valid weekday and is
    /// resynchronized to 1 (see [`FALLBACK_MONTH_DAYS`] model note). Encoding
    /// is the same in BCD and binary modes (values 1–7).
    fn advance_day_of_week(&mut self) {
        let dow = self.ram[REG_DAY_OF_WEEK as usize];
        self.ram[REG_DAY_OF_WEEK as usize] = if (1..DAYS_PER_WEEK).contains(&dow) {
            dow + 1
        } else {
            1
        };
    }

    /// Date of month, carrying into month/year/century at the end of the month.
    fn advance_date(&mut self) {
        let binary = self.data_mode_binary();
        let days_in_month = month_length(self.decode_field(REG_MONTH), self.full_year());
        // Day `0` (reset / invalid) still increments to 1 without a month carry —
        // same total fallback as the BCD path (see [`FALLBACK_MONTH_DAYS`]).
        let next = if binary {
            let day = self.ram[REG_DAY_OF_MONTH as usize];
            (day < days_in_month).then_some(day + 1)
        } else {
            bcd_to_bin(self.ram[REG_DAY_OF_MONTH as usize])
                .filter(|day| *day < days_in_month)
                .map(|day| day + 1)
        };
        match next {
            Some(day) => self.ram[REG_DAY_OF_MONTH as usize] = encode_field(day, binary),
            None => {
                self.ram[REG_DAY_OF_MONTH as usize] = encode_field(1, binary);
                self.advance_month();
            }
        }
    }

    /// Month 01–12, carrying into the year after December.
    ///
    /// An unrecognized month resets to January without a year carry.
    fn advance_month(&mut self) {
        let binary = self.data_mode_binary();
        let month = match self.decode_field(REG_MONTH) {
            month if (1..MONTHS_PER_YEAR).contains(&month) => month + 1,
            MONTHS_PER_YEAR => {
                self.advance_year();
                1
            }
            _ => 1,
        };
        self.ram[REG_MONTH as usize] = encode_field(month, binary);
    }

    /// Year 00–99, carrying into the century register (`0x32`).
    ///
    /// Spec: PC/AT CMOS convention, standardized by the ACPI FADT `CENTURY`
    /// index byte; the base MC146818 has no century register.
    fn advance_year(&mut self) {
        let binary = self.data_mode_binary();
        let (year, carry_century) = field_inc_mod(self.ram[REG_YEAR as usize], 99, binary);
        self.ram[REG_YEAR as usize] = year;
        if !carry_century {
            return;
        }
        let century = self.decode_field(REG_CENTURY);
        self.ram[REG_CENTURY as usize] = encode_field((century + 1) % 100, binary);
    }

    /// Full Gregorian year from the century register (`0x32`) and year register.
    ///
    /// In BCD mode, non-BCD digits in either byte read as 00 (model note, not
    /// spec). In binary mode the stored bytes are used directly (0–99).
    fn full_year(&self) -> u16 {
        let century = self.decode_field(REG_CENTURY);
        let year = self.decode_field(REG_YEAR);
        u16::from(century) * 100 + u16::from(year)
    }

    /// Decode a calendar/time field byte per Status B `DM`.
    fn decode_field(&self, reg: u8) -> u8 {
        let raw = self.ram[reg as usize];
        if self.data_mode_binary() {
            raw
        } else {
            bcd_to_bin(raw).unwrap_or(0)
        }
    }

    fn maybe_set_alarm_flag(&mut self) {
        let sec = self.ram[REG_SEC as usize];
        let min = self.ram[REG_MIN as usize];
        let hour = self.ram[REG_HOUR as usize];
        let a_sec = self.ram[REG_SEC_ALARM as usize];
        let a_min = self.ram[REG_MIN_ALARM as usize];
        let a_hour = self.ram[REG_HOUR_ALARM as usize];
        // Spec: MC146818 — alarm bytes programmed C0h–FFh are "don't care".
        // Byte equality otherwise (BCD or binary per DM; 12-hour hours include AM/PM bit7).
        // AF is set on match regardless of AIE; AIE only gates IRQF (see recompute_irqf).
        let sec_ok = alarm_field_matches(a_sec, sec);
        let min_ok = alarm_field_matches(a_min, min);
        let hour_ok = alarm_field_matches(a_hour, hour);
        if sec_ok && min_ok && hour_ok {
            self.ram[REG_STATUS_C as usize] |= STC_AF;
        }
    }

    fn recompute_irqf(&mut self) {
        let b = self.ram[REG_STATUS_B as usize];
        let mut c = self.ram[REG_STATUS_C as usize] & (STC_PF | STC_AF | STC_UF);
        let irq = (c & STC_PF != 0 && b & STB_PIE != 0)
            || (c & STC_AF != 0 && b & STB_AIE != 0)
            || (c & STC_UF != 0 && b & STB_UIE != 0);
        if irq {
            c |= STC_IRQF;
        }
        self.ram[REG_STATUS_C as usize] = c;
    }
}

impl Default for CmosRtc {
    fn default() -> Self {
        Self::new()
    }
}

impl PortDevice for CmosRtc {
    fn port_read(&mut self, port: u16, _size: u8) -> u32 {
        match port {
            // Index port reads are undefined on many chipsets; return last index|NMI.
            CMOS_INDEX => {
                let nmi = if self.nmi_disabled { NMI_DISABLE } else { 0 };
                u32::from(self.selected_index() | nmi)
            }
            CMOS_DATA => {
                let idx = self.selected_index();
                let value = self.ram[idx as usize];
                // Spec: MC146818 status C is read-to-clear (PF/AF/UF/IRQF).
                if idx == REG_STATUS_C {
                    self.ram[REG_STATUS_C as usize] = 0;
                }
                u32::from(value)
            }
            _ => 0xFFFF_FFFF,
        }
    }

    fn port_write(&mut self, port: u16, _size: u8, value: u32) {
        let v = value as u8;
        match port {
            CMOS_INDEX => {
                self.nmi_disabled = v & NMI_DISABLE != 0;
                self.index = v & INDEX_MASK;
            }
            CMOS_DATA => {
                let idx = self.selected_index();
                // Status C is read-only on real RTC; ignore writes.
                if idx == REG_STATUS_C {
                    return;
                }
                let current = self.ram[idx as usize];
                self.ram[idx as usize] = Self::mask_status_a_write(idx, v, current);
                if idx == REG_STATUS_B {
                    self.recompute_irqf();
                }
            }
            _ => {}
        }
    }
}

/// Advance the hours register for one minute carry; returns date-of-month carry.
///
/// Spec: MC146818 Status B `24/12` + `DM` — 24-hour wraps 23→00; 12-hour advances
/// 1–12 with bit7 = PM (BCD AM `$01`–`$12` / PM `$81`–`$92`; binary AM `$01`–`$0C`
/// / PM `$81`–`$8C`). Day carry only on 11 PM → 12 AM.
fn advance_hour_reg(hour_reg: &mut u8, binary: bool, mode_24: bool) -> bool {
    if mode_24 {
        let (hour, carry_day) = field_inc_mod(*hour_reg, 23, binary);
        *hour_reg = hour;
        carry_day
    } else {
        let (hour, carry_day) = hour_inc_12(*hour_reg, binary);
        *hour_reg = hour;
        carry_day
    }
}

/// Spec: MC146818 12-hour hour increment (hours 1–12; bit7 = PM).
fn hour_inc_12(value: u8, binary: bool) -> (u8, bool) {
    let pm = value & HOUR_PM;
    let hour_bits = value & !HOUR_PM;
    let h = if binary {
        hour_bits
    } else {
        match bcd_to_bin(hour_bits) {
            Some(v) => v,
            // Invalid BCD nibble(s): resync to 1 AM, no day carry.
            None => return (encode_field(1, false), false),
        }
    };
    if !(1..=12).contains(&h) {
        // Unrecognized hour (e.g. reset zero): resync to 1, preserve AM/PM.
        return (encode_field(1, binary) | pm, false);
    }
    let (next_h, next_pm, carry_day) = if h == 11 {
        // 11 AM → 12 PM; 11 PM → 12 AM (+ day).
        (12, pm ^ HOUR_PM, pm != 0)
    } else if h == 12 {
        // 12 AM → 1 AM; 12 PM → 1 PM.
        (1, pm, false)
    } else {
        (h + 1, pm, false)
    };
    (encode_field(next_h, binary) | next_pm, carry_day)
}

/// Spec: MC146818 — alarm register matches current field, or is don't-care (C0h–FFh).
fn alarm_field_matches(alarm: u8, current: u8) -> bool {
    alarm >= 0xC0 || alarm == current
}

/// Convert a hours (or hours-alarm) byte between 24-hour and 12-hour encodings.
///
/// Spec: MC146818 "Time, Calendar, and Alarm Locations" — 24h `0–23`; 12h
/// `1–12` with bit7 = PM. Mapping (model choice when Status B `24/12` toggles):
/// `0 ↔ 12 AM`, `1–11 ↔ 1–11 AM`, `12 ↔ 12 PM`, `13–23 ↔ 1–11 PM`.
/// `DM` selects BCD vs binary for the numeric field. Unrecognized values are
/// returned unchanged.
fn convert_hour_format(value: u8, binary: bool, to_24: bool) -> u8 {
    if to_24 {
        let pm = value & HOUR_PM != 0;
        let hour_bits = value & !HOUR_PM;
        let Some(h) = decode_hour_field(hour_bits, binary) else {
            return value;
        };
        if !(1..=12).contains(&h) {
            return value;
        }
        let h24 = match (h, pm) {
            (12, false) => 0,
            (12, true) => 12,
            (h, false) => h,
            (h, true) => h + 12,
        };
        encode_field(h24, binary)
    } else {
        let Some(h) = decode_hour_field(value, binary) else {
            return value;
        };
        if h > 23 {
            return value;
        }
        let (h12, pm) = match h {
            0 => (12, false),
            1..=11 => (h, false),
            12 => (12, true),
            13..=23 => (h - 12, true),
            24.. => return value,
        };
        encode_field(h12, binary) | if pm { HOUR_PM } else { 0 }
    }
}

/// Decode the numeric portion of an hour byte per Status B `DM`.
fn decode_hour_field(value: u8, binary: bool) -> Option<u8> {
    if binary {
        Some(value)
    } else {
        bcd_to_bin(value)
    }
}

/// Increment a calendar field; `max` is the inclusive decimal maximum (59, 23, 99).
///
/// Spec: MC146818 Status B `DM` — BCD when `binary=false`, binary when `true`.
fn field_inc_mod(value: u8, max: u8, binary: bool) -> (u8, bool) {
    if binary {
        bin_inc_mod(value, max)
    } else {
        bcd_inc_mod(value, bin_to_bcd(max))
    }
}

/// Encode a 0–99 calendar value per Status B `DM`.
fn encode_field(value: u8, binary: bool) -> u8 {
    if binary {
        value % 100
    } else {
        bin_to_bcd(value)
    }
}

/// Increment a binary field; wrap to 0 and report carry when past `max` (inclusive).
fn bin_inc_mod(value: u8, max: u8) -> (u8, bool) {
    if value >= max {
        (0, true)
    } else {
        (value.wrapping_add(1), false)
    }
}

/// Increment a BCD field; wrap to 0 and report carry when past `max_bcd` (inclusive).
fn bcd_inc_mod(value: u8, max_bcd: u8) -> (u8, bool) {
    let ones = value & 0x0F;
    let tens = (value >> 4) & 0x0F;
    let mut next_ones = ones + 1;
    let mut next_tens = tens;
    if next_ones > 9 {
        next_ones = 0;
        next_tens += 1;
    }
    let next = (next_tens << 4) | next_ones;
    if next > max_bcd {
        (0, true)
    } else {
        (next, false)
    }
}

/// Decode a packed two-digit BCD byte; `None` when either nibble is above 9.
fn bcd_to_bin(value: u8) -> Option<u8> {
    let ones = value & 0x0F;
    let tens = value >> 4;
    if ones > 9 || tens > 9 {
        None
    } else {
        Some(tens * 10 + ones)
    }
}

/// Encode 0–99 as packed BCD (larger inputs fold modulo 100).
fn bin_to_bcd(value: u8) -> u8 {
    let value = value % 100;
    ((value / 10) << 4) | (value % 10)
}

/// Days in binary `month` (1–12) for `full_year`.
///
/// Spec: MC146818 date-of-month counting with automatic leap-year compensation
/// for February. Unrecognized months use [`FALLBACK_MONTH_DAYS`].
fn month_length(month: u8, full_year: u16) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(full_year) => 29,
        2 => 28,
        _ => FALLBACK_MONTH_DAYS,
    }
}

/// Gregorian leap year: divisible by 4, except centuries not divisible by 400.
fn is_leap_year(full_year: u16) -> bool {
    full_year.is_multiple_of(4) && (!full_year.is_multiple_of(100) || full_year.is_multiple_of(400))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spec: reset defaults for status A–D; time/calendar zeros (MC146818 / PC AT).
    #[test]
    fn reset_state() {
        let c = CmosRtc::new();
        assert_eq!(c.read_reg(0x00), 0);
        assert_eq!(c.read_reg(REG_STATUS_A), DEFAULT_STATUS_A);
        assert_eq!(c.read_reg(REG_STATUS_B), DEFAULT_STATUS_B);
        assert_eq!(c.read_reg(REG_STATUS_C), DEFAULT_STATUS_C);
        assert_eq!(c.read_reg(REG_STATUS_D), DEFAULT_STATUS_D);
        assert_eq!(c.shutdown_status(), SHUTDOWN_SOFT_OR_UNEXPECTED);
        assert!(!c.nmi_disabled);
        assert_eq!(c.selected_index(), 0);
        assert!(!c.irq_line());

        let mut c2 = CmosRtc::new();
        c2.port_write(CMOS_INDEX, 1, 0x10);
        c2.port_write(CMOS_DATA, 1, 0xAB);
        c2.reset();
        assert_eq!(c2, CmosRtc::new());
    }

    /// Spec: IBM PC/AT CMOS map `0x0F` — shutdown status R/W store/readback.
    #[test]
    fn shutdown_status_store_readback() {
        let mut c = CmosRtc::new();
        assert_eq!(c.read_reg(REG_SHUTDOWN), SHUTDOWN_SOFT_OR_UNEXPECTED);
        assert_eq!(c.shutdown_status(), SHUTDOWN_SOFT_OR_UNEXPECTED);

        c.port_write(CMOS_INDEX, 1, u32::from(REG_SHUTDOWN));
        c.port_write(CMOS_DATA, 1, u32::from(SHUTDOWN_JMP));
        assert_eq!(c.port_read(CMOS_DATA, 1) as u8, SHUTDOWN_JMP);
        assert_eq!(c.read_reg(REG_SHUTDOWN), SHUTDOWN_JMP);
        assert_eq!(c.shutdown_status(), SHUTDOWN_JMP);

        // Other common SeaBIOS / POST codes also store/read back.
        for code in [
            SHUTDOWN_INT19,
            SHUTDOWN_JMP_WITH_EOI,
            SHUTDOWN_BLOCK_MOVE,
            0x01,
            0x0B,
            0x0C,
            0xFF,
        ] {
            c.write_reg(REG_SHUTDOWN, code);
            assert_eq!(c.shutdown_status(), code);
            c.port_write(CMOS_INDEX, 1, u32::from(REG_SHUTDOWN));
            assert_eq!(c.port_read(CMOS_DATA, 1) as u8, code);
        }
    }

    /// Spec: battery-backed shutdown byte survives model reset (SeaBIOS soft-reset).
    #[test]
    fn shutdown_status_survives_reset() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_SHUTDOWN, SHUTDOWN_JMP);
        c.write_reg(0x10, 0xAB); // ordinary config RAM — still cleared on reset
        c.reset();
        assert_eq!(c.shutdown_status(), SHUTDOWN_JMP);
        assert_eq!(c.read_reg(REG_SHUTDOWN), SHUTDOWN_JMP);
        assert_eq!(c.read_reg(0x10), 0);
        assert_eq!(c.read_reg(REG_STATUS_A), DEFAULT_STATUS_A);
    }

    #[test]
    fn index_data_read_write() {
        let mut c = CmosRtc::new();
        c.port_write(CMOS_INDEX, 1, 0x10);
        c.port_write(CMOS_DATA, 1, 0x5A);
        assert_eq!(c.selected_index(), 0x10);
        assert_eq!(c.port_read(CMOS_DATA, 1) as u8, 0x5A);
        assert_eq!(c.read_reg(0x10), 0x5A);
    }

    /// Spec: port 0x70 bit7 is NMI mask; low 7 bits select register.
    #[test]
    fn nmi_disable_bit_tracked() {
        let mut c = CmosRtc::new();
        c.port_write(CMOS_INDEX, 1, 0x80 | 0x0B);
        assert!(c.nmi_disabled);
        assert!(c.nmi_masked());
        assert_eq!(c.selected_index(), REG_STATUS_B);
        c.port_write(CMOS_DATA, 1, 0x06);
        assert_eq!(c.read_reg(REG_STATUS_B), 0x06);

        c.port_write(CMOS_INDEX, 1, 0x0B); // clear NMI disable
        assert!(!c.nmi_disabled);
        assert!(!c.nmi_masked());
        assert_eq!(c.selected_index(), REG_STATUS_B);
    }

    /// Spec: IBM PC/AT — index port write/read preserves NMI bit with register index.
    #[test]
    fn index_port_rw_preserves_nmi_bit() {
        let mut c = CmosRtc::new();
        // Enable NMI mask + select 0x10.
        c.port_write(CMOS_INDEX, 1, 0x80 | 0x10);
        assert_eq!(c.port_read(CMOS_INDEX, 1) as u8, 0x80 | 0x10);
        assert!(c.nmi_masked());
        c.port_write(CMOS_DATA, 1, 0xA5);
        assert_eq!(c.port_read(CMOS_DATA, 1) as u8, 0xA5);
        // Re-select with NMI clear; data still at 0x10.
        c.port_write(CMOS_INDEX, 1, 0x10);
        assert_eq!(c.port_read(CMOS_INDEX, 1) as u8, 0x10);
        assert!(!c.nmi_masked());
        assert_eq!(c.port_read(CMOS_DATA, 1) as u8, 0xA5);
        // NMI mask alone (index 0) then enable again with different index.
        c.port_write(CMOS_INDEX, 1, 0x80);
        assert_eq!(c.port_read(CMOS_INDEX, 1) as u8, 0x80);
        assert!(c.nmi_masked());
        assert_eq!(c.selected_index(), 0);
    }

    #[test]
    fn index_masked_to_7_bits() {
        let mut c = CmosRtc::new();
        // 0xFF → index 0x7F with NMI disable
        c.port_write(CMOS_INDEX, 1, 0xFF);
        assert_eq!(c.selected_index(), 0x7F);
        assert!(c.nmi_disabled);
        c.port_write(CMOS_DATA, 1, 0x11);
        assert_eq!(c.read_reg(0x7F), 0x11);
        // Direct write_reg also masks.
        c.write_reg(0x80 | 0x05, 0x22);
        assert_eq!(c.read_reg(0x05), 0x22);
    }

    /// Spec: MC146818 status C read-to-clear; writes ignored.
    #[test]
    fn status_c_read_to_clear() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_STATUS_B, DEFAULT_STATUS_B | STB_PIE);
        assert!(c.tick(1));
        assert!(c.irq_line());
        let flags = c.read_reg(REG_STATUS_C);
        assert_ne!(flags & STC_PF, 0);
        assert_ne!(flags & STC_IRQF, 0);
        c.port_write(CMOS_INDEX, 1, u32::from(REG_STATUS_C));
        assert_eq!(c.port_read(CMOS_DATA, 1) as u8, flags);
        assert_eq!(c.read_reg(REG_STATUS_C), 0);
        assert!(!c.irq_line());
        // Writes to C ignored.
        c.port_write(CMOS_DATA, 1, 0xFF);
        assert_eq!(c.read_reg(REG_STATUS_C), 0);
    }

    /// Spec: MC146818 PIE + RS≠0 → PF/IRQF on tick; IRQ pin follows IRQF.
    #[test]
    fn pie_tick_asserts_irq_line() {
        let mut c = CmosRtc::new();
        c.port_write(CMOS_INDEX, 1, u32::from(REG_STATUS_B));
        c.port_write(CMOS_DATA, 1, u32::from(DEFAULT_STATUS_B | STB_PIE));
        assert!(!c.irq_line());
        assert!(c.tick(1));
        assert!(c.irq_line());
        assert_ne!(c.read_reg(REG_STATUS_C) & STC_PF, 0);
        assert_ne!(c.read_reg(REG_STATUS_C) & STC_IRQF, 0);
    }

    /// Spec: PF may set, but IRQF/IRQ require PIE.
    #[test]
    fn tick_without_pie_does_not_assert_irq() {
        let mut c = CmosRtc::new();
        assert!(!c.tick(1));
        assert!(!c.irq_line());
        // UF set (SET clear) but UIE off → no IRQF.
        assert_ne!(c.read_reg(REG_STATUS_C) & STC_UF, 0);
        assert_eq!(c.read_reg(REG_STATUS_C) & STC_IRQF, 0);
    }

    /// Spec: UIE + update-ended (UF) asserts IRQF.
    #[test]
    fn uie_tick_asserts_irq_line() {
        let mut c = CmosRtc::new();
        c.port_write(CMOS_INDEX, 1, u32::from(REG_STATUS_B));
        c.port_write(CMOS_DATA, 1, u32::from(DEFAULT_STATUS_B | STB_UIE));
        assert!(c.tick(1));
        assert!(c.irq_line());
        assert_ne!(c.read_reg(REG_STATUS_C) & STC_UF, 0);
    }

    /// Spec: AIE + matching alarm registers → AF/IRQF.
    #[test]
    fn aie_matching_alarm_asserts_irq() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_SEC, 0x30);
        c.write_reg(REG_MIN, 0x15);
        c.write_reg(REG_HOUR, 0x10);
        c.write_reg(REG_SEC_ALARM, 0x30);
        c.write_reg(REG_MIN_ALARM, 0x15);
        c.write_reg(REG_HOUR_ALARM, 0x10);
        c.port_write(CMOS_INDEX, 1, u32::from(REG_STATUS_B));
        c.port_write(CMOS_DATA, 1, u32::from(DEFAULT_STATUS_B | STB_AIE));
        assert!(c.tick(1));
        assert!(c.irq_line());
        assert_ne!(c.read_reg(REG_STATUS_C) & STC_AF, 0);
    }

    /// Spec: MC146818 — after `tick_second` advances into the alarm time, AIE → AF/IRQF.
    #[test]
    fn aie_tick_second_matching_alarm_asserts_irq() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_SEC, 0x29);
        c.write_reg(REG_MIN, 0x15);
        c.write_reg(REG_HOUR, 0x10);
        c.write_reg(REG_SEC_ALARM, 0x30);
        c.write_reg(REG_MIN_ALARM, 0x15);
        c.write_reg(REG_HOUR_ALARM, 0x10);
        c.write_reg(REG_STATUS_B, DEFAULT_STATUS_B | STB_AIE);
        assert!(c.tick_second());
        assert_eq!(c.read_reg(REG_SEC), 0x30);
        assert!(c.irq_line());
        assert_ne!(c.read_reg(REG_STATUS_C) & (STC_AF | STC_IRQF), 0);
    }

    /// Spec: MC146818 alarm "don't care" is C0h–FFh (not merely bit7).
    #[test]
    fn aie_alarm_dont_care_c0_matches_any_field() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_SEC, 0x45);
        c.write_reg(REG_MIN, 0x12);
        c.write_reg(REG_HOUR, 0x08);
        c.write_reg(REG_SEC_ALARM, 0xC0);
        c.write_reg(REG_MIN_ALARM, 0xFF);
        c.write_reg(REG_HOUR_ALARM, 0xC0);
        c.write_reg(REG_STATUS_B, DEFAULT_STATUS_B | STB_AIE);
        assert!(c.tick(1));
        assert_ne!(c.read_reg(REG_STATUS_C) & STC_AF, 0);
        assert!(c.irq_line());
    }

    /// Spec: values below C0h (e.g. 80h) are not don't-care — needed for 12-hour PM hours.
    #[test]
    fn aie_alarm_0x80_is_not_dont_care() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_SEC, 0x00);
        c.write_reg(REG_MIN, 0x00);
        c.write_reg(REG_HOUR, 0x01);
        c.write_reg(REG_SEC_ALARM, 0x80); // bit7 set but < C0 → must match exactly
        c.write_reg(REG_MIN_ALARM, 0x00);
        c.write_reg(REG_HOUR_ALARM, 0x01);
        c.write_reg(REG_STATUS_B, DEFAULT_STATUS_B | STB_AIE);
        assert!(!c.tick(1)); // UF may set; AIE∧AF must not
        assert_eq!(c.read_reg(REG_STATUS_C) & STC_AF, 0);
        assert_eq!(c.read_reg(REG_STATUS_C) & STC_IRQF, 0);
    }

    /// Spec: DM=1 binary time/alarm bytes compare equal on match.
    #[test]
    fn aie_tick_second_binary_dm_matching_alarm_asserts_irq() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_SEC, 29);
        c.write_reg(REG_MIN, 15);
        c.write_reg(REG_HOUR, 16);
        c.write_reg(REG_SEC_ALARM, 30);
        c.write_reg(REG_MIN_ALARM, 15);
        c.write_reg(REG_HOUR_ALARM, 16);
        c.write_reg(REG_STATUS_B, DEFAULT_STATUS_B | STB_DM | STB_AIE);
        assert!(c.tick_second());
        assert_eq!(c.read_reg(REG_SEC), 30);
        assert_ne!(c.read_reg(REG_STATUS_C) & STC_AF, 0);
        assert!(c.irq_line());
    }

    /// Spec: 12-hour mode matches full hours byte including AM/PM bit7.
    #[test]
    fn aie_tick_second_twelve_hour_pm_matching_alarm_asserts_irq() {
        let mut c = CmosRtc::new();
        // 12-hour BCD: 3:15:29 PM → alarm at 3:15:30 PM ($83).
        c.write_reg(REG_SEC, 0x29);
        c.write_reg(REG_MIN, 0x15);
        c.write_reg(REG_HOUR, HOUR_PM | 0x03);
        c.write_reg(REG_SEC_ALARM, 0x30);
        c.write_reg(REG_MIN_ALARM, 0x15);
        c.write_reg(REG_HOUR_ALARM, HOUR_PM | 0x03);
        c.write_reg(REG_STATUS_B, STB_AIE); // 12-hour (bit1 clear) + AIE
        assert!(c.tick_second());
        assert_eq!(c.read_reg(REG_HOUR), HOUR_PM | 0x03);
        assert_eq!(c.read_reg(REG_SEC), 0x30);
        assert_ne!(c.read_reg(REG_STATUS_C) & STC_AF, 0);
        assert!(c.irq_line());
    }

    /// Spec: AF sets on time==alarm even when AIE=0; IRQF still requires AIE.
    #[test]
    fn alarm_match_sets_af_without_aie_no_irqf() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_SEC, 0x30);
        c.write_reg(REG_MIN, 0x15);
        c.write_reg(REG_HOUR, 0x10);
        c.write_reg(REG_SEC_ALARM, 0x30);
        c.write_reg(REG_MIN_ALARM, 0x15);
        c.write_reg(REG_HOUR_ALARM, 0x10);
        // DEFAULT_STATUS_B: AIE clear
        assert!(!c.tick(1));
        assert_ne!(c.read_reg(REG_STATUS_C) & STC_AF, 0);
        assert_eq!(c.read_reg(REG_STATUS_C) & STC_IRQF, 0);
        assert!(!c.irq_line());
    }

    /// Spec: mismatched alarm field → AF clear (AIE alone does not assert).
    #[test]
    fn aie_mismatched_alarm_does_not_set_af() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_SEC, 0x30);
        c.write_reg(REG_MIN, 0x15);
        c.write_reg(REG_HOUR, 0x10);
        c.write_reg(REG_SEC_ALARM, 0x31);
        c.write_reg(REG_MIN_ALARM, 0x15);
        c.write_reg(REG_HOUR_ALARM, 0x10);
        c.write_reg(REG_STATUS_B, DEFAULT_STATUS_B | STB_AIE);
        assert!(!c.tick(1));
        assert_eq!(c.read_reg(REG_STATUS_C) & STC_AF, 0);
        assert_eq!(c.read_reg(REG_STATUS_C) & STC_IRQF, 0);
    }

    /// Spec: RS=0 disables periodic (PF not set from rate).
    #[test]
    fn rs_zero_skips_periodic_flag() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_STATUS_A, DEFAULT_STATUS_A & !STATUS_A_RS_MASK); // RS=0
        c.write_reg(REG_STATUS_B, DEFAULT_STATUS_B | STB_PIE);
        assert!(!c.tick(1));
        assert_eq!(c.read_reg(REG_STATUS_C) & STC_PF, 0);
    }

    #[test]
    fn state_clone_equality_round_trip() {
        let mut c = CmosRtc::new();
        c.port_write(CMOS_INDEX, 1, 0x80 | 0x14);
        c.port_write(CMOS_DATA, 1, 0xBE);
        let cloned = c.clone();
        assert_eq!(c, cloned);
        assert!(cloned.nmi_disabled);
        assert_eq!(cloned.read_reg(0x14), 0xBE);
    }

    #[test]
    fn unrelated_ports_ignored() {
        let mut c = CmosRtc::new();
        c.port_write(0x3F8, 1, 0x10);
        assert_eq!(c.selected_index(), 0);
        assert_eq!(c.port_read(0x3F8, 1), 0xFFFF_FFFF);
    }

    /// Spec: MC146818 Status A bit7 UIP is set during the update cycle and clear after.
    #[test]
    fn uip_set_during_update_cleared_after() {
        let mut c = CmosRtc::new();
        assert_eq!(c.read_reg(REG_STATUS_A) & STATUS_A_UIP, 0);
        assert!(c.begin_update_for_test());
        assert_ne!(c.read_reg(REG_STATUS_A) & STATUS_A_UIP, 0);
        let _ = c.end_update_for_test();
        assert_eq!(c.read_reg(REG_STATUS_A) & STATUS_A_UIP, 0);
    }

    /// Spec: MC146818 Status A UIP is readable high during the update window, then low.
    ///
    /// Model (approximate, not crystal-timed): `tick_second` leaves UIP set; each
    /// subsequent `tick` period decays the hold; after [`UIP_WINDOW_PERIODS`]
    /// periods UIP clears. UF still latches at calendar update-ended.
    #[test]
    fn uip_high_after_tick_second_then_low_after_window() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_SEC, 0x10);
        assert_eq!(c.read_reg(REG_STATUS_A) & STATUS_A_UIP, 0);
        assert!(!c.tick_second()); // UIE off → no IRQ rising edge
        assert_eq!(c.read_reg(REG_SEC), 0x11);
        assert_ne!(c.read_reg(REG_STATUS_C) & STC_UF, 0);
        // UIP stays high and is guest-readable via the data port.
        assert_ne!(c.read_reg(REG_STATUS_A) & STATUS_A_UIP, 0);
        c.port_write(CMOS_INDEX, 1, u32::from(REG_STATUS_A));
        assert_ne!(c.port_read(CMOS_DATA, 1) as u8 & STATUS_A_UIP, 0);
        // Mid-window: first period of the hold leaves UIP set; final period clears.
        let _ = c.tick(1);
        assert_ne!(c.read_reg(REG_STATUS_A) & STATUS_A_UIP, 0);
        let _ = c.tick(u64::from(UIP_WINDOW_PERIODS) - 1);
        assert_eq!(c.read_reg(REG_STATUS_A) & STATUS_A_UIP, 0);
    }

    /// Model: a single `tick(UIP_WINDOW_PERIODS)` call also clears the UIP window.
    #[test]
    fn uip_window_clears_in_one_multi_period_tick() {
        let mut c = CmosRtc::new();
        assert!(!c.tick_second());
        assert_ne!(c.read_reg(REG_STATUS_A) & STATUS_A_UIP, 0);
        let _ = c.tick(u64::from(UIP_WINDOW_PERIODS));
        assert_eq!(c.read_reg(REG_STATUS_A) & STATUS_A_UIP, 0);
    }

    /// Spec: UIP is hardware-driven / read-only; guest writes via 0x71 cannot sticky-set it.
    #[test]
    fn guest_cannot_sticky_write_uip() {
        let mut c = CmosRtc::new();
        c.port_write(CMOS_INDEX, 1, u32::from(REG_STATUS_A));
        c.port_write(CMOS_DATA, 1, u32::from(DEFAULT_STATUS_A | STATUS_A_UIP));
        assert_eq!(c.read_reg(REG_STATUS_A) & STATUS_A_UIP, 0);
        assert_eq!(c.read_reg(REG_STATUS_A), DEFAULT_STATUS_A);
        // write_reg path likewise.
        c.write_reg(REG_STATUS_A, DEFAULT_STATUS_A | STATUS_A_UIP | 0x01);
        assert_eq!(c.read_reg(REG_STATUS_A) & STATUS_A_UIP, 0);
    }

    /// Spec: when SET=0, update cycle advances BCD seconds (cascade min/hour stub).
    #[test]
    fn tick_second_advances_bcd_seconds() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_SEC, 0x58);
        c.write_reg(REG_MIN, 0x59);
        c.write_reg(REG_HOUR, 0x23);
        assert!(!c.tick_second()); // UIE off → no IRQ rising edge
        assert_eq!(c.read_reg(REG_SEC), 0x59);
        assert_eq!(c.read_reg(REG_MIN), 0x59);
        assert_eq!(c.read_reg(REG_HOUR), 0x23);
        let _ = c.tick_second();
        assert_eq!(c.read_reg(REG_SEC), 0x00);
        assert_eq!(c.read_reg(REG_MIN), 0x00);
        assert_eq!(c.read_reg(REG_HOUR), 0x00);
        assert_ne!(c.read_reg(REG_STATUS_C) & STC_UF, 0);
    }

    /// Spec: Status B SET inhibits the update cycle (no UIP, no calendar advance, no UF).
    #[test]
    fn set_inhibits_second_update() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_SEC, 0x10);
        c.write_reg(REG_STATUS_B, DEFAULT_STATUS_B | STB_SET);
        assert!(!c.begin_update_for_test());
        assert_eq!(c.read_reg(REG_STATUS_A) & STATUS_A_UIP, 0);
        assert!(!c.tick_second());
        assert_eq!(c.read_reg(REG_SEC), 0x10);
        assert_eq!(c.read_reg(REG_STATUS_C) & STC_UF, 0);
        assert_eq!(c.read_reg(REG_STATUS_A) & STATUS_A_UIP, 0);
    }

    /// Program the BCD calendar registers (century, year, month, day, weekday).
    fn set_date(c: &mut CmosRtc, century: u8, year: u8, month: u8, day: u8, dow: u8) {
        c.write_reg(REG_CENTURY, century);
        c.write_reg(REG_YEAR, year);
        c.write_reg(REG_MONTH, month);
        c.write_reg(REG_DAY_OF_MONTH, day);
        c.write_reg(REG_DAY_OF_WEEK, dow);
    }

    /// Park the clock one second before midnight so a single update cycle
    /// runs the whole sec→min→hour→day cascade.
    fn set_end_of_day(c: &mut CmosRtc) {
        c.write_reg(REG_HOUR, 0x23);
        c.write_reg(REG_MIN, 0x59);
        c.write_reg(REG_SEC, 0x59);
    }

    /// Stored calendar BCD as (century, year, month, day, weekday).
    fn date_of(c: &CmosRtc) -> (u8, u8, u8, u8, u8) {
        (
            c.read_reg(REG_CENTURY),
            c.read_reg(REG_YEAR),
            c.read_reg(REG_MONTH),
            c.read_reg(REG_DAY_OF_MONTH),
            c.read_reg(REG_DAY_OF_WEEK),
        )
    }

    /// Advance exactly one day boundary.
    fn tick_over_midnight(c: &mut CmosRtc) {
        set_end_of_day(c);
        let _ = c.tick_second();
    }

    /// Spec: MC146818 update cycle — hours 23 → 00 carries into the day of month,
    /// and the day-of-week register (0x06) advances with the date
    /// (datasheet "Time, Calendar, and Alarm Locations" + update-cycle section).
    #[test]
    fn hour_rollover_carries_into_day_and_weekday() {
        let mut c = CmosRtc::new();
        set_date(&mut c, 0x20, 0x24, 0x03, 0x10, 0x01);
        set_end_of_day(&mut c);
        let _ = c.tick_second();
        assert_eq!(c.read_reg(REG_SEC), 0x00);
        assert_eq!(c.read_reg(REG_MIN), 0x00);
        assert_eq!(c.read_reg(REG_HOUR), 0x00);
        assert_eq!(date_of(&c), (0x20, 0x24, 0x03, 0x11, 0x02));
        // A second inside the day leaves the calendar untouched.
        c.write_reg(REG_HOUR, 0x12);
        let _ = c.tick_second();
        assert_eq!(c.read_reg(REG_SEC), 0x01);
        assert_eq!(date_of(&c), (0x20, 0x24, 0x03, 0x11, 0x02));
    }

    /// Spec: MC146818 date of month counts per month length — 31-day and 30-day
    /// months roll to day 01 of the next month (datasheet calendar locations).
    #[test]
    fn end_of_month_rolls_31_and_30_day_months() {
        let mut jan = CmosRtc::new();
        set_date(&mut jan, 0x20, 0x24, 0x01, 0x31, 0x03);
        tick_over_midnight(&mut jan);
        assert_eq!(date_of(&jan), (0x20, 0x24, 0x02, 0x01, 0x04));

        let mut apr = CmosRtc::new();
        set_date(&mut apr, 0x20, 0x24, 0x04, 0x30, 0x03);
        tick_over_midnight(&mut apr);
        assert_eq!(date_of(&apr), (0x20, 0x24, 0x05, 0x01, 0x04));
    }

    /// Spec: MC146818 automatic leap-year compensation — February has 28 days in
    /// a common year, so Feb 28 rolls to Mar 1.
    #[test]
    fn february_non_leap_year_rolls_to_march() {
        let mut c = CmosRtc::new();
        set_date(&mut c, 0x20, 0x23, 0x02, 0x28, 0x02);
        tick_over_midnight(&mut c);
        assert_eq!(date_of(&c), (0x20, 0x23, 0x03, 0x01, 0x03));
    }

    /// Spec: MC146818 automatic leap-year compensation — a leap year gets Feb 29
    /// before rolling into March.
    #[test]
    fn february_leap_year_has_twenty_nine_days() {
        let mut c = CmosRtc::new();
        set_date(&mut c, 0x20, 0x24, 0x02, 0x28, 0x04);
        tick_over_midnight(&mut c);
        assert_eq!(date_of(&c), (0x20, 0x24, 0x02, 0x29, 0x05));
        tick_over_midnight(&mut c);
        assert_eq!(date_of(&c), (0x20, 0x24, 0x03, 0x01, 0x06));
    }

    /// Spec: Gregorian rule via the PC/AT + ACPI FADT century register (index 0x32):
    /// 2000 is divisible by 400 (leap), 1900 is divisible by 100 but not 400 (common).
    #[test]
    fn century_years_follow_gregorian_leap_rule() {
        let mut y2000 = CmosRtc::new();
        set_date(&mut y2000, 0x20, 0x00, 0x02, 0x28, 0x03);
        tick_over_midnight(&mut y2000);
        assert_eq!(date_of(&y2000), (0x20, 0x00, 0x02, 0x29, 0x04));

        let mut y1900 = CmosRtc::new();
        set_date(&mut y1900, 0x19, 0x00, 0x02, 0x28, 0x04);
        tick_over_midnight(&mut y1900);
        assert_eq!(date_of(&y1900), (0x19, 0x00, 0x03, 0x01, 0x05));
    }

    /// Spec: MC146818 month 12 → 01 carries into the year register.
    #[test]
    fn december_31_rolls_into_next_year() {
        let mut c = CmosRtc::new();
        set_date(&mut c, 0x20, 0x23, 0x12, 0x31, 0x01);
        tick_over_midnight(&mut c);
        assert_eq!(date_of(&c), (0x20, 0x24, 0x01, 0x01, 0x02));
    }

    /// Spec: year 99 → 00 carries into the PC/AT + ACPI FADT century byte (0x32),
    /// which is outside the base MC146818 register file.
    #[test]
    fn year_99_carries_into_century_register() {
        let mut c = CmosRtc::new();
        set_date(&mut c, 0x19, 0x99, 0x12, 0x31, 0x06);
        tick_over_midnight(&mut c);
        assert_eq!(date_of(&c), (0x20, 0x00, 0x01, 0x01, 0x07));
    }

    /// Spec: MC146818 day-of-week counts 1–7 (1 = Sunday) and wraps 7 → 1,
    /// independently of the month/day arithmetic.
    #[test]
    fn day_of_week_wraps_seven_to_one() {
        let mut c = CmosRtc::new();
        set_date(&mut c, 0x20, 0x24, 0x06, 0x15, 0x07);
        tick_over_midnight(&mut c);
        assert_eq!(date_of(&c), (0x20, 0x24, 0x06, 0x16, 0x01));
    }

    /// Spec: MC146818 Status B SET=1 inhibits the update cycle — no field of the
    /// time or calendar advances while the guest is programming the clock.
    #[test]
    fn set_inhibits_calendar_advance() {
        let mut c = CmosRtc::new();
        set_date(&mut c, 0x20, 0x24, 0x12, 0x31, 0x03);
        set_end_of_day(&mut c);
        c.write_reg(REG_STATUS_B, DEFAULT_STATUS_B | STB_SET);
        assert!(!c.tick_second());
        assert_eq!(c.read_reg(REG_SEC), 0x59);
        assert_eq!(c.read_reg(REG_MIN), 0x59);
        assert_eq!(c.read_reg(REG_HOUR), 0x23);
        assert_eq!(date_of(&c), (0x20, 0x24, 0x12, 0x31, 0x03));
    }

    /// Model note (not MC146818 spec): reset leaves the calendar zeroed, so month
    /// 0x00 / day 0x00 / weekday 0x00 are reachable but are not valid dates.
    /// The cascade stays total: an unrecognized month uses the 31-day fallback
    /// length, a day past that length wraps to 01 and steps the month, an
    /// unrecognized month steps to January without a year carry, and an
    /// unrecognized weekday resets to 1. No panic or wrap-around.
    #[test]
    fn invalid_zero_calendar_advances_without_panic() {
        let mut zeroed = CmosRtc::new();
        assert_eq!(date_of(&zeroed), (0x00, 0x00, 0x00, 0x00, 0x00));
        tick_over_midnight(&mut zeroed);
        assert_eq!(date_of(&zeroed), (0x00, 0x00, 0x00, 0x01, 0x01));

        let mut past_fallback = CmosRtc::new();
        set_date(&mut past_fallback, 0x00, 0x00, 0x00, 0x31, 0x00);
        tick_over_midnight(&mut past_fallback);
        assert_eq!(date_of(&past_fallback), (0x00, 0x00, 0x01, 0x01, 0x01));

        // Non-BCD day/month digits are equally well defined (day wraps, month
        // falls back to January).
        let mut non_bcd = CmosRtc::new();
        set_date(&mut non_bcd, 0x20, 0x24, 0x1A, 0x9F, 0x0F);
        tick_over_midnight(&mut non_bcd);
        assert_eq!(date_of(&non_bcd), (0x20, 0x24, 0x01, 0x01, 0x01));
    }

    /// Spec: UIE + update-ended from `tick_second` asserts IRQF (same IRQ pin as PIE path).
    /// UIP remains high for the approximate window; UF still latches immediately.
    #[test]
    fn uie_tick_second_asserts_irq_line() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_STATUS_B, DEFAULT_STATUS_B | STB_UIE);
        assert!(c.tick_second());
        assert!(c.irq_line());
        assert_ne!(c.read_reg(REG_STATUS_C) & STC_UF, 0);
        assert_ne!(c.read_reg(REG_STATUS_A) & STATUS_A_UIP, 0);
        let _ = c.tick(u64::from(UIP_WINDOW_PERIODS));
        assert_eq!(c.read_reg(REG_STATUS_A) & STATUS_A_UIP, 0);
    }

    // --- Status B DM (binary calendar mode) ---------------------------------
    // Spec: MC146818 Status Register B bit 2 (`DM`) — 0 = BCD, 1 = binary.
    // OSDev CMOS / IBM PC AT RTC: when DM=1, time and calendar registers are
    // stored and updated as binary integers (sec/min 0–59, hour 0–23 in 24h
    // mode, date/month/year as binary), not packed BCD.

    /// Spec: reset Status B has DM cleared (BCD) and 24-hour bit set (`0x02`).
    #[test]
    fn reset_defaults_dm_cleared_bcd_mode() {
        let c = CmosRtc::new();
        assert_eq!(c.read_reg(REG_STATUS_B), DEFAULT_STATUS_B);
        assert_eq!(c.read_reg(REG_STATUS_B) & STB_DM, 0);
        assert_ne!(c.read_reg(REG_STATUS_B) & STB_24_12, 0); // 24-hour
    }

    /// Spec: DM=0 keeps the existing BCD cascade (regression vs binary path).
    #[test]
    fn dm_clear_keeps_bcd_sec_min_cascade() {
        let mut c = CmosRtc::new();
        assert_eq!(c.read_reg(REG_STATUS_B) & STB_DM, 0);
        c.write_reg(REG_SEC, 0x58);
        c.write_reg(REG_MIN, 0x59);
        c.write_reg(REG_HOUR, 0x10);
        let _ = c.tick_second();
        assert_eq!(c.read_reg(REG_SEC), 0x59);
        assert_eq!(c.read_reg(REG_MIN), 0x59);
        assert_eq!(c.read_reg(REG_HOUR), 0x10);
        let _ = c.tick_second();
        assert_eq!(c.read_reg(REG_SEC), 0x00);
        assert_eq!(c.read_reg(REG_MIN), 0x00);
        assert_eq!(c.read_reg(REG_HOUR), 0x11);
    }

    /// Spec: DM=1 — seconds/minutes/hours update and store as binary (not BCD).
    #[test]
    fn dm_binary_tick_cascades_sec_min_hour() {
        let mut c = CmosRtc::new();
        // DEFAULT_STATUS_B already has 24-hour; add DM.
        c.write_reg(REG_STATUS_B, DEFAULT_STATUS_B | STB_DM);
        // Binary values that are not valid packed-BCD digits (0x3A = 58).
        c.write_reg(REG_SEC, 58);
        c.write_reg(REG_MIN, 59);
        c.write_reg(REG_HOUR, 23);
        let _ = c.tick_second();
        assert_eq!(c.read_reg(REG_SEC), 59);
        assert_eq!(c.read_reg(REG_MIN), 59);
        assert_eq!(c.read_reg(REG_HOUR), 23);
        let _ = c.tick_second();
        assert_eq!(c.read_reg(REG_SEC), 0);
        assert_eq!(c.read_reg(REG_MIN), 0);
        assert_eq!(c.read_reg(REG_HOUR), 0);
        assert_ne!(c.read_reg(REG_STATUS_C) & STC_UF, 0);
    }

    /// Spec: DM=1 — midnight binary cascade into date/month/year/century/weekday.
    #[test]
    fn dm_binary_midnight_carries_calendar() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_STATUS_B, DEFAULT_STATUS_B | STB_DM);
        // 2023-12-31 23:59:59 binary → 2024-01-01 00:00:00, weekday 6→7.
        c.write_reg(REG_CENTURY, 20);
        c.write_reg(REG_YEAR, 23);
        c.write_reg(REG_MONTH, 12);
        c.write_reg(REG_DAY_OF_MONTH, 31);
        c.write_reg(REG_DAY_OF_WEEK, 6);
        c.write_reg(REG_HOUR, 23);
        c.write_reg(REG_MIN, 59);
        c.write_reg(REG_SEC, 59);
        let _ = c.tick_second();
        assert_eq!(c.read_reg(REG_SEC), 0);
        assert_eq!(c.read_reg(REG_MIN), 0);
        assert_eq!(c.read_reg(REG_HOUR), 0);
        assert_eq!(c.read_reg(REG_CENTURY), 20);
        assert_eq!(c.read_reg(REG_YEAR), 24);
        assert_eq!(c.read_reg(REG_MONTH), 1);
        assert_eq!(c.read_reg(REG_DAY_OF_MONTH), 1);
        assert_eq!(c.read_reg(REG_DAY_OF_WEEK), 7);
    }

    /// Spec: DM=1 leap-year February uses binary month/day (29 Feb in 2024).
    #[test]
    fn dm_binary_february_leap_year() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_STATUS_B, DEFAULT_STATUS_B | STB_DM);
        c.write_reg(REG_CENTURY, 20);
        c.write_reg(REG_YEAR, 24);
        c.write_reg(REG_MONTH, 2);
        c.write_reg(REG_DAY_OF_MONTH, 28);
        c.write_reg(REG_DAY_OF_WEEK, 4);
        c.write_reg(REG_HOUR, 23);
        c.write_reg(REG_MIN, 59);
        c.write_reg(REG_SEC, 59);
        let _ = c.tick_second();
        assert_eq!(c.read_reg(REG_MONTH), 2);
        assert_eq!(c.read_reg(REG_DAY_OF_MONTH), 29);
        c.write_reg(REG_HOUR, 23);
        c.write_reg(REG_MIN, 59);
        c.write_reg(REG_SEC, 59);
        let _ = c.tick_second();
        assert_eq!(c.read_reg(REG_MONTH), 3);
        assert_eq!(c.read_reg(REG_DAY_OF_MONTH), 1);
    }

    /// Spec: Status B SET inhibits updates even when DM=1.
    #[test]
    fn dm_set_inhibits_binary_calendar_advance() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_STATUS_B, DEFAULT_STATUS_B | STB_DM | STB_SET);
        c.write_reg(REG_SEC, 58);
        c.write_reg(REG_MIN, 59);
        c.write_reg(REG_HOUR, 23);
        assert!(!c.tick_second());
        assert_eq!(c.read_reg(REG_SEC), 58);
        assert_eq!(c.read_reg(REG_MIN), 59);
        assert_eq!(c.read_reg(REG_HOUR), 23);
        assert_eq!(c.read_reg(REG_STATUS_C) & STC_UF, 0);
        assert_eq!(c.read_reg(REG_STATUS_A) & STATUS_A_UIP, 0);
    }

    /// Spec: DM does not change IRQF / UIE / status C semantics.
    #[test]
    fn dm_uie_tick_second_still_asserts_irq() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_STATUS_B, DEFAULT_STATUS_B | STB_DM | STB_UIE);
        assert!(c.tick_second());
        assert!(c.irq_line());
        assert_ne!(c.read_reg(REG_STATUS_C) & STC_UF, 0);
        assert_ne!(c.read_reg(REG_STATUS_C) & STC_IRQF, 0);
    }

    // --- Status B 24/12 (12-hour mode) --------------------------------------
    // Spec: MC146818 Status Register B bit 1 (`24/12`) — 1 = 24-hour (0–23),
    // 0 = 12-hour (1–12) with bit7 of the hours byte = PM. Encoding interacts
    // with `DM`: BCD AM $01–$12 / PM $81–$92; binary AM $01–$0C / PM $81–$8C.

    /// Spec: reset Status B has 24/12 set (24-hour) and DM clear.
    #[test]
    fn reset_defaults_24_hour_bit_set() {
        let c = CmosRtc::new();
        assert_eq!(c.read_reg(REG_STATUS_B), DEFAULT_STATUS_B);
        assert_ne!(c.read_reg(REG_STATUS_B) & STB_24_12, 0);
        assert_eq!(c.read_reg(REG_STATUS_B) & STB_DM, 0);
    }

    /// Spec: 12-hour + BCD — minutes cascade into hours (10:59:59 AM → 11:00:00 AM).
    #[test]
    fn twelve_hour_bcd_tick_cascades_sec_min_hour() {
        let mut c = CmosRtc::new();
        // Clear 24/12 → 12-hour; DM remains BCD.
        c.write_reg(REG_STATUS_B, 0x00);
        c.write_reg(REG_HOUR, 0x10); // 10 AM
        c.write_reg(REG_MIN, 0x59);
        c.write_reg(REG_SEC, 0x59);
        let _ = c.tick_second();
        assert_eq!(c.read_reg(REG_SEC), 0x00);
        assert_eq!(c.read_reg(REG_MIN), 0x00);
        assert_eq!(c.read_reg(REG_HOUR), 0x11); // 11 AM
        assert_eq!(c.read_reg(REG_HOUR) & HOUR_PM, 0);
    }

    /// Spec: 12-hour + BCD — 11:59:59 AM → 12:00:00 PM (noon; bit7 set, no day carry).
    #[test]
    fn twelve_hour_bcd_noon_edge() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_STATUS_B, 0x00);
        c.write_reg(REG_CENTURY, 0x20);
        c.write_reg(REG_YEAR, 0x24);
        c.write_reg(REG_MONTH, 0x06);
        c.write_reg(REG_DAY_OF_MONTH, 0x15);
        c.write_reg(REG_DAY_OF_WEEK, 0x03);
        c.write_reg(REG_HOUR, 0x11); // 11 AM
        c.write_reg(REG_MIN, 0x59);
        c.write_reg(REG_SEC, 0x59);
        let _ = c.tick_second();
        assert_eq!(c.read_reg(REG_HOUR), 0x92); // 12 PM (BCD 12 | PM)
        assert_eq!(c.read_reg(REG_MIN), 0x00);
        assert_eq!(c.read_reg(REG_SEC), 0x00);
        assert_eq!(c.read_reg(REG_DAY_OF_MONTH), 0x15);
        assert_eq!(c.read_reg(REG_DAY_OF_WEEK), 0x03);
    }

    /// Spec: 12-hour + BCD — 12:59:59 PM → 1:00:00 PM (keep PM; no day carry).
    #[test]
    fn twelve_hour_bcd_noon_to_one_pm() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_STATUS_B, 0x00);
        c.write_reg(REG_HOUR, 0x92); // 12 PM
        c.write_reg(REG_MIN, 0x59);
        c.write_reg(REG_SEC, 0x59);
        let _ = c.tick_second();
        assert_eq!(c.read_reg(REG_HOUR), 0x81); // 1 PM
        assert_eq!(c.read_reg(REG_MIN), 0x00);
        assert_eq!(c.read_reg(REG_SEC), 0x00);
    }

    /// Spec: 12-hour + BCD — 11:59:59 PM → 12:00:00 AM (midnight) + date/weekday carry.
    #[test]
    fn twelve_hour_bcd_midnight_edge_carries_calendar() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_STATUS_B, 0x00);
        c.write_reg(REG_CENTURY, 0x20);
        c.write_reg(REG_YEAR, 0x23);
        c.write_reg(REG_MONTH, 0x12);
        c.write_reg(REG_DAY_OF_MONTH, 0x31);
        c.write_reg(REG_DAY_OF_WEEK, 0x06);
        c.write_reg(REG_HOUR, 0x91); // 11 PM (BCD 11 | PM)
        c.write_reg(REG_MIN, 0x59);
        c.write_reg(REG_SEC, 0x59);
        let _ = c.tick_second();
        assert_eq!(c.read_reg(REG_HOUR), 0x12); // 12 AM
        assert_eq!(c.read_reg(REG_HOUR) & HOUR_PM, 0);
        assert_eq!(c.read_reg(REG_MIN), 0x00);
        assert_eq!(c.read_reg(REG_SEC), 0x00);
        assert_eq!(c.read_reg(REG_CENTURY), 0x20);
        assert_eq!(c.read_reg(REG_YEAR), 0x24);
        assert_eq!(c.read_reg(REG_MONTH), 0x01);
        assert_eq!(c.read_reg(REG_DAY_OF_MONTH), 0x01);
        assert_eq!(c.read_reg(REG_DAY_OF_WEEK), 0x07);
    }

    /// Spec: 12-hour + BCD — 12:59:59 AM → 1:00:00 AM (no day carry).
    #[test]
    fn twelve_hour_bcd_midnight_to_one_am() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_STATUS_B, 0x00);
        c.write_reg(REG_DAY_OF_MONTH, 0x10);
        c.write_reg(REG_DAY_OF_WEEK, 0x02);
        c.write_reg(REG_HOUR, 0x12); // 12 AM
        c.write_reg(REG_MIN, 0x59);
        c.write_reg(REG_SEC, 0x59);
        let _ = c.tick_second();
        assert_eq!(c.read_reg(REG_HOUR), 0x01); // 1 AM
        assert_eq!(c.read_reg(REG_DAY_OF_MONTH), 0x10);
        assert_eq!(c.read_reg(REG_DAY_OF_WEEK), 0x02);
    }

    /// Spec: 12-hour + binary (`DM=1`) — 11:59:59 AM → 12:00:00 PM (`0x8C`).
    #[test]
    fn twelve_hour_binary_noon_edge() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_STATUS_B, STB_DM); // 12-hour + binary
        c.write_reg(REG_DAY_OF_MONTH, 15);
        c.write_reg(REG_HOUR, 11); // 11 AM binary
        c.write_reg(REG_MIN, 59);
        c.write_reg(REG_SEC, 59);
        let _ = c.tick_second();
        assert_eq!(c.read_reg(REG_HOUR), 0x8C); // 12 | PM
        assert_eq!(c.read_reg(REG_MIN), 0);
        assert_eq!(c.read_reg(REG_SEC), 0);
        assert_eq!(c.read_reg(REG_DAY_OF_MONTH), 15);
    }

    /// Spec: 12-hour + binary — 11:59:59 PM → 12:00:00 AM + day carry.
    #[test]
    fn twelve_hour_binary_midnight_edge_carries_calendar() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_STATUS_B, STB_DM);
        c.write_reg(REG_CENTURY, 20);
        c.write_reg(REG_YEAR, 23);
        c.write_reg(REG_MONTH, 12);
        c.write_reg(REG_DAY_OF_MONTH, 31);
        c.write_reg(REG_DAY_OF_WEEK, 6);
        c.write_reg(REG_HOUR, 0x8B); // 11 | PM
        c.write_reg(REG_MIN, 59);
        c.write_reg(REG_SEC, 59);
        let _ = c.tick_second();
        assert_eq!(c.read_reg(REG_HOUR), 12); // 12 AM
        assert_eq!(c.read_reg(REG_HOUR) & HOUR_PM, 0);
        assert_eq!(c.read_reg(REG_YEAR), 24);
        assert_eq!(c.read_reg(REG_MONTH), 1);
        assert_eq!(c.read_reg(REG_DAY_OF_MONTH), 1);
        assert_eq!(c.read_reg(REG_DAY_OF_WEEK), 7);
    }

    /// Spec: 12-hour + binary cascade through sec/min into hour (10→11 AM).
    #[test]
    fn twelve_hour_binary_tick_cascades_sec_min_hour() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_STATUS_B, STB_DM);
        c.write_reg(REG_HOUR, 10);
        c.write_reg(REG_MIN, 59);
        c.write_reg(REG_SEC, 59);
        let _ = c.tick_second();
        assert_eq!(c.read_reg(REG_HOUR), 11);
        assert_eq!(c.read_reg(REG_MIN), 0);
        assert_eq!(c.read_reg(REG_SEC), 0);
    }

    /// Spec: Status B SET inhibits the update cycle in 12-hour mode too.
    #[test]
    fn twelve_hour_set_inhibits_calendar_advance() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_STATUS_B, STB_SET); // 12-hour + SET
        c.write_reg(REG_HOUR, 0x11);
        c.write_reg(REG_MIN, 0x59);
        c.write_reg(REG_SEC, 0x59);
        assert!(!c.tick_second());
        assert_eq!(c.read_reg(REG_HOUR), 0x11);
        assert_eq!(c.read_reg(REG_MIN), 0x59);
        assert_eq!(c.read_reg(REG_SEC), 0x59);
        assert_eq!(c.read_reg(REG_STATUS_C) & STC_UF, 0);
        assert_eq!(c.read_reg(REG_STATUS_A) & STATUS_A_UIP, 0);
    }

    /// Spec: with 24/12 set, hour counting stays 0–23 (regression vs 12-hour path).
    #[test]
    fn twenty_four_hour_bit_keeps_0_23_cascade() {
        let mut c = CmosRtc::new();
        assert_ne!(c.read_reg(REG_STATUS_B) & STB_24_12, 0);
        c.write_reg(REG_HOUR, 0x23);
        c.write_reg(REG_MIN, 0x59);
        c.write_reg(REG_SEC, 0x59);
        let _ = c.tick_second();
        assert_eq!(c.read_reg(REG_HOUR), 0x00);
        assert_eq!(c.read_reg(REG_MIN), 0x00);
        assert_eq!(c.read_reg(REG_SEC), 0x00);
    }

    // --- Status B 24/12 toggle: auto hour (+ alarm) conversion ---------------
    // Spec: MC146818 hour encodings (24h 0–23 vs 12h 1–12 + bit7 PM) under DM.
    // Silicon requires reinitializing hours after changing 24/12; this model
    // converts current hour and hour-alarm so guests can toggle the bit safely.

    /// Spec encodings: 24h BCD → 12h BCD (0→12 AM, 13→1 PM, 12→12 PM).
    #[test]
    fn status_b_24_to_12_converts_bcd_hour_and_alarm() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_HOUR, 0x13); // 13:00 24h BCD
        c.write_reg(REG_HOUR_ALARM, 0x00); // midnight alarm
                                           // Clear 24/12 → 12-hour; DM remains BCD.
        c.write_reg(REG_STATUS_B, 0x00);
        assert_eq!(c.read_reg(REG_STATUS_B) & STB_24_12, 0);
        assert_eq!(c.read_reg(REG_HOUR), HOUR_PM | 0x01); // 1 PM
        assert_eq!(c.read_reg(REG_HOUR_ALARM), 0x12); // 12 AM
    }

    /// Spec encodings: 12h BCD → 24h BCD (1 PM → 13, 12 AM → 0).
    #[test]
    fn status_b_12_to_24_converts_bcd_hour_and_alarm() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_STATUS_B, 0x00); // enter 12h first (also converts zeros)
        c.write_reg(REG_HOUR, HOUR_PM | 0x01); // 1 PM
        c.write_reg(REG_HOUR_ALARM, 0x12); // 12 AM
        c.write_reg(REG_STATUS_B, STB_24_12); // back to 24h
        assert_ne!(c.read_reg(REG_STATUS_B) & STB_24_12, 0);
        assert_eq!(c.read_reg(REG_HOUR), 0x13);
        assert_eq!(c.read_reg(REG_HOUR_ALARM), 0x00);
    }

    /// Spec: DM=1 binary hour bytes convert both directions with AM/PM bit7.
    #[test]
    fn status_b_24_12_toggle_converts_binary_dm_hours() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_STATUS_B, DEFAULT_STATUS_B | STB_DM);
        c.write_reg(REG_HOUR, 0); // midnight binary
        c.write_reg(REG_HOUR_ALARM, 23); // 11 PM
        c.write_reg(REG_STATUS_B, STB_DM); // 12h + binary
        assert_eq!(c.read_reg(REG_HOUR), 12); // 12 AM
        assert_eq!(c.read_reg(REG_HOUR_ALARM), HOUR_PM | 11); // 11 PM
        c.write_reg(REG_STATUS_B, DEFAULT_STATUS_B | STB_DM); // 24h + binary
        assert_eq!(c.read_reg(REG_HOUR), 0);
        assert_eq!(c.read_reg(REG_HOUR_ALARM), 23);
    }

    /// Spec: MC146818 alarm don't-care C0h–FFh is left alone on 24/12 toggle.
    #[test]
    fn status_b_24_12_toggle_preserves_alarm_dont_care() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_HOUR, 0x12); // noon 24h BCD
        c.write_reg(REG_HOUR_ALARM, 0xC0);
        c.write_reg(REG_STATUS_B, 0x00);
        assert_eq!(c.read_reg(REG_HOUR), HOUR_PM | 0x12); // 12 PM
        assert_eq!(c.read_reg(REG_HOUR_ALARM), 0xC0);
    }

    /// Spec: rewriting Status B without flipping 24/12 must not rewrite hours.
    #[test]
    fn status_b_write_without_24_12_flip_leaves_hours() {
        let mut c = CmosRtc::new();
        c.write_reg(REG_HOUR, 0x10);
        c.write_reg(REG_HOUR_ALARM, 0x11);
        c.write_reg(REG_STATUS_B, DEFAULT_STATUS_B | STB_PIE); // still 24h
        assert_eq!(c.read_reg(REG_HOUR), 0x10);
        assert_eq!(c.read_reg(REG_HOUR_ALARM), 0x11);
    }
}
