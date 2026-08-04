//! Dual Intel 8237A ISA DMA controllers — registers/pages + single-channel helper.
//!
//! Classic PC: master (ch0–3) at `0x00`–`0x0F`, slave (ch4–7) at even ports
//! `0xC0`–`0xDE`, plus external page address registers.
//!
//! # Spec refs
//!
//! - Intel 8237A High Performance Programmable DMA Controller — address/count
//!   registers, byte pointer flip-flop, mode/mask/command/status, master reset,
//!   terminal count (word count = N−1), current address/count after transfer.
//! - OSDev Wiki ISA DMA — AT port map and page register assignment
//!   (`0x87`/`0x83`/`0x81`/`0x82` ch0–3; `0x8F`/`0x8B`/`0x89`/`0x8A` ch4–7);
//!   8-bit channel physical address `(page << 16) | addr`.
//! - IBM PC/AT: port `0x80` is POST/diagnostic scratch, **not** a DMA page.
//! - `docs/machine-model-pc-v1.md`, `plan.md` §15.3 / §21 DMA.
//!
//! # Scope (this slice)
//!
//! - Dual 8237A programming model: addr/count with flip-flop, mode, masks,
//!   command/request accept, status read, master/mask reset.
//! - Base Address / Base Word Count loaded whenever Current is programmed
//!   (Intel 8237A programming model).
//! - Status register TC bits (3:0) clear-on-read + `latch_tc` device/test API.
//! - `transfer_block` software helper for 8-bit channels 0–3: length `count+1`,
//!   Single + Increment/Decrement + Read/Write, Autoinitialize optional,
//!   `mem_read` / `mem_write` callbacks. With Autoinitialize, after TC Current
//!   is reloaded from Base and the channel stays unmasked/ready for another
//!   helper call. Without Autoinitialize, after TC the channel mask bit is set
//!   (Intel 8237A hardware auto-mask). Address decrement (mode bit 5) steps
//!   Current Address by −1 per byte within the 64 KiB page.
//! - Page address register R/W for the eight AT channels above.
//! - `PortDevice` for MachineBus wiring.
//!
//! # Unsupported (explicit)
//!
//! - Hardware DREQ/DACK handshake / cycle-accurate bus timing
//! - Demand/block/cascade/verify modes
//! - 16-bit channels 4–7 word addressing / transfers
//! - Floppy / IDE automatic DMA engine / DREQ path (Machine PhysMem wiring lives in
//!   `machine-pc::Machine::dma_transfer`; no SeaBIOS floppy DMA)

use crate::PortDevice;

