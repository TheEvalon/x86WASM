//! Primary ATA IDE channel — IDENTIFY + READ SECTORS PIO stub.
//!
//! Classic PC primary command block `0x1F0`–`0x1F7` and control block `0x3F6`.
//!
//! # Spec refs
//!
//! - ATA / ATAPI Command Set — IDENTIFY DEVICE (`0xEC`), READ SECTORS (`0x20`),
//!   task-file registers, status bits BSY/DRDY/DRQ/ERR, LBA28 addressing.
//! - OSDev ATA PIO Mode — primary port map, IDENTIFY/READ polling sequence,
//!   256-word PIO transfers, sector-count `0` = 256 sectors.
//! - IBM PC/AT IDE — alternate status / device control at `0x3F6`.
//! - `docs/machine-model-pc-v1.md`, `plan.md` §15.5 / §21 PIIX IDE.
//!
//! # Scope (this slice)
//!
//! - Primary channel master only; optional backing image (`Vec<u8>`)
//! - Commands: IDENTIFY DEVICE (`0xEC`), READ SECTORS (`0x20`) PIO
//! - Status: BSY/DRDY/DRQ/ERR; alt status at `0x3F6` (no IRQ side effects)
//! - Device control: SRST (bit2) software reset; nIEN stored
//! - `PortDevice` for MachineBus wiring
//!
//! # Unsupported (explicit)
//!
//! - ATAPI PACKET / IDENTIFY PACKET DEVICE
//! - WRITE SECTORS, DMA IDE (UDMA/MDMA), LBA48
//! - Slave drive, secondary channel (`0x170`), IRQ14 delivery
//! - SeaBIOS / PCI IDE BAR remapping

use crate::PortDevice;

/// Primary ATA data port (16-bit PIO).
pub const IDE_PRIMARY_DATA: u16 = 0x1F0;
/// Error (R) / Features (W).
pub const IDE_PRIMARY_ERROR: u16 = 0x1F1;
/// Sector count.
pub const IDE_PRIMARY_SECCOUNT: u16 = 0x1F2;
/// LBA 7:0 / sector number.
pub const IDE_PRIMARY_LBA_LO: u16 = 0x1F3;
/// LBA 15:8 / cylinder low.
pub const IDE_PRIMARY_LBA_MID: u16 = 0x1F4;
/// LBA 23:16 / cylinder high.
pub const IDE_PRIMARY_LBA_HI: u16 = 0x1F5;
/// Drive/head select + LBA 27:24.
pub const IDE_PRIMARY_DRIVE: u16 = 0x1F6;
/// Status (R) / Command (W).
pub const IDE_PRIMARY_STATUS: u16 = 0x1F7;
/// Alternate status (R) / Device control (W).
pub const IDE_PRIMARY_CTRL: u16 = 0x3F6;

/// Status: busy.
pub const ATA_SR_BSY: u8 = 0x80;
/// Status: drive ready.
pub const ATA_SR_DRDY: u8 = 0x40;
/// Status: drive seek complete (stub always set with DRDY when ready).
pub const ATA_SR_DSC: u8 = 0x10;
/// Status: data request.
pub const ATA_SR_DRQ: u8 = 0x08;
/// Status: error.
pub const ATA_SR_ERR: u8 = 0x01;

/// IDENTIFY DEVICE.
pub const ATA_CMD_IDENTIFY: u8 = 0xEC;
/// READ SECTORS (with retry) — LBA28 PIO.
pub const ATA_CMD_READ_SECTORS: u8 = 0x20;

/// Device control: software reset.
pub const ATA_DC_SRST: u8 = 0x04;
/// Device control: nIEN (1 = IRQ disabled). Stored only; IRQ14 unsupported.
pub const ATA_DC_NIEN: u8 = 0x02;

/// Drive/head: LBA mode bit.
pub const ATA_DRIVE_LBA: u8 = 0x40;
/// Drive/head: slave select (bit4). Master = 0.
pub const ATA_DRIVE_SLAVE: u8 = 0x10;

const SECTOR_SIZE: usize = 512;
const IDENTIFY_WORDS: usize = 256;

