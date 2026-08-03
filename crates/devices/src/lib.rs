//! Device models. Milestone 1: COM1 data port + debug port 0x402.
//! Milestone 2 (partial): 8259 PIC ICW+OCW/IRQ; 8254 PIT ch0+ch2/port 0x61 speaker;
//! CMOS/RTC IRQ8; 8042/PS2 controller on MachineBus ports 0x60/0x64 with IRQ1.

#![forbid(unsafe_code)]

mod cmos;
mod i8042;
mod pic;
mod pit;
mod serial;

pub use cmos::{
    CmosRtc, CMOS_DATA, CMOS_INDEX, REG_STATUS_A, REG_STATUS_B, REG_STATUS_C, REG_STATUS_D,
    STB_AIE, STB_PIE, STB_SET, STB_UIE, STC_AF, STC_IRQF, STC_PF, STC_UF,
};
pub use i8042::{
    CFG_INT1, CFG_INT12, CMD_DISABLE_KBD, CMD_ENABLE_KBD, CMD_READ_CONFIG, CMD_SELF_TEST,
    CMD_WRITE_CONFIG, I8042, I8042_DATA, I8042_STATUS_CMD, SELF_TEST_OK, STATUS_CMD, STATUS_IBF,
    STATUS_OBF, STATUS_SYS,
};
pub use pic::{DualPic, Pic8259, PIC_MASTER_CMD, PIC_MASTER_DATA, PIC_SLAVE_CMD, PIC_SLAVE_DATA};
pub use pit::{
    Pit8254, PitChannel, PIT_CH0_DATA, PIT_CH1_DATA, PIT_CH2_DATA, PIT_CONTROL, PORT61_GATE2,
    PORT61_OUT2, PORT61_SPKR_DATA, PORT_SYSTEM_CONTROL,
};
pub use serial::{DebugConsole, Serial16550, SerialOutput};

/// Port I/O sink shared by CLI and browser.
pub trait PortDevice {
    fn port_read(&mut self, port: u16, size: u8) -> u32;
    fn port_write(&mut self, port: u16, size: u8, value: u32);
}