/// Error from [`Dma8237::transfer_block`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmaTransferError {
    /// ISA channel outside the 8-bit subset (`0`–`3`) this helper supports.
    BadChannel,
    /// Channel mask bit is set (masked channels do not transfer).
    Masked,
    /// Mode register outside Single + Inc/Dec + Read/Write (+ optional Autoinitialize).
    UnsupportedMode,
    /// `io_buf` shorter than programmed `count + 1`.
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
    /// bits 7:4 = request pending (DREQ path unsupported; stay 0 unless latched).
    pub status: u8,
    /// Software request register (stored).
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

    /// Spec: Intel 8237A master clear — masks all channels, clears flip-flop.
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
                // (bits 7:4); TC bits clear on read. Master clear also clears them.
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
            9 => self.request = value,
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

    /// Complete one programmed 8-bit channel transfer via memory callbacks.
    ///
    /// Spec: Intel 8237A — word count holds N−1 so length is `count + 1`; after N
    /// transfers the channel TC status bit is set. Without Autoinitialize,
    /// Current Address ends at the post-step value, Current Word Count is
    /// `0xFFFF`, and the channel is hardware-masked (mask bit set). With
    /// Autoinitialize (mode bit 4), Current Address / Word Count are restored
    /// from Base Address / Base Word Count at TC and the channel remains ready
    /// (not auto-masked). Address direction follows mode bit 5: `0` increments
    /// Current Address by +1 per byte, `1` decrements by −1 per byte (both wrap
    /// in the 64 KiB page; page register is not auto-bumped).
    /// Physical byte address = `(page << 16) | current_addr` (OSDev ISA DMA).
    ///
    /// # Mode subset honored
    ///
    /// - bits 7:6 = Single mode (`01`)
    /// - bit 5 = address increment (`0`) or decrement (`1`)
    /// - bit 4 = Autoinitialize enable (`0` or `1`)
    /// - bits 3:2 = Write (`01`, I/O→memory from `io_buf` via `mem_write`) or
    ///   Read (`10`, memory→I/O into `io_buf` via `mem_read`)
    /// - bits 1:0 must match the controller channel index
    ///
    /// Demand/block/cascade/verify and ISA channels 4–7 return
    /// [`DmaTransferError::UnsupportedMode`] / [`BadChannel`].
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
        if isa_channel > 3 {
            return Err(DmaTransferError::BadChannel);
        }
        if self.master.mask & (1 << isa_channel) != 0 {
            return Err(DmaTransferError::Masked);
        }
        let mode = self.master.channels[isa_channel].mode;
        if !Self::mode_supports_transfer_block(mode, isa_channel) {
            return Err(DmaTransferError::UnsupportedMode);
        }
        let len = usize::from(self.master.channels[isa_channel].count).wrapping_add(1);
        if io_buf.len() < len {
            return Err(DmaTransferError::BufferTooShort);
        }
        let page = self.page[isa_channel];
        let write_to_mem = ((mode >> 2) & 0x03) == 0b01;
        let auto_init = (mode >> 4) & 1 != 0;
        let decrement = (mode >> 5) & 1 != 0;
        let mut addr = self.master.channels[isa_channel].addr;
        for byte in io_buf.iter_mut().take(len) {
            let phys = (u32::from(page) << 16) | u32::from(addr);
            if write_to_mem {
                mem_write(phys, *byte);
            } else {
                *byte = mem_read(phys);
            }
            // Spec: Intel 8237A mode bit 5 — address increment (0) / decrement (1).
            addr = if decrement {
                addr.wrapping_sub(1)
            } else {
                addr.wrapping_add(1)
            };
        }
        if auto_init {
            // Spec: Intel 8237A Autoinitialize — restore Current from Base at TC;
            // channel is not masked and remains ready for another transfer.
            let ch = &mut self.master.channels[isa_channel];
            ch.addr = ch.base_addr;
            ch.count = ch.base_count;
        } else {
            // Spec: Intel 8237A — without Autoinitialize, after TC Current ends at
            // the post-step address / 0xFFFF and the channel is hardware-masked.
            self.master.channels[isa_channel].addr = addr;
            self.master.channels[isa_channel].count = 0xFFFF;
            self.master.mask |= 1 << isa_channel;
        }
        self.master.latch_tc(isa_channel);
        Ok(len)
    }

    /// Single + Inc/Dec + Read/Write, Autoinitialize optional, channel matches.
    fn mode_supports_transfer_block(mode: u8, ctrl_channel: usize) -> bool {
        let sel = (mode & 0x03) as usize;
        let xfer = (mode >> 2) & 0x03;
        let mode_sel = (mode >> 6) & 0x03;
        sel == ctrl_channel && mode_sel == 0b01 && (xfer == 0b01 || xfer == 0b10)
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
    fn software_request_and_mask_unchanged_with_tc() {
        // Existing programming model must stay green beside TC latch/status.
        let mut d = Dma8237::new();
        d.port_write(0x09, 1, 0x04); // software request set ch0
        assert_eq!(d.master.request, 0x04);
        d.port_write(0x0A, 1, 0x00); // unmask ch0
        assert_eq!(d.master.mask & 0x01, 0);
        d.latch_tc(0);
        assert_eq!(d.port_read(0x08, 1) as u8 & 0x0F, 0x01);
        assert_eq!(d.master.request, 0x04);
        assert_eq!(d.master.mask & 0x01, 0);
    }

    #[test]
    fn mode_register_stored() {
        let mut d = Dma8237::new();
        d.port_write(0x0B, 1, 0x46); // ch2, single mode style pattern
        assert_eq!(d.master.channels[2].mode, 0x46);
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
        // Block mode (bits 7:6 = 10) instead of Single — out of subset.
        d.port_write(0x0B, 1, 0x86);
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
    fn transfer_block_rejects_word_channels_and_short_buffer() {
        let mut d = Dma8237::new();
        let mut io = [0u8; 4];
        assert_eq!(
            d.transfer_block(5, &mut io, |_| 0, |_, _| {}),
            Err(DmaTransferError::BadChannel)
        );
        program_ch2_write(&mut d, 0, 0, 3); // wants 4 bytes
        let mut short = [0u8; 2];
        assert_eq!(
            d.transfer_block(2, &mut short, |_| 0, |_, _| {}),
            Err(DmaTransferError::BufferTooShort)
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
}