/// Primary IDE channel (master drive stub).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdePrimary {
    /// When false, status reads as `0` (no device) until a drive is attached.
    pub present: bool,
    /// Backing image bytes (multiple of 512 preferred; short reads zero-pad).
    pub image: Vec<u8>,
    error: u8,
    features: u8,
    sector_count: u8,
    lba_lo: u8,
    lba_mid: u8,
    lba_hi: u8,
    drive_head: u8,
    status: u8,
    dev_ctrl: u8,
    /// Current PIO sector payload (512 bytes).
    pio: [u8; SECTOR_SIZE],
    pio_off: usize,
    /// Sectors still to present after the current PIO block (incl. current).
    sectors_left: u32,
    /// Next LBA to load when advancing multi-sector READ.
    next_lba: u32,
    /// True while host must drain/fill the data port under DRQ.
    transferring: bool,
}

impl Default for IdePrimary {
    fn default() -> Self {
        Self::new()
    }
}

impl IdePrimary {
    /// Empty channel (no drive) — status reads `0`.
    pub fn new() -> Self {
        Self {
            present: false,
            image: Vec::new(),
            error: 0,
            features: 0,
            sector_count: 0,
            lba_lo: 0,
            lba_mid: 0,
            lba_hi: 0,
            drive_head: 0xA0,
            status: 0,
            dev_ctrl: ATA_DC_NIEN,
            pio: [0; SECTOR_SIZE],
            pio_off: 0,
            sectors_left: 0,
            next_lba: 0,
            transferring: false,
        }
    }

    /// Attach a master disk image and mark the drive present / ready.
    ///
    /// Spec: ATA — after power-on / reset, DRDY set when ready to accept commands.
    pub fn with_image(image: Vec<u8>) -> Self {
        let mut ide = Self::new();
        ide.attach_image(image);
        ide
    }

    pub fn attach_image(&mut self, image: Vec<u8>) {
        self.image = image;
        self.present = true;
        self.reset_ready();
    }

    pub fn reset(&mut self) {
        // Preserve backing image / presence across Machine::reset.
        let image = std::mem::take(&mut self.image);
        let present = self.present;
        *self = Self::new();
        self.image = image;
        self.present = present;
        if self.present {
            self.reset_ready();
        }
    }

    fn reset_ready(&mut self) {
        self.error = 0;
        self.features = 0;
        self.sector_count = 1;
        self.lba_lo = 1;
        self.lba_mid = 0;
        self.lba_hi = 0;
        self.drive_head = 0xA0;
        self.dev_ctrl = ATA_DC_NIEN;
        self.pio = [0; SECTOR_SIZE];
        self.pio_off = 0;
        self.sectors_left = 0;
        self.next_lba = 0;
        self.transferring = false;
        self.status = if self.present {
            ATA_SR_DRDY | ATA_SR_DSC
        } else {
            0
        };
    }

    /// True if this device owns the I/O port.
    pub fn owns_port(port: u16) -> bool {
        matches!(port, 0x1F0..=0x1F7 | IDE_PRIMARY_CTRL)
    }

    fn is_slave_selected(&self) -> bool {
        self.drive_head & ATA_DRIVE_SLAVE != 0
    }

    fn lba28(&self) -> u32 {
        let hi = u32::from(self.drive_head & 0x0F) << 24;
        hi | (u32::from(self.lba_hi) << 16)
            | (u32::from(self.lba_mid) << 8)
            | u32::from(self.lba_lo)
    }

    fn sector_count_effective(&self) -> u32 {
        if self.sector_count == 0 {
            256
        } else {
            u32::from(self.sector_count)
        }
    }

    fn total_sectors(&self) -> u32 {
        (self.image.len() / SECTOR_SIZE) as u32
    }

