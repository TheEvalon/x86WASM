//! Dual Intel 8237A ISA DMA controllers — registers/pages + single-channel helper.
//!
//! Classic PC: master (ch0–3) at `0x00`–`0x0F`, slave (ch4–7) at even ports
//! `0xC0`–`0xDE`, plus external page address registers.
//!
//! # Spec refs
//!
//! - Intel 8237A High Performance Programmable DMA Controller — address/count
//!   registers, byte pointer flip-flop, mode/mask/command, Request Register,
//!   Status Register, Master Clear, terminal count (word count = N−1), current
//!   address/count after transfer.
//! - OSDev Wiki ISA DMA — AT port map and page register assignment
//!   (`0x87`/`0x83`/`0x81`/`0x82` ch0–3; `0x8F`/`0x8B`/`0x89`/`0x8A` ch4–7);
//!   8-bit channel physical address `(page << 16) | addr`; 16-bit channels 4–7
//!   use word address/count with phys `(page << 16) | (addr << 1)`.
//! - IBM PC/AT: port `0x80` is POST/diagnostic scratch, **not** a DMA page.
//! - `docs/machine-model-pc-v1.md`, `plan.md` §15.3 / §21 DMA.
//!
//! # Scope (this slice)
//!
//! - Dual 8237A programming model: addr/count with flip-flop, mode, masks,
//!   command, software Request Register set/reset, status read, master/mask reset.
//! - Base Address / Base Word Count loaded whenever Current is programmed
//!   (Intel 8237A programming model).
//! - Status register TC bits (3:0) clear-on-read + `latch_tc` device/test API;
//!   request bits (7:4) persist until their software request is reset.
//! - Mode register programming (including Cascade bits 7:6 = `11`) on master
//!   channels 0–3 and slave channels 0–3 (ISA 4–7); ISA channel 4 is the AT
//!   cascade channel and may be programmed like any other channel.
//! - `transfer_block` software helper for 8-bit channels 0–3 and 16-bit
//!   channels 5–7: Demand/Single/Block + Increment/Decrement + Verify/Read/Write,
//!   Autoinitialize optional. Read/Write use `mem_read` / `mem_write`
//!   callbacks; Verify advances address/count and latches TC without memory
//!   R/W (`io_buf` length still checked; payload bytes left untouched). With
//!   Autoinitialize, after TC Current is reloaded from Base and the channel
//!   stays unmasked/ready for another helper call. Without Autoinitialize,
//!   after TC the channel mask bit is set (Intel 8237A hardware auto-mask).
//!   Demand and Block complete the programmed count in one helper call with
//!   the same TC/mask/autoinit rules as Single (DREQ hold/release not modeled).
//!   - 8-bit (ch0–3): length `count+1` bytes; phys `(page << 16) | addr`;
//!     address steps ±1 per byte within the 64 KiB page.
//!   - 16-bit (ch5–7, AT cascade slave data channels): length `2*(count+1)` bytes;
//!     Current Address is a word address; phys `(page << 16) | (addr << 1)` (A0=0);
//!     address steps ±1 per word; page register pairing as for AT page ports.
//! - Page address register R/W for the eight AT channels above.
//! - `PortDevice` for MachineBus wiring.
//!
//! # Unsupported (explicit)
//!
//! - Hardware DREQ/DACK handshake / cycle-accurate bus timing
//! - Cascade mode (mode bits 7:6 = `11`) for `transfer_block`
//! - ISA channel 4 `transfer_block` (AT cascade reserved; programming only)
//! - Floppy / IDE automatic DMA engine / DREQ path (Machine PhysMem wiring lives in
//!   `machine-pc::Machine::dma_transfer`; no SeaBIOS floppy DMA)

use crate::PortDevice;

/// Error from [`Dma8237::transfer_block`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmaTransferError {
    /// ISA channel outside `0`–`7`.
    BadChannel,
    /// Channel mask bit is set (masked channels do not transfer).
    Masked,
    /// Mode register outside Demand/Single/Block + Inc/Dec + Verify/Read/Write
    /// (+ optional Autoinitialize); Cascade and ISA channel 4 are rejected.
    UnsupportedMode,
    /// `io_buf` shorter than programmed transfer length in bytes
    /// (`count+1` for 8-bit ch0–3; `2*(count+1)` for 16-bit ch5–7).
    BufferTooShort,
}

/// Master 8237A base (channels 0–3).
pub const DMA_MASTER_BASE: u16 = 0x00;
/// Slave 8237A base (channels 4–7); addressed on even ports only.
pub const DMA_SLAVE_BASE: u16 = 0xC0;

/// Page address ports (IBM PC/AT). Spec: OSDev ISA DMA.
pub const DMA_PAGE_CH0: u16 = 0x87;
pub const DMA_PAGE_CH1: u16 = 0x83;
pub const DMA_PAGE_CH2: u16 = 0x81;
pub const DMA_PAGE_CH3: u16 = 0x82;
pub const DMA_PAGE_CH4: u16 = 0x8F;
pub const DMA_PAGE_CH5: u16 = 0x8B;
pub const DMA_PAGE_CH6: u16 = 0x89;
pub const DMA_PAGE_CH7: u16 = 0x8A;

/// One 8237A channel: current + base address/count and mode.
///
/// Spec: Intel 8237A — programming Current Address / Current Word Count also
/// loads Base Address / Base Word Count. Autoinitialize restores Current from
/// Base at terminal count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DmaChannel {
    /// Current Address (software-readable via addr ports).
    pub addr: u16,
    /// Current Word Count (software-readable via count ports).
    pub count: u16,
    /// Base Address (loaded with Current on program; used by Autoinitialize).
    pub base_addr: u16,
    /// Base Word Count (loaded with Current on program; used by Autoinitialize).
    pub base_count: u16,
    pub mode: u8,
}

/// One 8237A controller (master or slave role).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DmaController {
    pub channels: [DmaChannel; 4],
    /// Byte pointer flip-flop: false = low byte next, true = high byte next.
    pub flip_flop: bool,
    /// Command register (stored; transfer engine unsupported).
    pub command: u8,
    /// Status register: bits 3:0 = TC per channel (clear-on-read);
    /// bits 7:4 = software request pending (external DREQ path unsupported).
    pub status: u8,
    /// Software request pending bits 3:0, one bit per controller-local channel.
    pub request: u8,
    /// Channel mask bits 3:0 (1 = masked). Reset default: all masked.
    pub mask: u8,
    /// Temporary / intermediate register (master clear clears it).
    pub temporary: u8,
}

impl Default for DmaController {
    fn default() -> Self {
        Self::new()
    }
}

impl DmaController {
    pub fn new() -> Self {
        Self {
            channels: [DmaChannel::default(); 4],
            flip_flop: false,
            command: 0,
            status: 0,
            request: 0,
            mask: 0x0F,
            temporary: 0,
        }
    }

    /// Spec: Intel 8237A Master Clear — masks all channels and clears the byte
    /// pointer, command, status, software requests, and temporary register.
    pub fn master_reset(&mut self) {
        *self = Self::new();
    }

    /// Latch terminal-count status for `channel` (0–3).
    ///
    /// Spec: Intel 8237A status register bits 3:0 are set when a channel reaches
    /// TC. Without a DREQ/DACK transfer engine, firmware/tests use this hook to
    /// exercise clear-on-read status semantics. Does **not** move memory.
    pub fn latch_tc(&mut self, channel: usize) {
        debug_assert!(channel < 4);
        if channel < 4 {
            self.status |= 1 << channel;
        }
    }

    /// Apply an Intel 8237A Request Register command.
    ///
    /// Bits 1:0 select a controller-local channel; bit 2 sets (`1`) or resets
    /// (`0`) only that channel's software request. Channel masking is independent.
    fn write_request(&mut self, value: u8) {
        let channel = value & 0x03;
        let request_bit = 1u8 << channel;
        if value & 0x04 != 0 {
            self.request |= request_bit;
        } else {
            self.request &= !request_bit;
        }
        self.status = (self.status & 0x0F) | ((self.request & 0x0F) << 4);
    }

    fn clear_flip_flop(&mut self) {
        self.flip_flop = false;
    }

    fn next_byte_is_high(&mut self) -> bool {
        let high = self.flip_flop;
        self.flip_flop = !self.flip_flop;
        high
    }

    fn write_addr_count_byte(&mut self, ch: usize, is_count: bool, value: u8) {
        // Spec: Intel 8237A — a write to Current Address / Word Count also loads
        // the corresponding Base register (byte-wise via the flip-flop).
        let high = self.next_byte_is_high();
        let (cur, base) = if is_count {
            (
                &mut self.channels[ch].count,
                &mut self.channels[ch].base_count,
            )
        } else {
            (
                &mut self.channels[ch].addr,
                &mut self.channels[ch].base_addr,
            )
        };
        if high {
            *cur = (*cur & 0x00FF) | (u16::from(value) << 8);
            *base = (*base & 0x00FF) | (u16::from(value) << 8);
        } else {
            *cur = (*cur & 0xFF00) | u16::from(value);
            *base = (*base & 0xFF00) | u16::from(value);
        }
    }

    fn read_addr_count_byte(&mut self, ch: usize, is_count: bool) -> u8 {
        let high = self.next_byte_is_high();
        let reg = if is_count {
            self.channels[ch].count
        } else {
            self.channels[ch].addr
        };
        if high {
            (reg >> 8) as u8
        } else {
            reg as u8
        }
    }

    /// Internal offset 0–15 (master port = offset; slave port = base + 2*offset).
    fn port_read_offset(&mut self, offset: u8) -> u8 {
        match offset {
            0..=7 => {
                let ch = (offset / 2) as usize;
                let is_count = offset % 2 == 1;
                self.read_addr_count_byte(ch, is_count)
            }
            8 => {
                // Spec: Intel 8237A — status read returns TC (bits 3:0) + request
                // (bits 7:4); only TC bits clear on read. Software requests persist
                // until reset through the Request Register or Master Clear.
                let status = self.status;
                self.status &= !0x0F;
                status
            }
            13 => self.temporary,
            15 => self.mask & 0x0F,
            _ => 0xFF, // write-only / undefined reads
        }
    }

    fn port_write_offset(&mut self, offset: u8, value: u8) {
        match offset {
            0..=7 => {
                let ch = (offset / 2) as usize;
                let is_count = offset % 2 == 1;
                self.write_addr_count_byte(ch, is_count, value);
            }
            8 => self.command = value,
            9 => self.write_request(value),
            10 => {
                // Single-channel mask: bits1:0 = channel, bit2 = mask set/clear.
                let ch = value & 0x03;
                if value & 0x04 != 0 {
                    self.mask |= 1 << ch;
                } else {
                    self.mask &= !(1 << ch);
                }
            }
            11 => {
                let ch = (value & 0x03) as usize;
                self.channels[ch].mode = value;
            }
            12 => self.clear_flip_flop(),
            13 => self.master_reset(),
            14 => self.mask = 0, // clear mask register
            15 => self.mask = value & 0x0F,
            _ => {}
        }
    }
}

/// Dual 8237A + AT page registers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dma8237 {
    pub master: DmaController,
    pub slave: DmaController,
    /// Page registers indexed by ISA channel 0–7.
    pub page: [u8; 8],
}

impl Default for Dma8237 {
    fn default() -> Self {
        Self::new()
    }
}

