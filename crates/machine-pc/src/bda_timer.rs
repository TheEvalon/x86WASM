//! BDA timer tick count (`0040:006C`) advanced from PIT channel-0 IRQ0.
//!
//! Spec: IBM PC/AT Technical Reference — BIOS Data Area timer ticks;
//! Ralf Brown's Interrupt List MEM `0040h:006Ch` / `0070h`; Intel 8254 ch0 →
//! 8259 IR0.

use crate::{bda_kbd::BDA_PHYS_BASE, Machine, MachineError};

/// Timer ticks since midnight — dword at `0040:006C`.
pub const BDA_TIMER_TICKS: u64 = BDA_PHYS_BASE + 0x6C;

/// Timer overflow (midnight rolled) — byte at `0040:0070`.
pub const BDA_TIMER_OVERFLOW: u64 = BDA_PHYS_BASE + 0x70;

/// Ticks in 24 hours at the classic PC rate (≈18.2065 Hz).
pub const BDA_TICKS_PER_DAY: u32 = 0x0018_00B0;

impl Machine {
    /// Read the BDA timer tick dword (`0040:006C`).
    pub fn bda_timer_ticks(&self) -> Result<u32, MachineError> {
        let b0 = self
            .mem
            .read_u8(BDA_TIMER_TICKS)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        let b1 = self
            .mem
            .read_u8(BDA_TIMER_TICKS + 1)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        let b2 = self
            .mem
            .read_u8(BDA_TIMER_TICKS + 2)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        let b3 = self
            .mem
            .read_u8(BDA_TIMER_TICKS + 3)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        Ok(u32::from_le_bytes([b0, b1, b2, b3]))
    }

    /// Write the BDA timer tick dword (`0040:006C`).
    pub fn set_bda_timer_ticks(&mut self, ticks: u32) -> Result<(), MachineError> {
        let [b0, b1, b2, b3] = ticks.to_le_bytes();
        self.mem
            .write_u8(BDA_TIMER_TICKS, b0)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        self.mem
            .write_u8(BDA_TIMER_TICKS + 1, b1)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        self.mem
            .write_u8(BDA_TIMER_TICKS + 2, b2)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        self.mem
            .write_u8(BDA_TIMER_TICKS + 3, b3)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        Ok(())
    }

    /// Read the BDA midnight-overflow flag (`0040:0070`).
    pub fn bda_timer_overflow(&self) -> Result<u8, MachineError> {
        self.mem
            .read_u8(BDA_TIMER_OVERFLOW)
            .map_err(|_| MachineError::MbrRamTooSmall)
    }

    /// Clear BDA timer ticks and overflow (host / cold-start helper).
    pub fn clear_bda_timer_ticks(&mut self) -> Result<(), MachineError> {
        self.set_bda_timer_ticks(0)?;
        self.mem
            .write_u8(BDA_TIMER_OVERFLOW, 0)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        Ok(())
    }

    /// Advance BDA `40:6C` by one IRQ0 tick; wrap at 24 h and set `40:70`.
    pub fn advance_bda_timer_tick(&mut self) -> Result<(), MachineError> {
        let mut ticks = self.bda_timer_ticks()?;
        ticks = ticks.wrapping_add(1);
        if ticks >= BDA_TICKS_PER_DAY {
            ticks = 0;
            self.mem
                .write_u8(BDA_TIMER_OVERFLOW, 1)
                .map_err(|_| MachineError::MbrRamTooSmall)?;
        }
        self.set_bda_timer_ticks(ticks)
    }

    /// Called from [`Self::tick_pit`] on a rising PIT ch0 OUT edge.
    pub(crate) fn on_pit_irq0_rising_bda_tick(&mut self) {
        let _ = self.advance_bda_timer_tick();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use devices::{PortDevice, PIT_CH0_DATA, PIT_CONTROL};

    fn arm_pit_mode0_count4(m: &mut Machine) {
        m.pit.port_write(PIT_CONTROL, 1, 0x30);
        m.pit.port_write(PIT_CH0_DATA, 1, 0x04);
        m.pit.port_write(PIT_CH0_DATA, 1, 0x00);
    }

    #[test]
    fn pit_irq0_rising_advances_bda_ticks() {
        let mut m = Machine::new(64 * 1024);
        m.clear_bda_timer_ticks().unwrap();
        arm_pit_mode0_count4(&mut m);
        assert_eq!(m.bda_timer_ticks().unwrap(), 0);
        assert!(!m.pit.out_ch0());
        m.tick_pit(5);
        assert!(m.pit.out_ch0());
        assert_eq!(m.bda_timer_ticks().unwrap(), 1);
    }

    #[test]
    fn midnight_wrap_sets_overflow_flag() {
        let mut m = Machine::new(64 * 1024);
        m.set_bda_timer_ticks(BDA_TICKS_PER_DAY - 1).unwrap();
        m.mem.write_u8(BDA_TIMER_OVERFLOW, 0).unwrap();
        m.advance_bda_timer_tick().unwrap();
        assert_eq!(m.bda_timer_ticks().unwrap(), 0);
        assert_eq!(m.bda_timer_overflow().unwrap(), 1);
    }

    #[test]
    fn advance_helper_increments_dword() {
        let mut m = Machine::new(64 * 1024);
        m.clear_bda_timer_ticks().unwrap();
        m.advance_bda_timer_tick().unwrap();
        m.advance_bda_timer_tick().unwrap();
        assert_eq!(m.bda_timer_ticks().unwrap(), 2);
        assert_eq!(m.bda_timer_overflow().unwrap(), 0);
    }
}