    /// Build a minimal IDENTIFY DEVICE payload (256 words, little-endian words).
    ///
    /// Spec: ATA IDENTIFY DEVICE — words 60–61 = total LBA28 user sectors;
    /// word 49 bit9 = LBA supported; model string words 27–46 (byte-swapped).
    fn fill_identify(&mut self) {
        let mut words = [0u16; IDENTIFY_WORDS];
        words[0] = 0x0040; // non-removable ATA disk (bit6)
        words[1] = 16383; // obsolete cylinders
        words[3] = 16; // obsolete heads
        words[6] = 63; // obsolete sectors/track
                       // Model: "x86WASM IDE STUB" padded, ATA byte-swap within words.
        let model = b"x86WASM IDE STUB                        ";
        for (i, chunk) in model.chunks(2).take(20).enumerate() {
            let a = chunk.first().copied().unwrap_or(b' ');
            let b = chunk.get(1).copied().unwrap_or(b' ');
            words[27 + i] = u16::from(a) << 8 | u16::from(b);
        }
        words[47] = 0x8000 | 16; // max sectors per DRQ (stub)
        words[49] = 1 << 9; // LBA supported
        words[53] = 0x0001; // words 54–58 valid (legacy)
        let total = self.total_sectors().max(1);
        words[60] = (total & 0xFFFF) as u16;
        words[61] = (total >> 16) as u16;
        words[63] = 0; // no multiword DMA
        words[80] = 1 << 4; // ATA/ATAPI-4 major version bit (informational)
        words[82] = 0;
        words[83] = 0x4000; // bit14 must be 1 in word 83
        words[85] = 0;
        words[86] = 0;

        for (i, w) in words.iter().enumerate() {
            let off = i * 2;
            self.pio[off] = (*w & 0xFF) as u8;
            self.pio[off + 1] = (*w >> 8) as u8;
        }
    }

    fn begin_pio_out(&mut self) {
        self.pio_off = 0;
        self.transferring = true;
        self.status = ATA_SR_DRDY | ATA_SR_DSC | ATA_SR_DRQ;
        self.error = 0;
    }

    fn load_sector_into_pio(&mut self, lba: u32) -> bool {
        let total = self.total_sectors();
        if total == 0 || lba >= total {
            return false;
        }
        let start = (lba as usize) * SECTOR_SIZE;
        self.pio.fill(0);
        let end = (start + SECTOR_SIZE).min(self.image.len());
        if start < self.image.len() {
            let n = end - start;
            self.pio[..n].copy_from_slice(&self.image[start..end]);
        }
        true
    }

    fn abort_command(&mut self, error: u8) {
        self.error = error;
        self.transferring = false;
        self.sectors_left = 0;
        self.status = ATA_SR_DRDY | ATA_SR_DSC | ATA_SR_ERR;
    }

    fn exec_identify(&mut self) {
        // Spec: OSDev ATA PIO — no device / slave → status 0 after IDENTIFY.
        if !self.present || self.is_slave_selected() {
            self.status = 0;
            self.transferring = false;
            return;
        }
        self.fill_identify();
        self.sectors_left = 1;
        self.next_lba = 0;
        self.begin_pio_out();
    }

    fn exec_read_sectors(&mut self) {
        if !self.present || self.is_slave_selected() {
            self.status = 0;
            self.transferring = false;
            return;
        }
        // Require LBA bit for this stub (CHS not implemented).
        if self.drive_head & ATA_DRIVE_LBA == 0 {
            self.abort_command(0x04); // ABRT
            return;
        }
        let count = self.sector_count_effective();
        let lba = self.lba28();
        if !self.load_sector_into_pio(lba) {
            self.abort_command(0x10); // IDNF / sector not found style
            return;
        }
        self.sectors_left = count;
        self.next_lba = lba.wrapping_add(1);
        self.begin_pio_out();
    }

    fn exec_command(&mut self, cmd: u8) {
        match cmd {
            ATA_CMD_IDENTIFY => self.exec_identify(),
            ATA_CMD_READ_SECTORS => self.exec_read_sectors(),
            _ => self.abort_command(0x04), // ABRT — unsupported command
        }
    }

    fn read_data(&mut self, size: u8) -> u32 {
        if !self.transferring || self.status & ATA_SR_DRQ == 0 {
            return 0xFFFF_FFFF;
        }
        let mut val = 0u32;
        let nbytes = match size {
            4 => 4,
            2 => 2,
            _ => 1,
        };
        for i in 0..nbytes {
            if self.pio_off < SECTOR_SIZE {
                val |= u32::from(self.pio[self.pio_off]) << (8 * i);
                self.pio_off += 1;
            }
        }
        if self.pio_off >= SECTOR_SIZE {
            self.finish_sector_pio();
        }
        val
    }