impl Dma8237 {
    pub fn new() -> Self {
        Self {
            master: DmaController::new(),
            slave: DmaController::new(),
            page: [0; 8],
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Latch TC status for ISA channel `0`–`7` (master 0–3 / slave 4–7).
    ///
    /// Device-level test/firmware-probe hook; also used by [`Self::transfer_block`]
    /// on completion. Spec: Intel 8237A status register TC bits.
    pub fn latch_tc(&mut self, isa_channel: usize) {
        match isa_channel {
            0..=3 => self.master.latch_tc(isa_channel),
            4..=7 => self.slave.latch_tc(isa_channel - 4),
            _ => {}
        }
    }

    /// Complete one programmed channel transfer via memory callbacks.
    ///
    /// Spec: Intel 8237A — word count holds N−1 so the unit count is `count + 1`;
    /// after N transfers the channel TC status bit is set. Without Autoinitialize,
    /// Current Address ends at the post-step value, Current Word Count is
    /// `0xFFFF`, and the channel is hardware-masked (mask bit set). With
    /// Autoinitialize (mode bit 4), Current Address / Word Count are restored
    /// from Base Address / Base Word Count at TC and the channel remains ready
    /// (not auto-masked). Address direction follows mode bit 5: `0` increments
    /// Current Address by +1 per unit, `1` decrements by −1 (page register is
    /// not auto-bumped).
    ///
    /// # Address / length (OSDev ISA DMA + AT page regs)
    ///
    /// - **8-bit channels 0–3:** unit = byte; phys = `(page << 16) | addr`;
    ///   returns `count+1` bytes.
    /// - **16-bit channels 5–7:** unit = word; Current Address is a word address;
    ///   phys = `(page << 16) | (addr << 1)` (A0 forced 0); each unit moves two
    ///   LE bytes; returns `2*(count+1)` bytes. Word address wraps as `u16`.
    /// - **ISA channel 4:** AT cascade reserved (OSDev ISA DMA / IBM PC/AT); mode
    ///   and page/addr/count may be programmed, but this helper always returns
    ///   [`DmaTransferError::UnsupportedMode`].
    ///
    /// # Mode subset honored
    ///
    /// - bits 7:6 = Demand (`00`), Single (`01`), or Block (`10`)
    ///   (Cascade `11` rejected). Spec: Intel 8237A mode register — Demand and
    ///   Block differ in DREQ service timing; this helper completes all
    ///   programmed units in one call with Single-equivalent TC/mask/autoinit.
    /// - bit 5 = address increment (`0`) or decrement (`1`)
    /// - bit 4 = Autoinitialize enable (`0` or `1`)
    /// - bits 3:2 = Verify (`00`, no memory R/W; `io_buf` length still checked),
    ///   Write (`01`, I/O→memory from `io_buf` via `mem_write`), or
    ///   Read (`10`, memory→I/O into `io_buf` via `mem_read`)
    /// - bits 1:0 must match the controller channel index (master 0–3 / slave 0–3)
    ///
    /// Cascade mode returns [`DmaTransferError::UnsupportedMode`].
    /// ISA channel 4 returns [`DmaTransferError::UnsupportedMode`].
    /// ISA channel `> 7` returns [`BadChannel`].
    ///
    /// This is a software helper for device/unit tests — **not** DREQ/DACK timing.
    pub fn transfer_block<R, W>(
        &mut self,
        isa_channel: usize,
        io_buf: &mut [u8],
        mut mem_read: R,
        mut mem_write: W,
    ) -> Result<usize, DmaTransferError>
    where
        R: FnMut(u32) -> u8,
        W: FnMut(u32, u8),
    {
        if isa_channel > 7 {
            return Err(DmaTransferError::BadChannel);
        }
        // Spec: OSDev ISA DMA / IBM PC/AT — channel 4 cascades the 8-bit master
        // through the 16-bit slave; it is not a data-transfer channel.
        if isa_channel == 4 {
            return Err(DmaTransferError::UnsupportedMode);
        }
        let word16 = isa_channel >= 5;
        let ctrl_ch = if word16 { isa_channel - 4 } else { isa_channel };
        let (mask, mode, count, addr0) = if word16 {
            let ch = &self.slave.channels[ctrl_ch];
            (self.slave.mask, ch.mode, ch.count, ch.addr)
        } else {
            let ch = &self.master.channels[ctrl_ch];
            (self.master.mask, ch.mode, ch.count, ch.addr)
        };
        if mask & (1 << ctrl_ch) != 0 {
            return Err(DmaTransferError::Masked);
        }
        if !Self::mode_supports_transfer_block(mode, ctrl_ch) {
            return Err(DmaTransferError::UnsupportedMode);
        }
        let units = usize::from(count).wrapping_add(1);
        let byte_len = if word16 {
            units.saturating_mul(2)
        } else {
            units
        };
        if io_buf.len() < byte_len {
            return Err(DmaTransferError::BufferTooShort);
        }
        let page = self.page[isa_channel];
        let xfer = (mode >> 2) & 0x03;
        // Spec: Intel 8237A mode bits 3:2 — Verify (00) / Write (01) / Read (10).
        let verify = xfer == 0b00;
        let write_to_mem = xfer == 0b01;
        let auto_init = (mode >> 4) & 1 != 0;
        let decrement = (mode >> 5) & 1 != 0;
        let mut addr = addr0;
        let mut io_off = 0usize;
        for _ in 0..units {
            if !verify {
                let phys = if word16 {
                    // Spec: OSDev ISA DMA — 16-bit channel word address → byte phys.
                    (u32::from(page) << 16) | (u32::from(addr) << 1)
                } else {
                    (u32::from(page) << 16) | u32::from(addr)
                };
                if word16 {
                    if write_to_mem {
                        mem_write(phys, io_buf[io_off]);
                        mem_write(phys.wrapping_add(1), io_buf[io_off + 1]);
                    } else {
                        io_buf[io_off] = mem_read(phys);
                        io_buf[io_off + 1] = mem_read(phys.wrapping_add(1));
                    }
                } else if write_to_mem {
                    mem_write(phys, io_buf[io_off]);
                } else {
                    io_buf[io_off] = mem_read(phys);
                }
            }
            io_off += if word16 { 2 } else { 1 };
            // Spec: Intel 8237A mode bit 5 — address increment (0) / decrement (1).
            addr = if decrement {
                addr.wrapping_sub(1)
            } else {
                addr.wrapping_add(1)
            };
        }
        if word16 {
            if auto_init {
                let ch = &mut self.slave.channels[ctrl_ch];
                ch.addr = ch.base_addr;
                ch.count = ch.base_count;
            } else {
                self.slave.channels[ctrl_ch].addr = addr;
                self.slave.channels[ctrl_ch].count = 0xFFFF;
                self.slave.mask |= 1 << ctrl_ch;
            }
            self.slave.latch_tc(ctrl_ch);
        } else if auto_init {
            let ch = &mut self.master.channels[ctrl_ch];
            ch.addr = ch.base_addr;
            ch.count = ch.base_count;
            self.master.latch_tc(ctrl_ch);
        } else {
            self.master.channels[ctrl_ch].addr = addr;
            self.master.channels[ctrl_ch].count = 0xFFFF;
            self.master.mask |= 1 << ctrl_ch;
            self.master.latch_tc(ctrl_ch);
        }
        Ok(byte_len)
    }

    /// Demand/Single/Block + Inc/Dec + Verify/Read/Write, Autoinitialize optional,
    /// channel matches. Spec: Intel 8237A mode bits 7:6 — Cascade (`11`) rejected.
    fn mode_supports_transfer_block(mode: u8, ctrl_channel: usize) -> bool {
        let sel = (mode & 0x03) as usize;
        let xfer = (mode >> 2) & 0x03;
        let mode_sel = (mode >> 6) & 0x03;
        sel == ctrl_channel
            && (mode_sel == 0b00 || mode_sel == 0b01 || mode_sel == 0b10)
            && (xfer == 0b00 || xfer == 0b01 || xfer == 0b10)
    }

    fn page_channel(port: u16) -> Option<usize> {
        match port {
            DMA_PAGE_CH0 => Some(0),
            DMA_PAGE_CH1 => Some(1),
            DMA_PAGE_CH2 => Some(2),
            DMA_PAGE_CH3 => Some(3),
            DMA_PAGE_CH4 => Some(4),
            DMA_PAGE_CH5 => Some(5),
            DMA_PAGE_CH6 => Some(6),
            DMA_PAGE_CH7 => Some(7),
            _ => None,
        }
    }

    /// True if this device owns the I/O port (not POST `0x80` or unused page holes).
    pub fn owns_port(port: u16) -> bool {
        matches!(port, 0x00..=0x0F)
            || ((0xC0..=0xDE).contains(&port) && port.is_multiple_of(2))
            || Self::page_channel(port).is_some()
    }

    fn owns_slave_port(port: u16) -> bool {
        (0xC0..=0xDE).contains(&port) && port.is_multiple_of(2)
    }
}

impl PortDevice for Dma8237 {
    fn port_read(&mut self, port: u16, _size: u8) -> u32 {
        if let Some(ch) = Self::page_channel(port) {
            return u32::from(self.page[ch]);
        }
        if matches!(port, 0x00..=0x0F) {
            return u32::from(self.master.port_read_offset(port as u8));
        }
        if Self::owns_slave_port(port) {
            let offset = ((port - 0xC0) / 2) as u8;
            return u32::from(self.slave.port_read_offset(offset));
        }
        0xFFFFFFFF
    }

    fn port_write(&mut self, port: u16, _size: u8, value: u32) {
        let v = value as u8;
        if let Some(ch) = Self::page_channel(port) {
            self.page[ch] = v;
            return;
        }
        if matches!(port, 0x00..=0x0F) {
            self.master.port_write_offset(port as u8, v);
            return;
        }
        if Self::owns_slave_port(port) {
            let offset = ((port - 0xC0) / 2) as u8;
            self.slave.port_write_offset(offset, v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn flip_flop_addr_write_readback() {
        // Spec: Intel 8237A — successive low/high byte accesses via flip-flop.
        let mut d = Dma8237::new();
        d.port_write(0x00, 1, 0x34);
        d.port_write(0x00, 1, 0x12);
        d.master.clear_flip_flop();
        assert_eq!(d.port_read(0x00, 1) as u8, 0x34);
        assert_eq!(d.port_read(0x00, 1) as u8, 0x12);
        assert_eq!(d.master.channels[0].addr, 0x1234);
    }

    #[test]
    fn page_ch2_round_trip() {
        // Spec: OSDev ISA DMA — page port 0x81 = channel 2.
        let mut d = Dma8237::new();
        d.port_write(DMA_PAGE_CH2, 1, 0xAB);
        assert_eq!(d.port_read(DMA_PAGE_CH2, 1) as u8, 0xAB);
        assert_eq!(d.page[2], 0xAB);
    }

    #[test]
    fn master_reset_masks_and_clears_flip_flop() {
        // Spec: Intel 8237A master clear (write 0x0D).
        let mut d = Dma8237::new();
        d.port_write(0x00, 1, 0x11);
        d.port_write(0x0A, 1, 0x00); // unmask ch0
        assert_eq!(d.master.mask & 0x01, 0);
        d.port_write(0x0D, 1, 0x00);
        assert_eq!(d.master.mask, 0x0F);
        assert!(!d.master.flip_flop);
        assert_eq!(d.master.channels[0].addr, 0);
    }

    #[test]
    fn single_and_multi_mask() {
        let mut d = Dma8237::new();
        assert_eq!(d.port_read(0x0F, 1) as u8, 0x0F);
        d.port_write(0x0A, 1, 0x00); // clear mask ch0
        assert_eq!(d.master.mask & 0x01, 0);
        d.port_write(0x0A, 1, 0x04); // set mask ch0
        assert_eq!(d.master.mask & 0x01, 0x01);
        d.port_write(0x0F, 1, 0x05);
        assert_eq!(d.port_read(0x0F, 1) as u8, 0x05);
        d.port_write(0x0E, 1, 0x00); // clear all masks
        assert_eq!(d.master.mask, 0);
    }

    #[test]
    fn slave_addr_via_even_ports() {
        // Spec: OSDev ISA DMA — slave addr ch5 at 0xC4 (offset 2).
        let mut d = Dma8237::new();
        d.port_write(0xC4, 1, 0x78);
        d.port_write(0xC4, 1, 0x56);
        d.slave.clear_flip_flop();
        assert_eq!(d.port_read(0xC4, 1) as u8, 0x78);
        assert_eq!(d.port_read(0xC4, 1) as u8, 0x56);
        assert_eq!(d.slave.channels[1].addr, 0x5678);
    }

    #[test]
    fn does_not_own_post_port_80() {
        // Spec: IBM PC/AT — 0x80 is POST, not a DMA page register.
        assert!(!Dma8237::owns_port(0x80));
        assert!(!Dma8237::owns_port(0x84));
        assert!(Dma8237::owns_port(DMA_PAGE_CH2));
        assert!(Dma8237::owns_port(0x00));
        assert!(Dma8237::owns_port(0xC0));
    }

    #[test]
    fn status_tc_clear_after_reset() {
        // Spec: Intel 8237A — after master clear / power-on, TC bits are 0.
        let mut d = Dma8237::new();
        assert_eq!(d.port_read(0x08, 1) as u8, 0);
    }

    #[test]
    fn status_tc_bits_readable_and_clear_on_read() {
        // Spec: Intel 8237A status register — bits 3:0 set at TC; cleared by
        // reading status (port 0x08). No DREQ/DACK transfer; latch via API.
        let mut d = Dma8237::new();
        d.latch_tc(2); // ISA ch2 (floppy channel) on master
        assert_eq!(d.master.status & 0x0F, 0x04);
        assert_eq!(d.port_read(0x08, 1) as u8 & 0x0F, 0x04);
        assert_eq!(d.master.status & 0x0F, 0);
        assert_eq!(d.port_read(0x08, 1) as u8 & 0x0F, 0);
    }

    #[test]
    fn status_tc_multi_channel_and_master_reset() {
        let mut d = Dma8237::new();
        d.latch_tc(0);
        d.latch_tc(3);
        assert_eq!(d.port_read(0x08, 1) as u8 & 0x0F, 0x09);
        d.latch_tc(1);
        d.port_write(0x0D, 1, 0x00); // master clear
        assert_eq!(d.port_read(0x08, 1) as u8 & 0x0F, 0);
    }

    #[test]
    fn slave_status_tc_clear_on_read() {
        // Spec: Intel 8237A slave status at offset 8 → port 0xD0.
        let mut d = Dma8237::new();
        d.latch_tc(5); // slave channel 1 (ISA ch5)
        assert_eq!(d.port_read(0xD0, 1) as u8 & 0x0F, 0x02);
        assert_eq!(d.port_read(0xD0, 1) as u8 & 0x0F, 0);
    }

    #[test]
    fn software_request_set_reset_each_master_channel() {
        // Spec: Intel 8237A Request Register — bits 1:0 select the channel and
        // bit 2 sets (1) or resets (0) that channel's software request.
        let mut d = Dma8237::new();
        let mut expected = 0u8;
        for channel in 0u8..4 {
            let request_bit = 1u8 << channel;
            expected |= request_bit;

            d.port_write(0x09, 1, u32::from(0x04 | channel));
            assert_eq!(d.master.request, expected, "set channel {channel}");
            assert_eq!(
                d.port_read(0x08, 1) as u8 & 0xF0,
                expected << 4,
                "status channel {channel}"
            );
            assert_eq!(
                d.port_read(0x08, 1) as u8 & 0xF0,
                expected << 4,
                "status read preserves channel {channel} request"
            );
        }

        for channel in 0u8..4 {
            expected &= !(1u8 << channel);
            d.port_write(0x09, 1, u32::from(channel));
            assert_eq!(d.master.request, expected, "reset channel {channel}");
            assert_eq!(d.port_read(0x08, 1) as u8 & 0xF0, expected << 4);
        }
    }

    #[test]
    fn software_request_set_reset_each_slave_channel() {
        // Spec: Intel 8237A Request Register at slave offset 9 → PC/AT port
        // 0xD2. The two selector bits address slave-local channels 0–3
        // (ISA channels 4–7), reflected in slave status at 0xD0.
        let mut d = Dma8237::new();
        let mut expected = 0u8;
        for channel in 0u8..4 {
            let request_bit = 1u8 << channel;
            expected |= request_bit;

            d.port_write(0xD2, 1, u32::from(0x04 | channel));
            assert_eq!(d.slave.request, expected, "set slave channel {channel}");
            assert_eq!(d.master.request, 0, "master remains independent");
            assert_eq!(
                d.port_read(0xD0, 1) as u8 & 0xF0,
                expected << 4,
                "slave status channel {channel}"
            );
        }

        for channel in 0u8..4 {
            expected &= !(1u8 << channel);
            d.port_write(0xD2, 1, u32::from(channel));
            assert_eq!(d.slave.request, expected, "reset slave channel {channel}");
            assert_eq!(d.port_read(0xD0, 1) as u8 & 0xF0, expected << 4);
        }
    }

    #[test]
    fn software_requests_are_independent_of_channel_masks() {
        // Spec: Intel 8237A Request Register and Mask Register are independent:
        // a masked channel can hold a software request, and mask writes do not
        // set or reset that request.
        let mut d = Dma8237::new();
        assert_eq!(d.master.mask, 0x0F);

        d.port_write(0x09, 1, 0x06); // set request ch2 while masked
        assert_eq!(d.master.request, 0x04);
        assert_eq!(d.master.mask, 0x0F);
        assert_eq!(d.port_read(0x08, 1) as u8, 0x40);

        d.port_write(0x0A, 1, 0x02); // unmask ch2
        assert_eq!(d.master.mask, 0x0B);
        assert_eq!(d.master.request, 0x04);
        assert_eq!(d.port_read(0x08, 1) as u8, 0x40);

        d.port_write(0x09, 1, 0x02); // reset request ch2
        assert_eq!(d.master.request, 0);
        assert_eq!(d.master.mask, 0x0B);
    }

    #[test]
    fn status_read_clears_tc_only_and_preserves_request_bits() {
        // Spec: Intel 8237A Status Register — bits 7:4 report requests and bits
        // 3:0 report terminal count. A status read clears only the TC bits.
        let mut d = Dma8237::new();
        d.latch_tc(1); // TC ch1 → status bit 1
        d.port_write(0x09, 1, 0x07); // set request ch3 → status bit 7

        assert_eq!(d.port_read(0x08, 1) as u8, 0x82);
        assert_eq!(d.master.status & 0x0F, 0);
        assert_eq!(d.master.request, 0x08);
        assert_eq!(d.port_read(0x08, 1) as u8, 0x80);

        d.latch_tc(0);
        d.port_write(0x09, 1, 0x03); // reset request ch3 without disturbing TC
        assert_eq!(d.port_read(0x08, 1) as u8, 0x01);
        assert_eq!(d.port_read(0x08, 1) as u8, 0);
    }

    #[test]
    fn master_clear_clears_target_controller_software_requests() {
        // Spec: Intel 8237A Master Clear clears the Request Register and status,
        // resets the byte pointer, and sets all four channel masks.
        let mut d = Dma8237::new();
        d.port_write(0x09, 1, 0x05); // master request ch1
        d.port_write(0xD2, 1, 0x06); // slave request ch2 (ISA ch6)
        d.port_write(0x0A, 1, 0x01); // unmask master ch1
        d.port_write(0xD4, 1, 0x02); // unmask slave-local ch2
        d.latch_tc(1);
        d.latch_tc(6);

        d.port_write(0x0D, 1, 0); // master controller Master Clear
        assert_eq!(d.master.request, 0);
        assert_eq!(d.master.mask, 0x0F);
        assert_eq!(d.port_read(0x08, 1) as u8, 0);
        assert_eq!(d.slave.request, 0x04);
        assert_eq!(d.port_read(0xD0, 1) as u8, 0x44);

        d.port_write(0xDA, 1, 0); // slave controller Master Clear
        assert_eq!(d.slave.request, 0);
        assert_eq!(d.slave.mask, 0x0F);
        assert_eq!(d.port_read(0xD0, 1) as u8, 0);
    }

    #[test]
    fn software_request_and_mask_unchanged_with_tc() {
        // Existing programming model must stay green beside TC latch/status.
        let mut d = Dma8237::new();
        d.port_write(0x09, 1, 0x04); // software request set ch0
        assert_eq!(d.master.request, 0x01);
        d.port_write(0x0A, 1, 0x00); // unmask ch0
        assert_eq!(d.master.mask & 0x01, 0);
        d.latch_tc(0);
        assert_eq!(d.port_read(0x08, 1) as u8, 0x11);
        assert_eq!(d.port_read(0x08, 1) as u8, 0x10);
        assert_eq!(d.master.request, 0x01);
        assert_eq!(d.master.mask & 0x01, 0);
    }

    #[test]
    fn mode_register_stored() {
        let mut d = Dma8237::new();
        d.port_write(0x0B, 1, 0x46); // ch2, single mode style pattern
        assert_eq!(d.master.channels[2].mode, 0x46);
    }

    #[test]
    fn cascade_mode_master_and_ch4_program_and_readback() {
        // Spec: Intel 8237A mode bits 7:6 = Cascade (`11`); OSDev ISA DMA — ISA
        // channel 4 is the AT cascade channel (slave controller channel 0).
        // Programming (mode set + struct readback) is accepted; no transfer.
        let mut d = Dma8237::new();

        // Master mode port 0x0B: Cascade | channel 0.
        d.port_write(0x0B, 1, 0xC0);
        assert_eq!(d.master.channels[0].mode, 0xC0);

        // Slave mode port 0xD6: Cascade | slave ch0 → ISA channel 4.
        d.port_write(0xD6, 1, 0xC0);
        assert_eq!(d.slave.channels[0].mode, 0xC0);

        // Page / addr / count for ISA ch4 remain programmable beside cascade mode.
        d.port_write(0xD8, 1, 0); // clear slave flip-flop
        d.port_write(0xC0, 1, 0x34); // ch4 addr low
        d.port_write(0xC0, 1, 0x12);
        d.port_write(0xD8, 1, 0);
        d.port_write(0xC2, 1, 0x01); // count low
        d.port_write(0xC2, 1, 0x00);
        d.port_write(DMA_PAGE_CH4, 1, 0xAB);
        assert_eq!(d.slave.channels[0].addr, 0x1234);
        assert_eq!(d.slave.channels[0].count, 0x0001);
        assert_eq!(d.page[4], 0xAB);
        assert_eq!(d.slave.channels[0].mode, 0xC0);
    }

    #[test]
    fn transfer_block_rejects_cascade_mode_and_isa_ch4() {
        // Spec: Intel 8237A Cascade is not a data-transfer mode; ISA ch4 is the
        // AT cascade reserved channel (OSDev ISA DMA).
        let mut d = Dma8237::new();
        let mut io = [0u8; 4];

        // Master cascade mode: stored, transfer rejected, state unchanged.
        d.port_write(0x0C, 1, 0);
        d.port_write(0x00, 1, 0x00);
        d.port_write(0x00, 1, 0x10);
        d.port_write(0x0C, 1, 0);
        d.port_write(0x01, 1, 0x00);
        d.port_write(0x01, 1, 0x00);
        d.port_write(DMA_PAGE_CH0, 1, 0x00);
        d.port_write(0x0B, 1, 0xC0); // Cascade | ch0
        d.port_write(0x0A, 1, 0x00); // unmask ch0
        assert_eq!(d.master.channels[0].mode, 0xC0);
        assert_eq!(
            d.transfer_block(0, &mut io, |_| 0, |_, _| {}),
            Err(DmaTransferError::UnsupportedMode)
        );
        assert_eq!(d.master.channels[0].count, 0);
        assert_eq!(d.master.status & 0x0F, 0);

        // ISA ch4: even Single+Write (data-looking mode) is not transferable.
        d.port_write(0xD8, 1, 0);
        d.port_write(0xC0, 1, 0x00);
        d.port_write(0xC0, 1, 0x00);
        d.port_write(0xD8, 1, 0);
        d.port_write(0xC2, 1, 0x00);
        d.port_write(0xC2, 1, 0x00);
        d.port_write(DMA_PAGE_CH4, 1, 0x00);
        d.port_write(0xD6, 1, 0x44); // Single | Inc | Write | slave ch0
        d.port_write(0xD4, 1, 0x00); // unmask slave ch0
        assert_eq!(d.slave.channels[0].mode, 0x44);
        assert_eq!(
            d.transfer_block(4, &mut io, |_| 0, |_, _| {}),
            Err(DmaTransferError::UnsupportedMode)
        );
        assert_eq!(d.slave.channels[0].count, 0);
        assert_eq!(d.slave.status & 0x0F, 0);

        // Cascade mode on ch4: programming readback + transfer reject.
        d.port_write(0xD6, 1, 0xC0);
        assert_eq!(d.slave.channels[0].mode, 0xC0);
        assert_eq!(
            d.transfer_block(4, &mut io, |_| 0, |_, _| {}),
            Err(DmaTransferError::UnsupportedMode)
        );
    }

    /// Program master ch2: page, addr, count=N−1, Single+Inc+Write (`0x46`), unmasked.
    fn program_ch2_write(d: &mut Dma8237, page: u8, addr: u16, count_minus_one: u16) {
        d.port_write(0x0C, 1, 0); // clear flip-flop
        d.port_write(0x04, 1, (addr & 0xFF) as u32); // ch2 addr low
        d.port_write(0x04, 1, (addr >> 8) as u32);
        d.port_write(0x0C, 1, 0);
        d.port_write(0x05, 1, (count_minus_one & 0xFF) as u32);
        d.port_write(0x05, 1, (count_minus_one >> 8) as u32);
        d.port_write(DMA_PAGE_CH2, 1, u32::from(page));
        d.port_write(0x0B, 1, 0x46); // Single | Inc | Write | ch2
        d.port_write(0x0A, 1, 0x02); // unmask ch2
    }

    #[test]
    fn transfer_block_write_moves_io_to_memory_and_latches_tc() {
        // Spec: Intel 8237A — count register holds N−1; Write = device→memory;
        // Single mode + address increment; TC at end; current addr/count update.
        // Phys = (page << 16) | addr (OSDev ISA DMA, 8-bit ch0–3).
        let mut d = Dma8237::new();
        program_ch2_write(&mut d, 0x01, 0x1000, 3); // 4 bytes
        let mem = RefCell::new(vec![0u8; 0x2_0000]);
        let mut io = [0xAAu8, 0xBB, 0xCC, 0xDD];
        let n = d
            .transfer_block(
                2,
                &mut io,
                |phys| mem.borrow()[phys as usize],
                |phys, b| mem.borrow_mut()[phys as usize] = b,
            )
            .expect("write transfer");
        assert_eq!(n, 4);
        assert_eq!(&mem.borrow()[0x1_1000..0x1_1004], &[0xAA, 0xBB, 0xCC, 0xDD]);
        assert_eq!(d.master.channels[2].addr, 0x1004);
        assert_eq!(d.master.channels[2].count, 0xFFFF);
        assert_eq!(d.port_read(0x08, 1) as u8 & 0x0F, 0x04); // TC ch2
        assert_eq!(d.port_read(0x08, 1) as u8 & 0x0F, 0); // clear-on-read
    }

    #[test]
    fn transfer_block_read_moves_memory_to_io() {
        // Spec: Intel 8237A mode bits 3:2 = Read (10) — memory→device.
        let mut d = Dma8237::new();
        d.port_write(0x0C, 1, 0);
        d.port_write(0x04, 1, 0x00);
        d.port_write(0x04, 1, 0x20); // addr 0x2000
        d.port_write(0x0C, 1, 0);
        d.port_write(0x05, 1, 0x01); // count 1 → 2 bytes
        d.port_write(0x05, 1, 0x00);
        d.port_write(DMA_PAGE_CH2, 1, 0x00);
        d.port_write(0x0B, 1, 0x4A); // Single | Inc | Read | ch2
        d.port_write(0x0A, 1, 0x02);
        let mem = RefCell::new(vec![0u8; 0x3000]);
        mem.borrow_mut()[0x2000] = 0x11;
        mem.borrow_mut()[0x2001] = 0x22;
        let mut io = [0u8; 2];
        let n = d
            .transfer_block(
                2,
                &mut io,
                |phys| mem.borrow()[phys as usize],
                |phys, b| mem.borrow_mut()[phys as usize] = b,
            )
            .expect("read transfer");
        assert_eq!(n, 2);
        assert_eq!(io, [0x11, 0x22]);
        assert_eq!(d.master.channels[2].addr, 0x2002);
        assert_eq!(d.master.channels[2].count, 0xFFFF);
        assert_eq!(d.master.status & 0x0F, 0x04);
    }

    #[test]
    fn transfer_block_rejects_unsupported_mode_and_masked() {
        let mut d = Dma8237::new();
        program_ch2_write(&mut d, 0, 0x100, 0);
        // Cascade mode (bits 7:6 = 11) — not a data-transfer mode for the helper.
        // Spec: Intel 8237A mode register — Cascade selects a cascaded controller.
        d.port_write(0x0B, 1, 0xC6);
        let mut mem = [0u8; 16];
        let mut io = [0x55u8];
        assert_eq!(
            d.transfer_block(2, &mut io, |_| 0, |_, _| {},),
            Err(DmaTransferError::UnsupportedMode)
        );
        assert_eq!(d.master.channels[2].count, 0); // unchanged
        assert_eq!(d.master.status & 0x0F, 0);

        // Restore Single+Write, then mask.
        d.port_write(0x0B, 1, 0x46);
        d.port_write(0x0A, 1, 0x06); // mask ch2
        assert_eq!(
            d.transfer_block(2, &mut io, |_| 0, |i, b| mem[i as usize] = b,),
            Err(DmaTransferError::Masked)
        );
    }

    /// Program master channel: page/addr/count + mode + unmask.
    fn program_master_ch_write(
        d: &mut Dma8237,
        isa_ch: usize,
        page: u8,
        addr: u16,
        count_minus_one: u16,
        mode: u8,
    ) {
        assert!(isa_ch <= 3);
        let addr_port = (isa_ch * 2) as u16;
        let count_port = addr_port + 1;
        let page_port = match isa_ch {
            0 => DMA_PAGE_CH0,
            1 => DMA_PAGE_CH1,
            2 => DMA_PAGE_CH2,
            3 => DMA_PAGE_CH3,
            _ => unreachable!(),
        };
        d.port_write(0x0C, 1, 0);
        d.port_write(addr_port, 1, (addr & 0xFF) as u32);
        d.port_write(addr_port, 1, (addr >> 8) as u32);
        d.port_write(0x0C, 1, 0);
        d.port_write(count_port, 1, (count_minus_one & 0xFF) as u32);
        d.port_write(count_port, 1, (count_minus_one >> 8) as u32);
        d.port_write(page_port, 1, u32::from(page));
        d.port_write(0x0B, 1, u32::from(mode));
        d.port_write(0x0A, 1, isa_ch as u32); // unmask
    }

    #[test]
    fn transfer_block_demand_write_ch0_to_ch3_completes_count_tc_and_masks() {
        // Spec: Intel 8237A mode bits 7:6 = Demand (`00`). Software helper
        // completes the programmed count (same TC / post-TC Current / auto-mask
        // as Single when Autoinitialize is clear). DREQ hold/release not modeled.
        for isa_ch in 0usize..=3 {
            let mut d = Dma8237::new();
            // Demand | Inc | Write | chN — bits 7:6=00, 3:2=01, 1:0=ch
            let mode = 0x04 | (isa_ch as u8);
            program_master_ch_write(&mut d, isa_ch, 0x01, 0x1000, 3, mode); // 4 bytes
            let mem = RefCell::new(vec![0u8; 0x2_0000]);
            let mut io = [0xAAu8, 0xBB, 0xCC, 0xDD];
            let n = d
                .transfer_block(
                    isa_ch,
                    &mut io,
                    |phys| mem.borrow()[phys as usize],
                    |phys, b| mem.borrow_mut()[phys as usize] = b,
                )
                .unwrap_or_else(|e| panic!("demand write ch{isa_ch}: {e:?}"));
            assert_eq!(n, 4);
            assert_eq!(&mem.borrow()[0x1_1000..0x1_1004], &[0xAA, 0xBB, 0xCC, 0xDD]);
            assert_eq!(d.master.channels[isa_ch].addr, 0x1004);
            assert_eq!(d.master.channels[isa_ch].count, 0xFFFF);
            assert_eq!(d.master.status & (1 << isa_ch), 1 << isa_ch);
            assert_eq!(d.master.mask & (1 << isa_ch), 1 << isa_ch);
        }
    }

    #[test]
    fn transfer_block_block_write_ch0_to_ch3_completes_count_tc_and_masks() {
        // Spec: Intel 8237A mode bits 7:6 = Block (`10`). Helper finishes all
        // count+1 units in one call; TC + auto-mask match Single (no Autoinit).
        for isa_ch in 0usize..=3 {
            let mut d = Dma8237::new();
            // Block | Inc | Write | chN — bits 7:6=10, 3:2=01, 1:0=ch
            let mode = 0x84 | (isa_ch as u8);
            program_master_ch_write(&mut d, isa_ch, 0x02, 0x2000, 1, mode); // 2 bytes
            let mem = RefCell::new(vec![0u8; 0x4_0000]);
            let mut io = [0x11u8, 0x22];
            let n = d
                .transfer_block(
                    isa_ch,
                    &mut io,
                    |phys| mem.borrow()[phys as usize],
                    |phys, b| mem.borrow_mut()[phys as usize] = b,
                )
                .unwrap_or_else(|e| panic!("block write ch{isa_ch}: {e:?}"));
            assert_eq!(n, 2);
            assert_eq!(&mem.borrow()[0x2_2000..0x2_2002], &[0x11, 0x22]);
            assert_eq!(d.master.channels[isa_ch].addr, 0x2002);
            assert_eq!(d.master.channels[isa_ch].count, 0xFFFF);
            assert_eq!(d.master.status & (1 << isa_ch), 1 << isa_ch);
            assert_eq!(d.master.mask & (1 << isa_ch), 1 << isa_ch);
        }
    }

    #[test]
    fn transfer_block_demand_autoinit_reloads_base_without_mask() {
        // Spec: Intel 8237A — Autoinitialize (mode bit 4) + Demand: after TC,
        // Current reloads from Base; channel stays unmasked (same as Single).
        let mut d = Dma8237::new();
        program_master_ch_write(&mut d, 2, 0x01, 0x1000, 1, 0x16); // Demand|Auto|Inc|Write|ch2
        let mem = RefCell::new(vec![0u8; 0x2_0000]);
        let mut io = [0xAAu8, 0xBB];
        d.transfer_block(
            2,
            &mut io,
            |phys| mem.borrow()[phys as usize],
            |phys, b| mem.borrow_mut()[phys as usize] = b,
        )
        .expect("demand autoinit write");
        assert_eq!(d.master.channels[2].addr, 0x1000);
        assert_eq!(d.master.channels[2].count, 1);
        assert_eq!(d.master.mask & 0x04, 0);
        assert_eq!(d.master.status & 0x04, 0x04);

        let mut io2 = [0xCCu8, 0xDD];
        d.transfer_block(
            2,
            &mut io2,
            |phys| mem.borrow()[phys as usize],
            |phys, b| mem.borrow_mut()[phys as usize] = b,
        )
        .expect("second demand autoinit write");
        assert_eq!(&mem.borrow()[0x1_1000..0x1_1002], &[0xCC, 0xDD]);
        assert_eq!(d.master.channels[2].addr, 0x1000);
        assert_eq!(d.master.mask & 0x04, 0);
    }

    #[test]
    fn transfer_block_block_autoinit_reloads_base_without_mask() {
        // Spec: Intel 8237A — Autoinitialize + Block: TC reloads Base; no auto-mask.
        let mut d = Dma8237::new();
        program_master_ch_write(&mut d, 1, 0x00, 0x0100, 0, 0x95); // Block|Auto|Inc|Write|ch1
        let mem = RefCell::new(vec![0u8; 0x1000]);
        let mut io = [0xEE];
        d.transfer_block(
            1,
            &mut io,
            |phys| mem.borrow()[phys as usize],
            |phys, b| mem.borrow_mut()[phys as usize] = b,
        )
        .expect("block autoinit write");
        assert_eq!(mem.borrow()[0x0100], 0xEE);
        assert_eq!(d.master.channels[1].addr, 0x0100);
        assert_eq!(d.master.channels[1].count, 0);
        assert_eq!(d.master.mask & 0x02, 0);
        assert_eq!(d.master.status & 0x02, 0x02);
    }

    #[test]
    fn transfer_block_block_read_moves_memory_to_io() {
        // Spec: Intel 8237A Block + Read (bits 3:2 = 10) — memory→device.
        let mut d = Dma8237::new();
        program_master_ch_write(&mut d, 2, 0x00, 0x2000, 1, 0x8A); // Block|Inc|Read|ch2
        let mem = RefCell::new(vec![0u8; 0x3000]);
        mem.borrow_mut()[0x2000] = 0x33;
        mem.borrow_mut()[0x2001] = 0x44;
        let mut io = [0u8; 2];
        let n = d
            .transfer_block(
                2,
                &mut io,
                |phys| mem.borrow()[phys as usize],
                |phys, b| mem.borrow_mut()[phys as usize] = b,
            )
            .expect("block read");
        assert_eq!(n, 2);
        assert_eq!(io, [0x33, 0x44]);
        assert_eq!(d.master.channels[2].addr, 0x2002);
        assert_eq!(d.master.channels[2].count, 0xFFFF);
        assert_eq!(d.master.mask & 0x04, 0x04);
    }

    #[test]
    fn transfer_block_ch5_demand_and_block_word_write_parity() {
        // Spec: Intel 8237A Demand/Block on 16-bit slave ch1 (ISA ch5): same
        // word length / phys / TC / auto-mask as Single word helper.
        for (mode, label) in [(0x05u8, "demand"), (0x85u8, "block")] {
            let mut d = Dma8237::new();
            d.port_write(0xD8, 1, 0);
            d.port_write(0xC4, 1, 0x00);
            d.port_write(0xC4, 1, 0x10); // word addr 0x1000
            d.port_write(0xD8, 1, 0);
            d.port_write(0xC6, 1, 0x01); // 2 words
            d.port_write(0xC6, 1, 0x00);
            d.port_write(DMA_PAGE_CH5, 1, 0x01);
            d.port_write(0xD6, 1, u32::from(mode)); // Demand|Block | Inc | Write | ch1
            d.port_write(0xD4, 1, 0x01);
            let mem = RefCell::new(vec![0u8; 0x2_0000]);
            let mut io = [0xAAu8, 0xBB, 0xCC, 0xDD];
            let n = d
                .transfer_block(
                    5,
                    &mut io,
                    |phys| mem.borrow()[phys as usize],
                    |phys, b| mem.borrow_mut()[phys as usize] = b,
                )
                .unwrap_or_else(|e| panic!("ch5 {label} word write: {e:?}"));
            assert_eq!(n, 4, "{label}");
            assert_eq!(
                &mem.borrow()[0x1_2000..0x1_2004],
                &[0xAA, 0xBB, 0xCC, 0xDD],
                "{label}"
            );
            assert_eq!(d.slave.channels[1].addr, 0x1002, "{label}");
            assert_eq!(d.slave.channels[1].count, 0xFFFF, "{label}");
            assert_eq!(d.slave.status & 0x02, 0x02, "{label}");
            assert_eq!(d.slave.mask & 0x02, 0x02, "{label}");
        }
    }

    #[test]
    fn transfer_block_addr_wraps_within_64k_page() {
        // Spec: ISA DMA — 16-bit current address wraps; page does not auto-bump.
        let mut d = Dma8237::new();
        program_ch2_write(&mut d, 0x02, 0xFFFE, 1); // 2 bytes at end of page
        let mem = RefCell::new(vec![0u8; 0x4_0000]);
        let mut io = [0xEEu8, 0xFF];
        d.transfer_block(
            2,
            &mut io,
            |phys| mem.borrow()[phys as usize],
            |phys, b| mem.borrow_mut()[phys as usize] = b,
        )
        .unwrap();
        assert_eq!(mem.borrow()[0x2_FFFE], 0xEE);
        assert_eq!(mem.borrow()[0x2_FFFF], 0xFF);
        assert_eq!(d.master.channels[2].addr, 0x0000); // wrapped
        assert_eq!(d.page[2], 0x02); // page unchanged
    }

    #[test]
    fn transfer_block_rejects_bad_channel_and_short_buffer() {
        let mut d = Dma8237::new();
        let mut io = [0u8; 4];
        assert_eq!(
            d.transfer_block(8, &mut io, |_| 0, |_, _| {}),
            Err(DmaTransferError::BadChannel)
        );
        program_ch2_write(&mut d, 0, 0, 3); // wants 4 bytes
        let mut short = [0u8; 2];
        assert_eq!(
            d.transfer_block(2, &mut short, |_| 0, |_, _| {}),
            Err(DmaTransferError::BufferTooShort)
        );
    }

    /// Program slave ISA ch5 (controller ch1): page, word addr, count=N−1 words,
    /// Single+Inc+Write (`0x45`), unmasked.
    ///
    /// Spec: OSDev ISA DMA — 16-bit channels 4–7; slave even ports; page `0x8B`.
    fn program_ch5_write(d: &mut Dma8237, page: u8, word_addr: u16, count_minus_one: u16) {
        d.port_write(0xD8, 1, 0); // clear slave flip-flop
        d.port_write(0xC4, 1, (word_addr & 0xFF) as u32); // ch5 addr low
        d.port_write(0xC4, 1, (word_addr >> 8) as u32);
        d.port_write(0xD8, 1, 0);
        d.port_write(0xC6, 1, (count_minus_one & 0xFF) as u32);
        d.port_write(0xC6, 1, (count_minus_one >> 8) as u32);
        d.port_write(DMA_PAGE_CH5, 1, u32::from(page));
        d.port_write(0xD6, 1, 0x45); // Single | Inc | Write | slave ch1
        d.port_write(0xD4, 1, 0x01); // unmask slave ch1 (ISA ch5)
    }

    /// Program slave ISA ch5: Single+Inc+AutoInit+Write (`0x55`).
    fn program_ch5_autoinit_write(d: &mut Dma8237, page: u8, word_addr: u16, count_minus_one: u16) {
        d.port_write(0xD8, 1, 0);
        d.port_write(0xC4, 1, (word_addr & 0xFF) as u32);
        d.port_write(0xC4, 1, (word_addr >> 8) as u32);
        d.port_write(0xD8, 1, 0);
        d.port_write(0xC6, 1, (count_minus_one & 0xFF) as u32);
        d.port_write(0xC6, 1, (count_minus_one >> 8) as u32);
        d.port_write(DMA_PAGE_CH5, 1, u32::from(page));
        d.port_write(0xD6, 1, 0x55); // Single | AutoInit | Inc | Write | ch1
        d.port_write(0xD4, 1, 0x01);
    }

    /// Program slave ISA ch5: Single+Inc+Verify (`0x41`).
    fn program_ch5_verify(d: &mut Dma8237, page: u8, word_addr: u16, count_minus_one: u16) {
        d.port_write(0xD8, 1, 0);
        d.port_write(0xC4, 1, (word_addr & 0xFF) as u32);
        d.port_write(0xC4, 1, (word_addr >> 8) as u32);
        d.port_write(0xD8, 1, 0);
        d.port_write(0xC6, 1, (count_minus_one & 0xFF) as u32);
        d.port_write(0xC6, 1, (count_minus_one >> 8) as u32);
        d.port_write(DMA_PAGE_CH5, 1, u32::from(page));
        d.port_write(0xD6, 1, 0x41); // Single | Inc | Verify | slave ch1
        d.port_write(0xD4, 1, 0x01);
    }

    #[test]
    fn transfer_block_ch5_write_moves_words_to_memory_and_latches_tc() {
        // Spec: Intel 8237A + OSDev ISA DMA — channels 4–7 are 16-bit: count is
        // words−1; Current Address is a word address; phys =
        // `(page << 16) | (addr << 1)` (A0 forced 0). Write = device→memory;
        // each count step moves two bytes (LE). TC + auto-mask without Autoinit.
        let mut d = Dma8237::new();
        program_ch5_write(&mut d, 0x01, 0x1000, 1); // 2 words → 4 bytes at phys 0x1_2000
        let mem = RefCell::new(vec![0u8; 0x2_0000]);
        let mut io = [0xAAu8, 0xBB, 0xCC, 0xDD];
        let n = d
            .transfer_block(
                5,
                &mut io,
                |phys| mem.borrow()[phys as usize],
                |phys, b| mem.borrow_mut()[phys as usize] = b,
            )
            .expect("ch5 word write");
        assert_eq!(n, 4);
        assert_eq!(&mem.borrow()[0x1_2000..0x1_2004], &[0xAA, 0xBB, 0xCC, 0xDD]);
        assert_eq!(d.slave.channels[1].addr, 0x1002); // +2 words
        assert_eq!(d.slave.channels[1].count, 0xFFFF);
        assert_eq!(d.port_read(0xD0, 1) as u8 & 0x0F, 0x02); // TC slave ch1
        assert_eq!(d.slave.mask & 0x02, 0x02); // auto-masked
    }

    #[test]
    fn transfer_block_ch5_read_moves_memory_words_to_io() {
        // Spec: Intel 8237A mode bits 3:2 = Read — memory→device; 16-bit ch5.
        let mut d = Dma8237::new();
        d.port_write(0xD8, 1, 0);
        d.port_write(0xC4, 1, 0x00);
        d.port_write(0xC4, 1, 0x10); // word addr 0x1000 → phys 0x2000
        d.port_write(0xD8, 1, 0);
        d.port_write(0xC6, 1, 0x00); // 1 word
        d.port_write(0xC6, 1, 0x00);
        d.port_write(DMA_PAGE_CH5, 1, 0x00);
        d.port_write(0xD6, 1, 0x49); // Single | Inc | Read | slave ch1
        d.port_write(0xD4, 1, 0x01);
        let mem = RefCell::new(vec![0u8; 0x3000]);
        mem.borrow_mut()[0x2000] = 0x11;
        mem.borrow_mut()[0x2001] = 0x22;
        let mut io = [0u8; 2];
        let n = d
            .transfer_block(
                5,
                &mut io,
                |phys| mem.borrow()[phys as usize],
                |phys, b| mem.borrow_mut()[phys as usize] = b,
            )
            .expect("ch5 word read");
        assert_eq!(n, 2);
        assert_eq!(io, [0x11, 0x22]);
        assert_eq!(d.slave.channels[1].addr, 0x1001);
        assert_eq!(d.slave.channels[1].count, 0xFFFF);
        assert_eq!(d.slave.status & 0x0F, 0x02);
        assert_eq!(d.slave.mask & 0x02, 0x02);
    }

    #[test]
    fn transfer_block_ch5_multi_word_write() {
        // Parity with 8-bit multi-byte: count=3 → 4 words / 8 bytes.
        let mut d = Dma8237::new();
        program_ch5_write(&mut d, 0x02, 0x0100, 3);
        let mem = RefCell::new(vec![0u8; 0x4_0000]);
        let mut io = [0x10u8, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80];
        let n = d
            .transfer_block(
                5,
                &mut io,
                |phys| mem.borrow()[phys as usize],
                |phys, b| mem.borrow_mut()[phys as usize] = b,
            )
            .expect("ch5 multi-word write");
        assert_eq!(n, 8);
        assert_eq!(
            &mem.borrow()[0x2_0200..0x2_0208],
            &[0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80]
        );
        assert_eq!(d.slave.channels[1].addr, 0x0104);
    }

    #[test]
    fn transfer_block_ch5_verify_advances_without_memory_rw() {
        // Spec: Intel 8237A Verify on 16-bit channel — word addr/count + TC/mask;
        // no mem_read/mem_write. Buffer length still checked in bytes (2×words).
        let mut d = Dma8237::new();
        program_ch5_verify(&mut d, 0x01, 0x1000, 1); // 2 words
        let mem = RefCell::new(vec![0x5Au8; 0x2_0000]);
        let before = mem.borrow().clone();
        let mut io = [0xAAu8, 0xBB, 0xCC, 0xDD];
        let n = d
            .transfer_block(
                5,
                &mut io,
                |_| panic!("Verify must not call mem_read"),
                |_, _| panic!("Verify must not call mem_write"),
            )
            .expect("ch5 verify");
        assert_eq!(n, 4);
        assert_eq!(io, [0xAA, 0xBB, 0xCC, 0xDD]);
        assert_eq!(*mem.borrow(), before);
        assert_eq!(d.slave.channels[1].addr, 0x1002);
        assert_eq!(d.slave.channels[1].count, 0xFFFF);
        assert_eq!(d.slave.status & 0x0F, 0x02);
        assert_eq!(d.slave.mask & 0x02, 0x02);
        program_ch5_verify(&mut d, 0x01, 0x2000, 1);
        let mut short = [0u8; 2]; // needs 4 bytes
        assert_eq!(
            d.transfer_block(5, &mut short, |_| 0, |_, _| {}),
            Err(DmaTransferError::BufferTooShort)
        );
    }

    #[test]
    fn transfer_block_ch5_autoinit_write_reloads_base_and_second_transfer() {
        // Spec: Intel 8237A Autoinitialize on slave word channel — reload Base;
        // channel stays unmasked for a second helper call.
        let mut d = Dma8237::new();
        program_ch5_autoinit_write(&mut d, 0x01, 0x1000, 1); // 2 words
        assert_eq!(d.slave.channels[1].base_addr, 0x1000);
        assert_eq!(d.slave.channels[1].base_count, 1);
        let mem = RefCell::new(vec![0u8; 0x2_0000]);
        let mut io = [0xAAu8, 0xBB, 0xCC, 0xDD];
        d.transfer_block(
            5,
            &mut io,
            |phys| mem.borrow()[phys as usize],
            |phys, b| mem.borrow_mut()[phys as usize] = b,
        )
        .expect("ch5 autoinit write");
        assert_eq!(&mem.borrow()[0x1_2000..0x1_2004], &[0xAA, 0xBB, 0xCC, 0xDD]);
        assert_eq!(d.slave.channels[1].addr, 0x1000);
        assert_eq!(d.slave.channels[1].count, 1);
        assert_eq!(d.slave.mask & 0x02, 0);
        assert_eq!(d.slave.status & 0x0F, 0x02);

        let mut io2 = [0x11u8, 0x22, 0x33, 0x44];
        d.transfer_block(
            5,
            &mut io2,
            |phys| mem.borrow()[phys as usize],
            |phys, b| mem.borrow_mut()[phys as usize] = b,
        )
        .expect("second ch5 autoinit write");
        assert_eq!(&mem.borrow()[0x1_2000..0x1_2004], &[0x11, 0x22, 0x33, 0x44]);
        assert_eq!(d.slave.channels[1].addr, 0x1000);
        assert_eq!(d.slave.channels[1].count, 1);
        assert_eq!(d.slave.mask & 0x02, 0);
    }

    #[test]
    fn transfer_block_ch5_word_addr_wraps_within_page() {
        // Spec: ISA DMA 16-bit — word Current Address wraps as u16; page unchanged.
        // phys = (page<<16)|(addr<<1): word 0xFFFE → 0x1FFFC ORed into the page
        // base (A16 from addr bit15), so page=0 → phys 0x1FFFC / 0x1FFFE.
        let mut d = Dma8237::new();
        program_ch5_write(&mut d, 0x00, 0xFFFE, 1); // 2 words at end of word space
        let mem = RefCell::new(vec![0u8; 0x2_0000]);
        let mut io = [0xEEu8, 0xFF, 0x11, 0x22];
        d.transfer_block(
            5,
            &mut io,
            |phys| mem.borrow()[phys as usize],
            |phys, b| mem.borrow_mut()[phys as usize] = b,
        )
        .unwrap();
        assert_eq!(mem.borrow()[0x1_FFFC], 0xEE);
        assert_eq!(mem.borrow()[0x1_FFFD], 0xFF);
        assert_eq!(mem.borrow()[0x1_FFFE], 0x11);
        assert_eq!(mem.borrow()[0x1_FFFF], 0x22);
        assert_eq!(d.slave.channels[1].addr, 0x0000); // wrapped
        assert_eq!(d.page[5], 0x00); // page register not auto-bumped
    }

    #[test]
    fn transfer_block_ch5_without_autoinit_auto_masks_after_tc() {
        // Spec: Intel 8237A — non-Autoinit TC hardware-masks the slave channel.
        let mut d = Dma8237::new();
        program_ch5_write(&mut d, 0x01, 0x1000, 0); // 1 word
        assert_eq!(d.slave.mask & 0x02, 0);
        let mem = RefCell::new(vec![0u8; 0x2_0000]);
        let mut io = [0xAAu8, 0xBB];
        d.transfer_block(
            5,
            &mut io,
            |phys| mem.borrow()[phys as usize],
            |phys, b| mem.borrow_mut()[phys as usize] = b,
        )
        .expect("ch5 non-autoinit");
        assert_eq!(d.slave.mask & 0x02, 0x02);
        let mut io2 = [0x11u8, 0x22];
        assert_eq!(
            d.transfer_block(
                5,
                &mut io2,
                |phys| mem.borrow()[phys as usize],
                |phys, b| mem.borrow_mut()[phys as usize] = b,
            ),
            Err(DmaTransferError::Masked)
        );
    }

    #[test]
    fn transfer_block_ch5_rejects_unsupported_mode_and_masked() {
        let mut d = Dma8237::new();
        program_ch5_write(&mut d, 0, 0x100, 0);
        // Spec: Intel 8237A — Cascade (bits 7:6 = 11) is not a data-transfer mode.
        d.port_write(0xD6, 1, 0xC5); // Cascade | Inc | Write | slave ch1
        let mut io = [0x55u8, 0x66];
        assert_eq!(
            d.transfer_block(5, &mut io, |_| 0, |_, _| {}),
            Err(DmaTransferError::UnsupportedMode)
        );
        d.port_write(0xD6, 1, 0x45);
        d.port_write(0xD4, 1, 0x05); // mask slave ch1
        assert_eq!(
            d.transfer_block(5, &mut io, |_| 0, |_, _| {}),
            Err(DmaTransferError::Masked)
        );
    }

    /// Program master ch2: Single+Inc+AutoInit+Write (`0x56`).
    fn program_ch2_autoinit_write(d: &mut Dma8237, page: u8, addr: u16, count_minus_one: u16) {
        d.port_write(0x0C, 1, 0);
        d.port_write(0x04, 1, (addr & 0xFF) as u32);
        d.port_write(0x04, 1, (addr >> 8) as u32);
        d.port_write(0x0C, 1, 0);
        d.port_write(0x05, 1, (count_minus_one & 0xFF) as u32);
        d.port_write(0x05, 1, (count_minus_one >> 8) as u32);
        d.port_write(DMA_PAGE_CH2, 1, u32::from(page));
        d.port_write(0x0B, 1, 0x56); // Single | AutoInit | Inc | Write | ch2
        d.port_write(0x0A, 1, 0x02); // unmask ch2
    }

    #[test]
    fn transfer_block_autoinit_write_reloads_base_and_second_transfer() {
        // Spec: Intel 8237A — mode bit 4 Autoinitialize: at TC, Current Address and
        // Current Word Count are restored from Base Address / Base Word Count; the
        // channel remains ready (mask unchanged) for another transfer. Base is
        // loaded when Current is programmed. TC status bit still latches.
        let mut d = Dma8237::new();
        program_ch2_autoinit_write(&mut d, 0x01, 0x1000, 3); // 4 bytes
        assert_eq!(d.master.channels[2].base_addr, 0x1000);
        assert_eq!(d.master.channels[2].base_count, 3);
        let mem = RefCell::new(vec![0u8; 0x2_0000]);
        let mut io = [0xAAu8, 0xBB, 0xCC, 0xDD];
        let n = d
            .transfer_block(
                2,
                &mut io,
                |phys| mem.borrow()[phys as usize],
                |phys, b| mem.borrow_mut()[phys as usize] = b,
            )
            .expect("autoinit write");
        assert_eq!(n, 4);
        assert_eq!(&mem.borrow()[0x1_1000..0x1_1004], &[0xAA, 0xBB, 0xCC, 0xDD]);
        // Reloaded from base — not left at post-TC current (0x1004 / 0xFFFF).
        assert_eq!(d.master.channels[2].addr, 0x1000);
        assert_eq!(d.master.channels[2].count, 3);
        assert_eq!(d.master.channels[2].base_addr, 0x1000);
        assert_eq!(d.master.channels[2].base_count, 3);
        assert_eq!(d.master.mask & 0x04, 0); // still unmasked / ready
        assert_eq!(d.master.status & 0x0F, 0x04); // TC latched

        // Second transfer without reprogramming uses reloaded current.
        let mut io2 = [0x11u8, 0x22, 0x33, 0x44];
        d.transfer_block(
            2,
            &mut io2,
            |phys| mem.borrow()[phys as usize],
            |phys, b| mem.borrow_mut()[phys as usize] = b,
        )
        .expect("second autoinit write");
        assert_eq!(&mem.borrow()[0x1_1000..0x1_1004], &[0x11, 0x22, 0x33, 0x44]);
        assert_eq!(d.master.channels[2].addr, 0x1000);
        assert_eq!(d.master.channels[2].count, 3);
    }

    #[test]
    fn transfer_block_autoinit_read_reloads_base() {
        // Spec: Intel 8237A — Autoinitialize + Read (bits 3:2 = 10).
        let mut d = Dma8237::new();
        d.port_write(0x0C, 1, 0);
        d.port_write(0x04, 1, 0x00);
        d.port_write(0x04, 1, 0x20); // addr 0x2000
        d.port_write(0x0C, 1, 0);
        d.port_write(0x05, 1, 0x01); // count 1 → 2 bytes
        d.port_write(0x05, 1, 0x00);
        d.port_write(DMA_PAGE_CH2, 1, 0x00);
        d.port_write(0x0B, 1, 0x5A); // Single | AutoInit | Inc | Read | ch2
        d.port_write(0x0A, 1, 0x02);
        let mem = RefCell::new(vec![0u8; 0x3000]);
        mem.borrow_mut()[0x2000] = 0x11;
        mem.borrow_mut()[0x2001] = 0x22;
        let mut io = [0u8; 2];
        d.transfer_block(
            2,
            &mut io,
            |phys| mem.borrow()[phys as usize],
            |phys, b| mem.borrow_mut()[phys as usize] = b,
        )
        .expect("autoinit read");
        assert_eq!(io, [0x11, 0x22]);
        assert_eq!(d.master.channels[2].addr, 0x2000);
        assert_eq!(d.master.channels[2].count, 1);
        assert_eq!(d.master.channels[2].base_addr, 0x2000);
        assert_eq!(d.master.channels[2].base_count, 1);
        assert_eq!(d.master.status & 0x0F, 0x04);
        assert_eq!(d.master.mask & 0x04, 0);
    }

    #[test]
    fn transfer_block_without_autoinit_leaves_post_tc_current() {
        // Contrast: Autoinitialize disabled — current ends at final addr / 0xFFFF.
        let mut d = Dma8237::new();
        program_ch2_write(&mut d, 0x01, 0x1000, 1); // 2 bytes, mode 0x46
        let mem = RefCell::new(vec![0u8; 0x2_0000]);
        let mut io = [0xAAu8, 0xBB];
        d.transfer_block(
            2,
            &mut io,
            |phys| mem.borrow()[phys as usize],
            |phys, b| mem.borrow_mut()[phys as usize] = b,
        )
        .unwrap();
        assert_eq!(d.master.channels[2].addr, 0x1002);
        assert_eq!(d.master.channels[2].count, 0xFFFF);
        assert_eq!(d.master.channels[2].base_addr, 0x1000);
        assert_eq!(d.master.channels[2].base_count, 1);
    }

    #[test]
    fn transfer_block_without_autoinit_auto_masks_after_tc() {
        // Spec: Intel 8237A — when Autoinitialize is not selected, the channel is
        // hardware-masked after terminal count. A subsequent transfer_block must
        // return Masked until software clears the mask bit. latch_tc alone does
        // not change mask (see software_request_and_mask_unchanged_with_tc).
        let mut d = Dma8237::new();
        program_ch2_write(&mut d, 0x01, 0x1000, 1); // 2 bytes, mode 0x46 (no AutoInit)
        assert_eq!(d.master.mask & 0x04, 0);
        let mem = RefCell::new(vec![0u8; 0x2_0000]);
        let mut io = [0xAAu8, 0xBB];
        d.transfer_block(
            2,
            &mut io,
            |phys| mem.borrow()[phys as usize],
            |phys, b| mem.borrow_mut()[phys as usize] = b,
        )
        .expect("first non-autoinit write");
        assert_eq!(d.master.status & 0x0F, 0x04); // TC latched
        assert_eq!(d.master.mask & 0x04, 0x04); // ch2 auto-masked

        let mut io2 = [0x11u8, 0x22];
        let err = d
            .transfer_block(
                2,
                &mut io2,
                |phys| mem.borrow()[phys as usize],
                |phys, b| mem.borrow_mut()[phys as usize] = b,
            )
            .expect_err("masked after non-autoinit TC");
        assert_eq!(err, DmaTransferError::Masked);

        // Software unmask allows another programmed transfer.
        d.port_write(0x0A, 1, 0x02); // clear mask ch2
        d.port_write(0x0C, 1, 0);
        d.port_write(0x04, 1, 0x00);
        d.port_write(0x04, 1, 0x20); // addr 0x2000
        d.port_write(0x0C, 1, 0);
        d.port_write(0x05, 1, 0x01);
        d.port_write(0x05, 1, 0x00);
        d.port_write(0x0B, 1, 0x46);
        let mut io3 = [0xCCu8, 0xDD];
        d.transfer_block(
            2,
            &mut io3,
            |phys| mem.borrow()[phys as usize],
            |phys, b| mem.borrow_mut()[phys as usize] = b,
        )
        .expect("after software unmask");
        assert_eq!(&mem.borrow()[0x1_2000..0x1_2002], &[0xCC, 0xDD]);
        assert_eq!(d.master.mask & 0x04, 0x04); // auto-masked again
    }

    #[test]
    fn transfer_block_autoinit_does_not_auto_mask_after_tc() {
        // Spec: Intel 8237A — Autoinitialize selected: channel is not masked at TC
        // and stays ready for another transfer without a mask-register write.
        let mut d = Dma8237::new();
        program_ch2_autoinit_write(&mut d, 0x01, 0x1000, 1); // 2 bytes, mode 0x56
        let mem = RefCell::new(vec![0u8; 0x2_0000]);
        let mut io = [0xAAu8, 0xBB];
        d.transfer_block(
            2,
            &mut io,
            |phys| mem.borrow()[phys as usize],
            |phys, b| mem.borrow_mut()[phys as usize] = b,
        )
        .expect("autoinit write");
        assert_eq!(d.master.status & 0x0F, 0x04);
        assert_eq!(d.master.mask & 0x04, 0); // not auto-masked
        let mut io2 = [0x11u8, 0x22];
        d.transfer_block(
            2,
            &mut io2,
            |phys| mem.borrow()[phys as usize],
            |phys, b| mem.borrow_mut()[phys as usize] = b,
        )
        .expect("second autoinit without unmask");
        assert_eq!(&mem.borrow()[0x1_1000..0x1_1002], &[0x11, 0x22]);
        assert_eq!(d.master.mask & 0x04, 0);
    }

    #[test]
    fn transfer_block_decrement_without_autoinit_auto_masks() {
        // Spec: Intel 8237A — auto-mask after non-autoinit TC is independent of
        // address direction (mode bit 5).
        let mut d = Dma8237::new();
        program_ch2_decrement_write(&mut d, 0x01, 0x1003, 1); // 2 bytes, mode 0x66
        let mem = RefCell::new(vec![0u8; 0x2_0000]);
        let mut io = [0xAAu8, 0xBB];
        d.transfer_block(
            2,
            &mut io,
            |phys| mem.borrow()[phys as usize],
            |phys, b| mem.borrow_mut()[phys as usize] = b,
        )
        .expect("decrement non-autoinit write");
        assert_eq!(d.master.mask & 0x04, 0x04);
        assert_eq!(d.master.status & 0x0F, 0x04);
    }

    #[test]
    fn transfer_block_ch0_without_autoinit_auto_masks() {
        // Spec: Intel 8237A — per-channel auto-mask on master ch0–3.
        let mut d = Dma8237::new();
        d.port_write(0x0C, 1, 0);
        d.port_write(0x00, 1, 0x00);
        d.port_write(0x00, 1, 0x10); // addr 0x1000
        d.port_write(0x0C, 1, 0);
        d.port_write(0x01, 1, 0x00); // count 0 → 1 byte
        d.port_write(0x01, 1, 0x00);
        d.port_write(DMA_PAGE_CH0, 1, 0x00);
        d.port_write(0x0B, 1, 0x44); // Single | Inc | Write | ch0
        d.port_write(0x0A, 1, 0x00); // unmask ch0
        let mem = RefCell::new(vec![0u8; 0x2000]);
        let mut io = [0x5Au8];
        d.transfer_block(
            0,
            &mut io,
            |phys| mem.borrow()[phys as usize],
            |phys, b| mem.borrow_mut()[phys as usize] = b,
        )
        .expect("ch0 write");
        assert_eq!(mem.borrow()[0x1000], 0x5A);
        assert_eq!(d.master.mask & 0x01, 0x01); // ch0 auto-masked
                                                // ch1–3 were still reset-masked (never unmasked); auto-mask only ORs ch0.
        assert_eq!(d.master.mask, 0x0F);
    }

    /// Program master ch2: Single+Dec+Write (`0x66`).
    fn program_ch2_decrement_write(d: &mut Dma8237, page: u8, addr: u16, count_minus_one: u16) {
        d.port_write(0x0C, 1, 0);
        d.port_write(0x04, 1, (addr & 0xFF) as u32);
        d.port_write(0x04, 1, (addr >> 8) as u32);
        d.port_write(0x0C, 1, 0);
        d.port_write(0x05, 1, (count_minus_one & 0xFF) as u32);
        d.port_write(0x05, 1, (count_minus_one >> 8) as u32);
        d.port_write(DMA_PAGE_CH2, 1, u32::from(page));
        d.port_write(0x0B, 1, 0x66); // Single | Dec | Write | ch2
        d.port_write(0x0A, 1, 0x02); // unmask ch2
    }

    #[test]
    fn transfer_block_decrement_write_moves_io_to_memory() {
        // Spec: Intel 8237A mode bit 5 = address decrement — Current Address
        // decrements by 1 per byte; wraps within the 64 KiB page (page register
        // unchanged). Without Autoinitialize, Current ends at the post-decrement
        // address and Current Word Count is 0xFFFF; TC latches.
        let mut d = Dma8237::new();
        program_ch2_decrement_write(&mut d, 0x01, 0x1003, 3); // 4 bytes
        let mem = RefCell::new(vec![0u8; 0x2_0000]);
        let mut io = [0xAAu8, 0xBB, 0xCC, 0xDD];
        let n = d
            .transfer_block(
                2,
                &mut io,
                |phys| mem.borrow()[phys as usize],
                |phys, b| mem.borrow_mut()[phys as usize] = b,
            )
            .expect("decrement write");
        assert_eq!(n, 4);
        assert_eq!(mem.borrow()[0x1_1003], 0xAA);
        assert_eq!(mem.borrow()[0x1_1002], 0xBB);
        assert_eq!(mem.borrow()[0x1_1001], 0xCC);
        assert_eq!(mem.borrow()[0x1_1000], 0xDD);
        assert_eq!(d.master.channels[2].addr, 0x0FFF);
        assert_eq!(d.master.channels[2].count, 0xFFFF);
        assert_eq!(d.master.status & 0x0F, 0x04);
        assert_eq!(d.page[2], 0x01);
    }

    #[test]
    fn transfer_block_decrement_read_moves_memory_to_io() {
        // Spec: Intel 8237A — address decrement + Read (bits 3:2 = 10).
        let mut d = Dma8237::new();
        d.port_write(0x0C, 1, 0);
        d.port_write(0x04, 1, 0x03);
        d.port_write(0x04, 1, 0x20); // addr 0x2003
        d.port_write(0x0C, 1, 0);
        d.port_write(0x05, 1, 0x01); // count 1 → 2 bytes
        d.port_write(0x05, 1, 0x00);
        d.port_write(DMA_PAGE_CH2, 1, 0x00);
        d.port_write(0x0B, 1, 0x6A); // Single | Dec | Read | ch2
        d.port_write(0x0A, 1, 0x02);
        let mem = RefCell::new(vec![0u8; 0x3000]);
        mem.borrow_mut()[0x2003] = 0x11;
        mem.borrow_mut()[0x2002] = 0x22;
        let mut io = [0u8; 2];
        let n = d
            .transfer_block(
                2,
                &mut io,
                |phys| mem.borrow()[phys as usize],
                |phys, b| mem.borrow_mut()[phys as usize] = b,
            )
            .expect("decrement read");
        assert_eq!(n, 2);
        assert_eq!(io, [0x11, 0x22]);
        assert_eq!(d.master.channels[2].addr, 0x2001);
        assert_eq!(d.master.channels[2].count, 0xFFFF);
        assert_eq!(d.master.status & 0x0F, 0x04);
    }

    #[test]
    fn transfer_block_decrement_wraps_within_64k_page() {
        // Spec: Intel 8237A / ISA DMA — 16-bit current address wraps on decrement;
        // page register does not auto-bump.
        let mut d = Dma8237::new();
        program_ch2_decrement_write(&mut d, 0x02, 0x0000, 1); // 2 bytes at page start
        let mem = RefCell::new(vec![0u8; 0x4_0000]);
        let mut io = [0xEEu8, 0xFF];
        d.transfer_block(
            2,
            &mut io,
            |phys| mem.borrow()[phys as usize],
            |phys, b| mem.borrow_mut()[phys as usize] = b,
        )
        .unwrap();
        assert_eq!(mem.borrow()[0x2_0000], 0xEE);
        assert_eq!(mem.borrow()[0x2_FFFF], 0xFF);
        assert_eq!(d.master.channels[2].addr, 0xFFFE); // wrapped past 0
        assert_eq!(d.page[2], 0x02);
    }

    #[test]
    fn transfer_block_decrement_autoinit_reloads_base() {
        // Spec: Intel 8237A — Autoinitialize (bit 4) still restores Current from
        // Base at TC when address decrement (bit 5) is selected; channel stays
        // unmasked/ready for another helper call.
        let mut d = Dma8237::new();
        d.port_write(0x0C, 1, 0);
        d.port_write(0x04, 1, 0x03);
        d.port_write(0x04, 1, 0x10); // addr 0x1003
        d.port_write(0x0C, 1, 0);
        d.port_write(0x05, 1, 0x03); // count 3 → 4 bytes
        d.port_write(0x05, 1, 0x00);
        d.port_write(DMA_PAGE_CH2, 1, 0x01);
        d.port_write(0x0B, 1, 0x76); // Single | AutoInit | Dec | Write | ch2
        d.port_write(0x0A, 1, 0x02);
        assert_eq!(d.master.channels[2].base_addr, 0x1003);
        assert_eq!(d.master.channels[2].base_count, 3);
        let mem = RefCell::new(vec![0u8; 0x2_0000]);
        let mut io = [0xAAu8, 0xBB, 0xCC, 0xDD];
        d.transfer_block(
            2,
            &mut io,
            |phys| mem.borrow()[phys as usize],
            |phys, b| mem.borrow_mut()[phys as usize] = b,
        )
        .expect("decrement autoinit write");
        assert_eq!(mem.borrow()[0x1_1003], 0xAA);
        assert_eq!(mem.borrow()[0x1_1000], 0xDD);
        assert_eq!(d.master.channels[2].addr, 0x1003);
        assert_eq!(d.master.channels[2].count, 3);
        assert_eq!(d.master.mask & 0x04, 0);
        assert_eq!(d.master.status & 0x0F, 0x04);

        let mut io2 = [0x11u8, 0x22, 0x33, 0x44];
        d.transfer_block(
            2,
            &mut io2,
            |phys| mem.borrow()[phys as usize],
            |phys, b| mem.borrow_mut()[phys as usize] = b,
        )
        .expect("second decrement autoinit write");
        assert_eq!(mem.borrow()[0x1_1003], 0x11);
        assert_eq!(mem.borrow()[0x1_1000], 0x44);
        assert_eq!(d.master.channels[2].addr, 0x1003);
        assert_eq!(d.master.channels[2].count, 3);
    }

    /// Program master ch2: Single+Inc+Verify (`0x42`).
    fn program_ch2_verify(d: &mut Dma8237, page: u8, addr: u16, count_minus_one: u16) {
        d.port_write(0x0C, 1, 0);
        d.port_write(0x04, 1, (addr & 0xFF) as u32);
        d.port_write(0x04, 1, (addr >> 8) as u32);
        d.port_write(0x0C, 1, 0);
        d.port_write(0x05, 1, (count_minus_one & 0xFF) as u32);
        d.port_write(0x05, 1, (count_minus_one >> 8) as u32);
        d.port_write(DMA_PAGE_CH2, 1, u32::from(page));
        d.port_write(0x0B, 1, 0x42); // Single | Inc | Verify | ch2
        d.port_write(0x0A, 1, 0x02); // unmask ch2
    }

    #[test]
    fn transfer_block_verify_increment_advances_without_memory_rw() {
        // Spec: Intel 8237A mode bits 3:2 = Verify (00) — address/count advance and
        // TC like a real transfer, but no memory or I/O data movement. Without
        // Autoinitialize the channel is hardware-masked after TC.
        let mut d = Dma8237::new();
        program_ch2_verify(&mut d, 0x01, 0x1000, 3); // 4 bytes
        let mem = RefCell::new(vec![0x5Au8; 0x2_0000]);
        let before = mem.borrow().clone();
        let mut io = [0xAAu8, 0xBB, 0xCC, 0xDD];
        let n = d
            .transfer_block(
                2,
                &mut io,
                |_| panic!("Verify must not call mem_read"),
                |_, _| panic!("Verify must not call mem_write"),
            )
            .expect("verify transfer");
        assert_eq!(n, 4);
        assert_eq!(io, [0xAA, 0xBB, 0xCC, 0xDD]); // io_buf payload untouched
        assert_eq!(*mem.borrow(), before); // memory unchanged
        assert_eq!(d.master.channels[2].addr, 0x1004);
        assert_eq!(d.master.channels[2].count, 0xFFFF);
        assert_eq!(d.port_read(0x08, 1) as u8 & 0x0F, 0x04); // TC ch2
        assert_eq!(d.master.mask & 0x04, 0x04); // auto-masked
                                                // Still require buffer length for API consistency.
        program_ch2_verify(&mut d, 0x01, 0x2000, 3);
        let mut short = [0u8; 2];
        assert_eq!(
            d.transfer_block(2, &mut short, |_| 0, |_, _| {}),
            Err(DmaTransferError::BufferTooShort)
        );
    }

    #[test]
    fn transfer_block_verify_autoinit_reloads_base_without_mask() {
        // Spec: Intel 8237A — Verify + Autoinitialize (bit 4): at TC Current reloads
        // from Base; channel is not auto-masked; still no memory R/W.
        let mut d = Dma8237::new();
        d.port_write(0x0C, 1, 0);
        d.port_write(0x04, 1, 0x00);
        d.port_write(0x04, 1, 0x10); // addr 0x1000
        d.port_write(0x0C, 1, 0);
        d.port_write(0x05, 1, 0x01); // count 1 → 2 bytes
        d.port_write(0x05, 1, 0x00);
        d.port_write(DMA_PAGE_CH2, 1, 0x01);
        d.port_write(0x0B, 1, 0x52); // Single | AutoInit | Inc | Verify | ch2
        d.port_write(0x0A, 1, 0x02);
        assert_eq!(d.master.channels[2].base_addr, 0x1000);
        assert_eq!(d.master.channels[2].base_count, 1);
        let mem = RefCell::new(vec![0xEEu8; 0x2_0000]);
        let before = mem.borrow().clone();
        let mut io = [0x11u8, 0x22];
        d.transfer_block(
            2,
            &mut io,
            |_| panic!("Verify must not call mem_read"),
            |_, _| panic!("Verify must not call mem_write"),
        )
        .expect("verify autoinit");
        assert_eq!(io, [0x11, 0x22]);
        assert_eq!(*mem.borrow(), before);
        assert_eq!(d.master.channels[2].addr, 0x1000);
        assert_eq!(d.master.channels[2].count, 1);
        assert_eq!(d.master.mask & 0x04, 0); // not auto-masked
        assert_eq!(d.master.status & 0x0F, 0x04);
        // Second verify without unmask must succeed.
        d.transfer_block(
            2,
            &mut io,
            |_| panic!("Verify must not call mem_read"),
            |_, _| panic!("Verify must not call mem_write"),
        )
        .expect("second verify autoinit");
        assert_eq!(d.master.channels[2].addr, 0x1000);
        assert_eq!(d.master.channels[2].count, 1);
    }

    #[test]
    fn transfer_block_verify_decrement_advances_without_memory_rw() {
        // Spec: Intel 8237A — Verify + address decrement (bit 5): Current Address
        // steps −1 per byte; no memory R/W; non-autoinit → auto-mask.
        let mut d = Dma8237::new();
        d.port_write(0x0C, 1, 0);
        d.port_write(0x04, 1, 0x03);
        d.port_write(0x04, 1, 0x10); // addr 0x1003
        d.port_write(0x0C, 1, 0);
        d.port_write(0x05, 1, 0x03); // 4 bytes
        d.port_write(0x05, 1, 0x00);
        d.port_write(DMA_PAGE_CH2, 1, 0x01);
        d.port_write(0x0B, 1, 0x62); // Single | Dec | Verify | ch2
        d.port_write(0x0A, 1, 0x02);
        let mem = RefCell::new(vec![0xA5u8; 0x2_0000]);
        let before = mem.borrow().clone();
        let mut io = [0xAAu8, 0xBB, 0xCC, 0xDD];
        let n = d
            .transfer_block(
                2,
                &mut io,
                |_| panic!("Verify must not call mem_read"),
                |_, _| panic!("Verify must not call mem_write"),
            )
            .expect("verify decrement");
        assert_eq!(n, 4);
        assert_eq!(io, [0xAA, 0xBB, 0xCC, 0xDD]);
        assert_eq!(*mem.borrow(), before);
        assert_eq!(d.master.channels[2].addr, 0x0FFF); // 0x1003 − 4
        assert_eq!(d.master.channels[2].count, 0xFFFF);
        assert_eq!(d.master.mask & 0x04, 0x04);
        assert_eq!(d.master.status & 0x0F, 0x04);
    }

    #[test]
    fn transfer_block_read_write_still_work_beside_verify() {
        // Smoke: Read/Write subset remains accepted after Verify support.
        let mut d = Dma8237::new();
        program_ch2_write(&mut d, 0x00, 0x0100, 0); // 1 byte Write
        let mem = RefCell::new(vec![0u8; 0x1000]);
        let mut io = [0x99u8];
        d.transfer_block(
            2,
            &mut io,
            |phys| mem.borrow()[phys as usize],
            |phys, b| mem.borrow_mut()[phys as usize] = b,
        )
        .expect("write smoke");
        assert_eq!(mem.borrow()[0x0100], 0x99);

        d.port_write(0x0C, 1, 0);
        d.port_write(0x04, 1, 0x00);
        d.port_write(0x04, 1, 0x02); // addr 0x0200
        d.port_write(0x0C, 1, 0);
        d.port_write(0x05, 1, 0x00); // 1 byte
        d.port_write(0x05, 1, 0x00);
        d.port_write(DMA_PAGE_CH2, 1, 0x00);
        d.port_write(0x0B, 1, 0x4A); // Single | Inc | Read | ch2
        d.port_write(0x0A, 1, 0x02);
        mem.borrow_mut()[0x0200] = 0x77;
        let mut io_r = [0u8];
        d.transfer_block(
            2,
            &mut io_r,
            |phys| mem.borrow()[phys as usize],
            |phys, b| mem.borrow_mut()[phys as usize] = b,
        )
        .expect("read smoke");
        assert_eq!(io_r, [0x77]);
    }
}