    fn finish_sector_pio(&mut self) {
        if self.sectors_left > 0 {
            self.sectors_left -= 1;
        }
        if self.sectors_left == 0 {
            self.transferring = false;
            self.pio_off = 0;
            self.status = ATA_SR_DRDY | ATA_SR_DSC;
            self.sector_count = 0;
            return;
        }
        // Multi-sector READ: present next sector.
        if !self.load_sector_into_pio(self.next_lba) {
            self.abort_command(0x10);
            return;
        }
        self.next_lba = self.next_lba.wrapping_add(1);
        self.pio_off = 0;
        self.status = ATA_SR_DRDY | ATA_SR_DSC | ATA_SR_DRQ;
        // Spec: sector count decrements as sectors transfer.
        if self.sector_count != 0 {
            self.sector_count = self.sector_count.wrapping_sub(1);
        }
    }

    fn write_dev_ctrl(&mut self, value: u8) {
        let prev = self.dev_ctrl;
        self.dev_ctrl = value & (ATA_DC_SRST | ATA_DC_NIEN | 0x01);
        // Spec: ATA device control — SRST high then low performs software reset.
        if prev & ATA_DC_SRST == 0 && value & ATA_DC_SRST != 0 {
            // Enter reset: BSY
            if self.present && !self.is_slave_selected() {
                self.status = ATA_SR_BSY;
            }
        } else if prev & ATA_DC_SRST != 0 && value & ATA_DC_SRST == 0 {
            if self.present {
                self.reset_ready();
            } else {
                self.status = 0;
            }
        }
    }

    fn status_byte(&self) -> u8 {
        // No slave / absent: floating bus reads 0x00 for IDENTIFY probe.
        if !self.present || self.is_slave_selected() {
            return 0;
        }
        self.status
    }
}

impl PortDevice for IdePrimary {
    fn port_read(&mut self, port: u16, size: u8) -> u32 {
        match port {
            IDE_PRIMARY_DATA => self.read_data(size),
            IDE_PRIMARY_ERROR => u32::from(self.error),
            IDE_PRIMARY_SECCOUNT => u32::from(self.sector_count),
            IDE_PRIMARY_LBA_LO => u32::from(self.lba_lo),
            IDE_PRIMARY_LBA_MID => u32::from(self.lba_mid),
            IDE_PRIMARY_LBA_HI => u32::from(self.lba_hi),
            IDE_PRIMARY_DRIVE => u32::from(self.drive_head),
            IDE_PRIMARY_STATUS => u32::from(self.status_byte()),
            IDE_PRIMARY_CTRL => u32::from(self.status_byte()), // alt status
            _ => 0xFFFF_FFFF,
        }
    }

    fn port_write(&mut self, port: u16, size: u8, value: u32) {
        match port {
            IDE_PRIMARY_DATA => {
                // Writes unsupported in this slice (no WRITE SECTORS).
                let _ = (size, value);
            }
            IDE_PRIMARY_ERROR => self.features = value as u8,
            IDE_PRIMARY_SECCOUNT => self.sector_count = value as u8,
            IDE_PRIMARY_LBA_LO => self.lba_lo = value as u8,
            IDE_PRIMARY_LBA_MID => self.lba_mid = value as u8,
            IDE_PRIMARY_LBA_HI => self.lba_hi = value as u8,
            IDE_PRIMARY_DRIVE => {
                self.drive_head = value as u8;
                // Selecting absent slave yields status 0 on subsequent status reads.
            }
            IDE_PRIMARY_STATUS => {
                // Command register.
                if self.status & ATA_SR_BSY != 0 {
                    return;
                }
                self.exec_command(value as u8);
            }
            IDE_PRIMARY_CTRL => self.write_dev_ctrl(value as u8),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identify_word(pio: &[u8; SECTOR_SIZE], idx: usize) -> u16 {
        let off = idx * 2;
        u16::from(pio[off]) | (u16::from(pio[off + 1]) << 8)
    }

    #[test]
    fn absent_drive_status_is_zero() {
        // Spec: OSDev ATA PIO — IDENTIFY on missing drive → status 0.
        let mut ide = IdePrimary::new();
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY));
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
    }

    #[test]
    fn identify_sets_drq_and_returns_256_words() {
        // Spec: ATA IDENTIFY DEVICE — 256 words via data port when DRQ set.
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE * 4]);
        assert_eq!(
            ide.port_read(IDE_PRIMARY_STATUS, 1) as u8,
            ATA_SR_DRDY | ATA_SR_DSC
        );
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY));
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_eq!(st & ATA_SR_BSY, 0);
        assert_ne!(st & ATA_SR_DRQ, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_LBA_MID, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_LBA_HI, 1) as u8, 0);

        let mut words = Vec::with_capacity(IDENTIFY_WORDS);
        for _ in 0..IDENTIFY_WORDS {
            words.push(ide.port_read(IDE_PRIMARY_DATA, 2) as u16);
        }
        assert_eq!(words[49] & (1 << 9), 1 << 9, "LBA supported");
        assert_eq!(words[60], 4);
        assert_eq!(words[61], 0);
        let st_done = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_eq!(st_done & ATA_SR_DRQ, 0);
        assert_ne!(st_done & ATA_SR_DRDY, 0);
    }

    #[test]
    fn read_sectors_lba28_pio() {
        // Spec: ATA READ SECTORS (0x20) — LBA28, sector count, 256 words/sector.
        let mut img = vec![0u8; SECTOR_SIZE * 3];
        img[SECTOR_SIZE] = 0xAA;
        img[SECTOR_SIZE + 1] = 0x55;
        img[SECTOR_SIZE + 511] = 0xC3;
        let mut ide = IdePrimary::with_image(img);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 1);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 1);
        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_SECTORS));
        assert_ne!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);
        let w0 = ide.port_read(IDE_PRIMARY_DATA, 2) as u16;
        assert_eq!(w0, 0x55AA);
        for _ in 1..255 {
            let _ = ide.port_read(IDE_PRIMARY_DATA, 2);
        }
        let w_last = ide.port_read(IDE_PRIMARY_DATA, 2) as u16;
        // Little-endian word at bytes 510–511: low=0x00, high=0xC3.
        assert_eq!(w_last, 0xC300);
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);
    }

    #[test]
    fn read_sectors_multi_two() {
        let mut img = vec![0u8; SECTOR_SIZE * 2];
        img[0] = 0x11;
        img[SECTOR_SIZE] = 0x22;
        let mut ide = IdePrimary::with_image(img);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 2);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_SECTORS));
        let first = ide.port_read(IDE_PRIMARY_DATA, 2) as u16;
        assert_eq!(first & 0xFF, 0x11);
        for _ in 1..256 {
            let _ = ide.port_read(IDE_PRIMARY_DATA, 2);
        }
        // Second sector should now be under DRQ.
        assert_ne!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);
        let second = ide.port_read(IDE_PRIMARY_DATA, 2) as u16;
        assert_eq!(second & 0xFF, 0x22);
        for _ in 1..256 {
            let _ = ide.port_read(IDE_PRIMARY_DATA, 2);
        }
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);
    }

    #[test]
    fn read_oob_sets_err() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 1);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 5);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_SECTORS));
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_ne!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
    }

    #[test]
    fn alt_status_mirrors_status() {
        // Spec: IBM PC/AT — 0x3F6 alternate status mirrors status without IRQ ack.
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        assert_eq!(
            ide.port_read(IDE_PRIMARY_CTRL, 1) as u8,
            ide.port_read(IDE_PRIMARY_STATUS, 1) as u8
        );
    }

    #[test]
    fn srst_restores_ready() {
        // Spec: ATA device control SRST pulse → software reset.
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_CTRL, 1, u32::from(ATA_DC_SRST | ATA_DC_NIEN));
        assert_ne!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8 & ATA_SR_BSY, 0);
        ide.port_write(IDE_PRIMARY_CTRL, 1, u32::from(ATA_DC_NIEN));
        assert_eq!(
            ide.port_read(IDE_PRIMARY_STATUS, 1) as u8,
            ATA_SR_DRDY | ATA_SR_DSC
        );
    }

    #[test]
    fn owns_primary_ports_only() {
        assert!(IdePrimary::owns_port(IDE_PRIMARY_DATA));
        assert!(IdePrimary::owns_port(IDE_PRIMARY_STATUS));
        assert!(IdePrimary::owns_port(IDE_PRIMARY_CTRL));
        assert!(!IdePrimary::owns_port(0x170));
        assert!(!IdePrimary::owns_port(0x3F7));
    }

    #[test]
    fn identify_total_sectors_in_words_60_61() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE * 0x1_0001]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY));
        assert_eq!(identify_word(&ide.pio, 60), 0x0001);
        assert_eq!(identify_word(&ide.pio, 61), 0x0001);
    }
}
