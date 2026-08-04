//! Primary ATA IDE channel — IDENTIFY + READ/WRITE SECTORS PIO + IRQ14 stub.
//!
//! Classic PC primary command block `0x1F0`–`0x1F7` and control block `0x3F6`.
//!
//! # Spec refs
//!
//! - ATA / ATAPI Command Set — IDENTIFY DEVICE (`0xEC`), READ SECTORS (`0x20`),
//!   WRITE SECTORS (`0x30`), PACKET (`0xA0`), IDENTIFY PACKET DEVICE (`0xA1`),
//!   task-file registers, status bits BSY/DRDY/DRQ/ERR, error ABRT, LBA28
//!   addressing; device control nIEN; INTRQ when drive needs attention.
//! - OSDev ATA PIO Mode — primary port map, IDENTIFY/READ/WRITE IRQ+PIO sequence,
//!   status read clears IRQ / alternate status does not, 256-word PIO,
//!   sector-count `0` = 256 sectors; primary channel → ISA IRQ14;
//!   WRITE: host fills data port after DRQ; ATAPI probe via `0xA1` / PACKET.
//! - IBM PC/AT IDE — alternate status / device control at `0x3F6`; IRQ14.
//! - Intel 8259A — DualPic IR14 (slave IR6) vectoring via MachineBus.
//! - `docs/machine-model-pc-v1.md`, `plan.md` §15.5 / §21 PIIX IDE / ATAPI.
//!
//! # Scope (this slice)
//!
//! - Primary channel master only; optional backing image (`Vec<u8>`)
//! - Commands: IDENTIFY (`0xEC`), READ SECTORS (`0x20`), WRITE SECTORS (`0x30`) PIO
//! - IDENTIFY PACKET DEVICE (`0xA1`): ATA master → ERR+ABRT (no PACKET device);
//!   SeaBIOS-friendly reject of ATAPI probe on disk master
//! - PACKET (`0xA0`): ATA master → ERR+ABRT (no 12-byte packet PIO / DRQ);
//!   absent/slave → status 0; INTRQ follows nIEN like WRITE/IDENTIFY abort
//! - Status: BSY/DRDY/DRQ/ERR; alt status at `0x3F6` (no IRQ clear)
//! - Device control: SRST (bit2) software reset; nIEN gates IRQ14
//! - IRQ14: assert when DRQ ready / error / command-complete if nIEN=0;
//!   status register read clears pending IRQ; `irq_line()` for MachineBus
//! - `PortDevice` for MachineBus wiring
//!
//! # Unsupported (explicit)
//!
//! - ATAPI PACKET media engine / CD-ROM / ISO boot / slave ATAPI identify buffer
//! - DMA IDE (UDMA/MDMA), WRITE DMA, LBA48
//! - Slave drive on either channel
//! - SeaBIOS / PCI IDE BAR remapping
//!
//! Secondary channel (`IdeSecondary`) remaps the same ATA PIO stub to ports
//! `0x170`–`0x177` / `0x376` and ISA IRQ15 (see below).

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

/// Secondary ATA data port (16-bit PIO).
pub const IDE_SECONDARY_DATA: u16 = 0x170;
/// Secondary error (R) / Features (W).
pub const IDE_SECONDARY_ERROR: u16 = 0x171;
/// Secondary sector count.
pub const IDE_SECONDARY_SECCOUNT: u16 = 0x172;
/// Secondary LBA 7:0.
pub const IDE_SECONDARY_LBA_LO: u16 = 0x173;
/// Secondary LBA 15:8.
pub const IDE_SECONDARY_LBA_MID: u16 = 0x174;
/// Secondary LBA 23:16.
pub const IDE_SECONDARY_LBA_HI: u16 = 0x175;
/// Secondary drive/head select.
pub const IDE_SECONDARY_DRIVE: u16 = 0x176;
/// Secondary status (R) / Command (W).
pub const IDE_SECONDARY_STATUS: u16 = 0x177;
/// Secondary alternate status (R) / Device control (W).
pub const IDE_SECONDARY_CTRL: u16 = 0x376;

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
/// PACKET (ATAPI) — rejected with ABRT on ATA master (no packet PIO).
pub const ATA_CMD_PACKET: u8 = 0xA0;
/// IDENTIFY PACKET DEVICE (ATAPI) — rejected with ABRT on ATA master.
pub const ATA_CMD_IDENTIFY_PACKET: u8 = 0xA1;
/// READ SECTORS (with retry) — LBA28 PIO.
pub const ATA_CMD_READ_SECTORS: u8 = 0x20;
/// WRITE SECTORS (with retry) — LBA28 PIO.
pub const ATA_CMD_WRITE_SECTORS: u8 = 0x30;
/// FLUSH CACHE — non-data command; completes with success on ATA master.
/// Spec: ATA/ATAPI Command Set — FLUSH CACHE (`0xE7`).
pub const ATA_CMD_FLUSH_CACHE: u8 = 0xE7;
/// EXECUTE DEVICE DIAGNOSTIC — error=0x01 means passed (master).
/// Spec: ATA/ATAPI Command Set — EXECUTE DEVICE DIAGNOSTIC (`0x90`).
pub const ATA_CMD_DIAGNOSTIC: u8 = 0x90;
/// Diagnostic passed code in error register.
pub const ATA_DIAG_PASSED: u8 = 0x01;
/// SET FEATURES — non-data; this stub accepts and succeeds (no feature side effects).
/// Spec: ATA/ATAPI Command Set — SET FEATURES (`0xEF`).
pub const ATA_CMD_SET_FEATURES: u8 = 0xEF;
/// NOP — non-data success on ATA master (no side effects).
/// Spec: ATA/ATAPI Command Set — NOP (`0x00`).
pub const ATA_CMD_NOP: u8 = 0x00;
/// READ MULTIPLE — multi-sector PIO; this stub aborts (block count not configured).
/// Spec: ATA/ATAPI Command Set — READ MULTIPLE (`0xC4`).
pub const ATA_CMD_READ_MULTIPLE: u8 = 0xC4;
/// WRITE MULTIPLE — multi-sector PIO; this stub aborts (block count not configured).
/// Spec: ATA/ATAPI Command Set — WRITE MULTIPLE (`0xC5`).
pub const ATA_CMD_WRITE_MULTIPLE: u8 = 0xC5;
/// IDLE IMMEDIATE — non-data success.
/// Spec: ATA/ATAPI Command Set — IDLE IMMEDIATE (`0xE1`).
pub const ATA_CMD_IDLE_IMMEDIATE: u8 = 0xE1;
/// IDLE — non-data success (timer value in sector_count ignored by stub).
/// Spec: ATA/ATAPI Command Set — IDLE (`0xE3`).
pub const ATA_CMD_IDLE: u8 = 0xE3;
/// STANDBY IMMEDIATE — non-data success.
/// Spec: ATA/ATAPI Command Set — STANDBY IMMEDIATE (`0xE0`).
pub const ATA_CMD_STANDBY_IMMEDIATE: u8 = 0xE0;
/// CHECK POWER MODE — non-data; sector_count ← `0xFF` (Active/Idle).
/// Spec: ATA/ATAPI Command Set — CHECK POWER MODE (`0xE5`).
pub const ATA_CMD_CHECK_POWER_MODE: u8 = 0xE5;
/// CHECK POWER MODE result: device is Active or Idle.
pub const ATA_POWER_ACTIVE_OR_IDLE: u8 = 0xFF;
/// STANDBY — non-data success (timer in sector_count ignored).
/// Spec: ATA/ATAPI Command Set — STANDBY (`0xE2`).
pub const ATA_CMD_STANDBY: u8 = 0xE2;
/// SLEEP — non-data success.
/// Spec: ATA/ATAPI Command Set — SLEEP (`0xE6`).
pub const ATA_CMD_SLEEP: u8 = 0xE6;
/// RECALIBRATE — non-data success stub.
/// Spec: ATA/ATAPI Command Set — RECALIBRATE (`0x10`).
pub const ATA_CMD_RECALIBRATE: u8 = 0x10;
/// SEEK — non-data success stub.
/// Spec: ATA/ATAPI Command Set — SEEK (`0x70`).
pub const ATA_CMD_SEEK: u8 = 0x70;
/// INITIALIZE DEVICE PARAMETERS — non-data success stub.
/// Spec: ATA/ATAPI Command Set — INITIALIZE DEVICE PARAMETERS (`0x91`).
pub const ATA_CMD_INIT_DEV_PARAMS: u8 = 0x91;
/// FLUSH CACHE EXT — same non-data success as FLUSH CACHE in this stub.
/// Spec: ATA/ATAPI Command Set — FLUSH CACHE EXT (`0xEA`).
pub const ATA_CMD_FLUSH_CACHE_EXT: u8 = 0xEA;
/// READ NATIVE MAX ADDRESS — returns max LBA28 in task-file registers.
/// Spec: ATA/ATAPI Command Set — READ NATIVE MAX ADDRESS (`0xF8`).
pub const ATA_CMD_READ_NATIVE_MAX: u8 = 0xF8;
/// SET MULTIPLE MODE — ABRT stub (multiple block count not stored).
/// Spec: ATA/ATAPI Command Set — SET MULTIPLE MODE (`0xC6`).
pub const ATA_CMD_SET_MULTIPLE_MODE: u8 = 0xC6;

/// Error register: aborted command.
pub const ATA_ER_ABRT: u8 = 0x04;

/// Device control: software reset.
pub const ATA_DC_SRST: u8 = 0x04;
/// Device control: nIEN (1 = IRQ disabled / INTRQ not driven).
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
    /// Latched INTRQ request (gated by nIEN on [`Self::irq_line`]).
    irq_pending: bool,
    /// Current PIO sector payload (512 bytes).
    pio: [u8; SECTOR_SIZE],
    pio_off: usize,
    /// Sectors still to present/accept after the current PIO block (incl. current).
    sectors_left: u32,
    /// Next LBA to load (READ) or LBA of current PIO block (WRITE).
    next_lba: u32,
    /// True while host must drain/fill the data port under DRQ.
    transferring: bool,
    /// True = host→device WRITE PIO; false = device→host READ/IDENTIFY PIO.
    pio_in: bool,
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
            irq_pending: false,
            pio: [0; SECTOR_SIZE],
            pio_off: 0,
            sectors_left: 0,
            next_lba: 0,
            transferring: false,
            pio_in: false,
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
        self.irq_pending = false;
        self.pio = [0; SECTOR_SIZE];
        self.pio_off = 0;
        self.sectors_left = 0;
        self.next_lba = 0;
        self.transferring = false;
        self.pio_in = false;
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

    /// ISA IRQ14 line level (INTRQ ∧ ¬nIEN).
    ///
    /// Spec: ATA device control nIEN; OSDev ATA PIO — primary → IRQ14.
    pub fn irq_line(&self) -> bool {
        self.irq_pending && (self.dev_ctrl & ATA_DC_NIEN == 0)
    }

    fn raise_irq(&mut self) {
        // Spec: ATA — INTRQ asserted when drive needs attention; nIEN gates pin.
        self.irq_pending = true;
    }

    fn clear_irq(&mut self) {
        self.irq_pending = false;
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
        self.pio_in = false;
        self.transferring = true;
        self.status = ATA_SR_DRDY | ATA_SR_DSC | ATA_SR_DRQ;
        self.error = 0;
        // Spec: OSDev ATA PIO — IRQ when data ready (DRQ) if nIEN clear.
        self.raise_irq();
    }

    fn begin_pio_in(&mut self) {
        self.pio_off = 0;
        self.pio.fill(0);
        self.pio_in = true;
        self.transferring = true;
        self.status = ATA_SR_DRDY | ATA_SR_DSC | ATA_SR_DRQ;
        self.error = 0;
        // Spec: OSDev ATA PIO WRITE — IRQ when DRQ set (host may fill data).
        self.raise_irq();
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

    fn store_sector_from_pio(&mut self, lba: u32) -> bool {
        let total = self.total_sectors();
        if total == 0 || lba >= total {
            return false;
        }
        let start = (lba as usize) * SECTOR_SIZE;
        let end = start + SECTOR_SIZE;
        if end > self.image.len() {
            self.image.resize(end, 0);
        }
        self.image[start..end].copy_from_slice(&self.pio);
        true
    }

    fn abort_command(&mut self, error: u8) {
        self.error = error;
        self.transferring = false;
        self.pio_in = false;
        self.sectors_left = 0;
        self.status = ATA_SR_DRDY | ATA_SR_DSC | ATA_SR_ERR;
        // Spec: ATA — INTRQ on error completion when interrupts enabled.
        self.raise_irq();
    }

    fn exec_identify(&mut self) {
        // Spec: OSDev ATA PIO — no device / slave → status 0 after IDENTIFY.
        if !self.present || self.is_slave_selected() {
            self.status = 0;
            self.transferring = false;
            self.clear_irq();
            return;
        }
        self.fill_identify();
        self.sectors_left = 1;
        self.next_lba = 0;
        self.begin_pio_out();
    }

    /// IDENTIFY PACKET DEVICE (`0xA1`) on an ATA-only master.
    ///
    /// Spec: ATA/ATAPI — PACKET identify is valid for ATAPI devices; ATA disks
    /// abort with ERR+ABRT (no 256-word PIO). SeaBIOS probes `0xA1` to detect
    /// ATAPI; master stays ATA in this stub (no slave ATAPI path yet).
    fn exec_identify_packet(&mut self) {
        if !self.present || self.is_slave_selected() {
            self.status = 0;
            self.transferring = false;
            self.clear_irq();
            return;
        }
        self.abort_command(ATA_ER_ABRT);
    }

    /// PACKET (`0xA0`) on an ATA-only master.
    ///
    /// Spec: ATA/ATAPI — PACKET starts a 12-byte command packet transfer on
    /// ATAPI devices (DRQ). Non-ATAPI (ATA disk) devices abort with ERR+ABRT
    /// and no packet PIO. SeaBIOS-friendly: honest reject without a packet
    /// engine. INTRQ follows the same nIEN rules as WRITE/IDENTIFY abort.
    fn exec_packet(&mut self) {
        if !self.present || self.is_slave_selected() {
            self.status = 0;
            self.transferring = false;
            self.clear_irq();
            return;
        }
        self.abort_command(ATA_ER_ABRT);
    }

    fn exec_read_sectors(&mut self) {
        if !self.present || self.is_slave_selected() {
            self.status = 0;
            self.transferring = false;
            self.clear_irq();
            return;
        }
        // Require LBA bit for this stub (CHS not implemented).
        if self.drive_head & ATA_DRIVE_LBA == 0 {
            self.abort_command(ATA_ER_ABRT);
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

    fn exec_write_sectors(&mut self) {
        // Spec: ATA WRITE SECTORS (0x30) — LBA28 PIO; host fills 256 words/sector.
        if !self.present || self.is_slave_selected() {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        if self.drive_head & ATA_DRIVE_LBA == 0 {
            self.abort_command(ATA_ER_ABRT); // CHS unsupported
            return;
        }
        let count = self.sector_count_effective();
        let lba = self.lba28();
        let total = self.total_sectors();
        if total == 0 || lba >= total {
            self.abort_command(0x10); // IDNF
            return;
        }
        self.sectors_left = count;
        self.next_lba = lba;
        self.begin_pio_in();
    }

    /// FLUSH CACHE (`0xE7`) on ATA master — non-data success completion.
    ///
    /// Spec: ATA/ATAPI Command Set — FLUSH CACHE writes volatile cache to media.
    /// This stub has no volatile cache; it completes immediately with
    /// DRDY|DSC, error=0, no DRQ, and raises INTRQ when nIEN=0 (SeaBIOS-friendly).
    fn exec_flush_cache(&mut self) {
        if !self.present || self.is_slave_selected() {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.error = 0;
        self.transferring = false;
        self.pio_in = false;
        self.sectors_left = 0;
        self.status = ATA_SR_DRDY | ATA_SR_DSC;
        self.raise_irq();
    }

    /// NOP (`0x00`) on ATA master — non-data success (no side effects).
    ///
    /// Spec: ATA/ATAPI Command Set — NOP completes with success; this stub
    /// mirrors other non-data success completions (DRDY|DSC, error=0).
    fn exec_nop(&mut self) {
        if !self.present || self.is_slave_selected() {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.error = 0;
        self.transferring = false;
        self.pio_in = false;
        self.sectors_left = 0;
        self.status = ATA_SR_DRDY | ATA_SR_DSC;
        self.raise_irq();
    }

    /// READ MULTIPLE (`0xC4`) on ATA master — ABRT stub.
    ///
    /// Spec: ATA READ MULTIPLE requires a prior SET MULTIPLE MODE block count.
    /// This stub has no multiple-mode state; SeaBIOS-friendly ERR+ABRT.
    fn exec_read_multiple(&mut self) {
        if !self.present || self.is_slave_selected() {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.abort_command(ATA_ER_ABRT);
    }

    /// WRITE MULTIPLE (`0xC5`) on ATA master — ABRT stub.
    ///
    /// Spec: ATA WRITE MULTIPLE requires a prior SET MULTIPLE MODE block count.
    /// This stub has no multiple-mode state; SeaBIOS-friendly ERR+ABRT.
    fn exec_write_multiple(&mut self) {
        if !self.present || self.is_slave_selected() {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.abort_command(ATA_ER_ABRT);
    }

    /// IDLE / IDLE IMMEDIATE / STANDBY IMMEDIATE — non-data success stubs.
    ///
    /// Spec: ATA power-management commands complete with DRDY|DSC; this stub
    /// does not model timers or standby spin-down.
    fn exec_power_mgmt_success(&mut self) {
        if !self.present || self.is_slave_selected() {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.error = 0;
        self.transferring = false;
        self.pio_in = false;
        self.sectors_left = 0;
        self.status = ATA_SR_DRDY | ATA_SR_DSC;
        self.raise_irq();
    }

    /// CHECK POWER MODE (`0xE5`) — report Active/Idle via sector_count=`0xFF`.
    ///
    /// Spec: ATA CHECK POWER MODE returns power state in the sector count
    /// register (`0xFF` = Active or Idle). Stub always reports Active/Idle.
    fn exec_check_power_mode(&mut self) {
        if !self.present || self.is_slave_selected() {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.sector_count = ATA_POWER_ACTIVE_OR_IDLE;
        self.error = 0;
        self.transferring = false;
        self.pio_in = false;
        self.sectors_left = 0;
        self.status = ATA_SR_DRDY | ATA_SR_DSC;
        self.raise_irq();
    }

    /// RECALIBRATE (`0x10`) / SEEK (`0x70`) — non-data success stubs.
    ///
    /// Spec: ATA RECALIBRATE/SEEK complete with DRDY|DSC; this stub does not
    /// model physical head motion (DSC always set when ready).
    fn exec_recalibrate_seek_success(&mut self) {
        if !self.present || self.is_slave_selected() {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.error = 0;
        self.transferring = false;
        self.pio_in = false;
        self.sectors_left = 0;
        self.status = ATA_SR_DRDY | ATA_SR_DSC;
        self.raise_irq();
    }

    /// INITIALIZE DEVICE PARAMETERS (`0x91`) — non-data success stub.
    ///
    /// Spec: ATA INITIALIZE DEVICE PARAMETERS programs sectors/heads from the
    /// task file; this stub accepts and succeeds without changing geometry.
    fn exec_init_dev_params(&mut self) {
        if !self.present || self.is_slave_selected() {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.error = 0;
        self.transferring = false;
        self.pio_in = false;
        self.sectors_left = 0;
        self.status = ATA_SR_DRDY | ATA_SR_DSC;
        self.raise_irq();
    }

    /// SET MULTIPLE MODE (`0xC6`) — ABRT stub (no multiple-mode state).
    fn exec_set_multiple_mode(&mut self) {
        if !self.present || self.is_slave_selected() {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.abort_command(ATA_ER_ABRT);
    }

    /// READ NATIVE MAX ADDRESS (`0xF8`) — write max LBA28 into task-file regs.
    ///
    /// Spec: ATA READ NATIVE MAX ADDRESS returns the native maximum address in
    /// LBA Low/Mid/High and Device bits 3:0. This stub uses `total_sectors-1`
    /// (or 0 if empty). Completes with DRDY|DSC and INTRQ when nIEN=0.
    fn exec_read_native_max(&mut self) {
        if !self.present || self.is_slave_selected() {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        let max = self.total_sectors().saturating_sub(1);
        self.lba_lo = (max & 0xFF) as u8;
        self.lba_mid = ((max >> 8) & 0xFF) as u8;
        self.lba_hi = ((max >> 16) & 0xFF) as u8;
        self.drive_head = (self.drive_head & 0xF0) | (((max >> 24) & 0x0F) as u8);
        self.error = 0;
        self.transferring = false;
        self.pio_in = false;
        self.sectors_left = 0;
        self.status = ATA_SR_DRDY | ATA_SR_DSC;
        self.raise_irq();
    }

    /// EXECUTE DEVICE DIAGNOSTIC (`0x90`).
    ///
    /// Spec: ATA — runs diagnostics; error register `0x01` = device 0 passed.
    /// This stub always reports passed on present master; absent/slave → status 0.
    fn exec_diagnostic(&mut self) {
        if !self.present || self.is_slave_selected() {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.error = ATA_DIAG_PASSED;
        self.transferring = false;
        self.pio_in = false;
        self.sectors_left = 0;
        self.status = ATA_SR_DRDY | ATA_SR_DSC;
        self.raise_irq();
    }

    /// SET FEATURES (`0xEF`) — accept features register, succeed without side effects.
    ///
    /// Spec: ATA SET FEATURES uses the Features register as a subcommand.
    /// This stub completes successfully on present master (SeaBIOS-friendly
    /// accept); feature-specific behavior remains unsupported.
    fn exec_set_features(&mut self) {
        if !self.present || self.is_slave_selected() {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        let _subcmd = self.features; // accepted; no side effects yet
        self.error = 0;
        self.transferring = false;
        self.pio_in = false;
        self.sectors_left = 0;
        self.status = ATA_SR_DRDY | ATA_SR_DSC;
        self.raise_irq();
    }

    fn exec_command(&mut self, cmd: u8) {
        match cmd {
            ATA_CMD_IDENTIFY => self.exec_identify(),
            ATA_CMD_PACKET => self.exec_packet(),
            ATA_CMD_IDENTIFY_PACKET => self.exec_identify_packet(),
            ATA_CMD_READ_SECTORS => self.exec_read_sectors(),
            ATA_CMD_WRITE_SECTORS => self.exec_write_sectors(),
            ATA_CMD_FLUSH_CACHE | ATA_CMD_FLUSH_CACHE_EXT => self.exec_flush_cache(),
            ATA_CMD_NOP => self.exec_nop(),
            ATA_CMD_READ_MULTIPLE => self.exec_read_multiple(),
            ATA_CMD_WRITE_MULTIPLE => self.exec_write_multiple(),
            ATA_CMD_SET_MULTIPLE_MODE => self.exec_set_multiple_mode(),
            ATA_CMD_IDLE
            | ATA_CMD_IDLE_IMMEDIATE
            | ATA_CMD_STANDBY_IMMEDIATE
            | ATA_CMD_STANDBY
            | ATA_CMD_SLEEP => self.exec_power_mgmt_success(),
            ATA_CMD_CHECK_POWER_MODE => self.exec_check_power_mode(),
            ATA_CMD_RECALIBRATE | ATA_CMD_SEEK => self.exec_recalibrate_seek_success(),
            ATA_CMD_INIT_DEV_PARAMS => self.exec_init_dev_params(),
            ATA_CMD_READ_NATIVE_MAX => self.exec_read_native_max(),
            ATA_CMD_DIAGNOSTIC => self.exec_diagnostic(),
            ATA_CMD_SET_FEATURES => self.exec_set_features(),
            _ => self.abort_command(ATA_ER_ABRT), // unsupported command
        }
    }

    fn read_data(&mut self, size: u8) -> u32 {
        if !self.transferring || self.pio_in || self.status & ATA_SR_DRQ == 0 {
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
            self.finish_sector_pio_out();
        }
        val
    }

    fn write_data(&mut self, size: u8, value: u32) {
        if !self.transferring || !self.pio_in || self.status & ATA_SR_DRQ == 0 {
            return;
        }
        let nbytes = match size {
            4 => 4,
            2 => 2,
            _ => 1,
        };
        for i in 0..nbytes {
            if self.pio_off < SECTOR_SIZE {
                self.pio[self.pio_off] = ((value >> (8 * i)) & 0xFF) as u8;
                self.pio_off += 1;
            }
        }
        if self.pio_off >= SECTOR_SIZE {
            self.finish_sector_pio_in();
        }
    }

    fn finish_sector_pio_out(&mut self) {
        if self.sectors_left > 0 {
            self.sectors_left -= 1;
        }
        if self.sectors_left == 0 {
            self.transferring = false;
            self.pio_in = false;
            self.pio_off = 0;
            self.status = ATA_SR_DRDY | ATA_SR_DSC;
            self.sector_count = 0;
            // Spec: ATA — INTRQ on command completion after final sector.
            self.raise_irq();
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
        // Spec: OSDev ATA PIO — IRQ again when next sector DRQ ready.
        self.raise_irq();
    }

    fn finish_sector_pio_in(&mut self) {
        // Spec: ATA WRITE SECTORS — commit filled sector, then next DRQ or complete.
        let lba = self.next_lba;
        if !self.store_sector_from_pio(lba) {
            self.abort_command(0x10);
            return;
        }
        if self.sectors_left > 0 {
            self.sectors_left -= 1;
        }
        if self.sectors_left == 0 {
            self.transferring = false;
            self.pio_in = false;
            self.pio_off = 0;
            self.status = ATA_SR_DRDY | ATA_SR_DSC;
            self.sector_count = 0;
            // Spec: ATA — INTRQ on WRITE command completion.
            self.raise_irq();
            return;
        }
        self.next_lba = lba.wrapping_add(1);
        if self.next_lba >= self.total_sectors() {
            self.abort_command(0x10);
            return;
        }
        self.pio_off = 0;
        self.pio.fill(0);
        self.status = ATA_SR_DRDY | ATA_SR_DSC | ATA_SR_DRQ;
        if self.sector_count != 0 {
            self.sector_count = self.sector_count.wrapping_sub(1);
        }
        // Spec: OSDev ATA PIO WRITE — IRQ when next sector DRQ ready.
        self.raise_irq();
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
            self.clear_irq();
        } else if prev & ATA_DC_SRST != 0 && value & ATA_DC_SRST == 0 {
            if self.present {
                self.reset_ready();
            } else {
                self.status = 0;
                self.clear_irq();
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

    fn read_status_clear_irq(&mut self) -> u8 {
        // Spec: OSDev ATA PIO — reading Status (not alt) clears IRQ.
        self.clear_irq();
        self.status_byte()
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
            IDE_PRIMARY_STATUS => u32::from(self.read_status_clear_irq()),
            // Spec: alt status mirrors status without clearing IRQ.
            IDE_PRIMARY_CTRL => u32::from(self.status_byte()),
            _ => 0xFFFF_FFFF,
        }
    }

    fn port_write(&mut self, port: u16, size: u8, value: u32) {
        match port {
            IDE_PRIMARY_DATA => self.write_data(size, value),
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

/// Secondary ATA IDE channel — thin port remap of [`IdePrimary`] to `0x170`/`0x376`.
///
/// # Spec refs
///
/// - OSDev ATA PIO Mode — secondary command block `0x170`–`0x177`, control `0x376`;
///   secondary channel → ISA IRQ15.
/// - ATA / ATAPI — same IDENTIFY / READ / WRITE PIO semantics as primary.
/// - Intel 8259A — DualPic IR15 (slave IR7) via MachineBus.
///
/// # Scope
///
/// - Master only; IDENTIFY / READ / WRITE / PACKET+IDENTIFY PACKET ABRT via
///   inner [`IdePrimary`]
/// - IRQ15 when INTRQ ∧ ¬nIEN (`irq_line`)
///
/// # Unsupported
///
/// - Slave drive, DMA, LBA48, PACKET media engine, PCI BAR remap
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct IdeSecondary {
    /// Shared ATA PIO engine (ports remapped in [`PortDevice`]).
    pub inner: IdePrimary,
}

impl IdeSecondary {
    /// Empty secondary channel (no drive) — status reads `0`.
    pub fn new() -> Self {
        Self {
            inner: IdePrimary::new(),
        }
    }

    pub fn with_image(image: Vec<u8>) -> Self {
        Self {
            inner: IdePrimary::with_image(image),
        }
    }

    pub fn attach_image(&mut self, image: Vec<u8>) {
        self.inner.attach_image(image);
    }

    pub fn reset(&mut self) {
        self.inner.reset();
    }

    /// True if this device owns the secondary I/O port.
    pub fn owns_port(port: u16) -> bool {
        matches!(port, 0x170..=0x177 | IDE_SECONDARY_CTRL)
    }

    /// ISA IRQ15 line level (INTRQ ∧ ¬nIEN).
    ///
    /// Spec: ATA device control nIEN; OSDev ATA PIO — secondary → IRQ15.
    pub fn irq_line(&self) -> bool {
        self.inner.irq_line()
    }

    /// Map secondary ports onto the primary register file used by [`IdePrimary`].
    fn map_port(port: u16) -> u16 {
        match port {
            0x170..=0x177 => port - IDE_SECONDARY_DATA + IDE_PRIMARY_DATA,
            IDE_SECONDARY_CTRL => IDE_PRIMARY_CTRL,
            _ => port,
        }
    }
}

impl PortDevice for IdeSecondary {
    fn port_read(&mut self, port: u16, size: u8) -> u32 {
        self.inner.port_read(Self::map_port(port), size)
    }

    fn port_write(&mut self, port: u16, size: u8, value: u32) {
        self.inner.port_write(Self::map_port(port), size, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identify_word(pio: &[u8; SECTOR_SIZE], idx: usize) -> u16 {
        let off = idx * 2;
        u16::from(pio[off]) | (u16::from(pio[off + 1]) << 8)
    }

    fn clear_nien(ide: &mut IdePrimary) {
        ide.port_write(IDE_PRIMARY_CTRL, 1, 0);
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

    #[test]
    fn identify_asserts_irq14_when_nien_clear() {
        // Spec: ATA + OSDev ATA PIO — INTRQ when DRQ ready if nIEN=0.
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
        assert!(ide.irq_line());
    }

    #[test]
    fn nien_set_masks_irq_line() {
        // Spec: ATA device control — nIEN=1 disables INTRQ pin.
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        // Default reset leaves nIEN set.
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
        assert!(!ide.irq_line());
    }

    #[test]
    fn status_read_clears_irq_alt_does_not() {
        // Spec: OSDev ATA PIO — Status clears IRQ; alternate status does not.
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY));
        assert!(ide.irq_line());
        let _ = ide.port_read(IDE_PRIMARY_CTRL, 1);
        assert!(ide.irq_line(), "alt status must not clear IRQ");
        let _ = ide.port_read(IDE_PRIMARY_STATUS, 1);
        assert!(!ide.irq_line());
    }

    #[test]
    fn read_sectors_asserts_irq_on_drq() {
        // Spec: ATA READ SECTORS — IRQ when sector data ready (DRQ).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 1);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_SECTORS));
        assert!(ide.irq_line());
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
    }

    #[test]
    fn error_completion_asserts_irq_when_nien_clear() {
        // Spec: ATA — INTRQ on error completion.
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 1);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 5);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_SECTORS));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(ide.irq_line());
    }

    #[test]
    fn multi_sector_raises_irq_per_drq_block() {
        // Spec: OSDev ATA PIO — IRQ for each sector DRQ when interrupts enabled.
        let mut img = vec![0u8; SECTOR_SIZE * 2];
        img[0] = 0x11;
        img[SECTOR_SIZE] = 0x22;
        let mut ide = IdePrimary::with_image(img);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 2);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_SECTORS));
        assert!(ide.irq_line());
        let _ = ide.port_read(IDE_PRIMARY_STATUS, 1); // ack first IRQ
        assert!(!ide.irq_line());
        for _ in 0..256 {
            let _ = ide.port_read(IDE_PRIMARY_DATA, 2);
        }
        // Second sector under DRQ → IRQ again.
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
        assert!(ide.irq_line());
    }

    fn write_sector_words(ide: &mut IdePrimary, first: u16, last: u16) {
        ide.port_write(IDE_PRIMARY_DATA, 2, u32::from(first));
        for _ in 1..255 {
            ide.port_write(IDE_PRIMARY_DATA, 2, 0);
        }
        ide.port_write(IDE_PRIMARY_DATA, 2, u32::from(last));
    }

    #[test]
    fn write_sectors_lba28_pio() {
        // Spec: ATA WRITE SECTORS (0x30) — LBA28, 256 words/sector into media.
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE * 3]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 1);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 1);
        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_WRITE_SECTORS));
        assert_ne!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);
        write_sector_words(&mut ide, 0x55AA, 0xC300);
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_eq!(st & ATA_SR_ERR, 0);
        assert_eq!(ide.image[SECTOR_SIZE], 0xAA);
        assert_eq!(ide.image[SECTOR_SIZE + 1], 0x55);
        assert_eq!(ide.image[SECTOR_SIZE + 511], 0xC3);
    }

    #[test]
    fn write_sectors_multi_two() {
        // Spec: ATA WRITE SECTORS — multi-sector PIO commits each sector in order.
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE * 2]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 2);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_WRITE_SECTORS));
        assert_ne!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);
        write_sector_words(&mut ide, 0x0011, 0);
        assert_ne!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);
        write_sector_words(&mut ide, 0x0022, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);
        assert_eq!(ide.image[0], 0x11);
        assert_eq!(ide.image[SECTOR_SIZE], 0x22);
    }

    #[test]
    fn write_oob_sets_err() {
        // Spec: ATA — out-of-range LBA → ERR (IDNF-style).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 1);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 5);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_WRITE_SECTORS));
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_ne!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
    }

    #[test]
    fn write_sectors_asserts_irq_on_drq_and_complete() {
        // Spec: ATA WRITE + OSDev — IRQ at DRQ and again at command complete.
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 1);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_WRITE_SECTORS));
        assert!(ide.irq_line());
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
        let _ = ide.port_read(IDE_PRIMARY_STATUS, 1); // ack DRQ IRQ
        assert!(!ide.irq_line());
        write_sector_words(&mut ide, 0xBEEF, 0);
        // Completion IRQ after final sector commit.
        assert!(ide.irq_line());
        assert_eq!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
        let _ = ide.port_read(IDE_PRIMARY_STATUS, 1);
        assert!(!ide.irq_line());
    }

    #[test]
    fn write_nien_masks_irq_line() {
        // Spec: ATA device control — nIEN=1 disables INTRQ during WRITE.
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 1);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_WRITE_SECTORS));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
        assert!(!ide.irq_line());
    }

    #[test]
    fn write_then_read_round_trip() {
        // Spec: WRITE then READ SECTORS see committed media.
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE * 2]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 1);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_WRITE_SECTORS));
        write_sector_words(&mut ide, 0x55AA, 0xC300);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 1);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_SECTORS));
        assert_eq!(ide.port_read(IDE_PRIMARY_DATA, 2) as u16, 0x55AA);
        for _ in 1..255 {
            let _ = ide.port_read(IDE_PRIMARY_DATA, 2);
        }
        assert_eq!(ide.port_read(IDE_PRIMARY_DATA, 2) as u16, 0xC300);
    }

    #[test]
    fn identify_packet_aborts_on_ata_master() {
        // Spec: ATA/ATAPI — IDENTIFY PACKET DEVICE (0xA1) on ATA disk → ERR+ABRT.
        // Master remains ATA; no ATAPI identify PIO buffer in this stub.
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY_PACKET));
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_ne!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_ER_ABRT);
    }

    #[test]
    fn identify_packet_asserts_irq_on_abort_when_nien_clear() {
        // Spec: ATA — INTRQ on error completion (SeaBIOS may poll or use IRQ).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY_PACKET));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(ide.irq_line());
    }

    #[test]
    fn identify_packet_absent_drive_status_zero() {
        // Spec: OSDev ATA PIO — missing device probe → status 0.
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY_PACKET));
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
    }

    #[test]
    fn packet_aborts_on_ata_master() {
        // Spec: ATA/ATAPI — PACKET (0xA0) is for ATAPI devices; ATA disk → ERR+ABRT.
        // No 12-byte packet PIO / DRQ phase on non-ATAPI master (SeaBIOS-friendly).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_PACKET));
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_ne!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_ER_ABRT);
    }

    #[test]
    fn packet_asserts_irq_on_abort_when_nien_clear() {
        // Spec: ATA — INTRQ on error completion when nIEN=0 (match WRITE/IDENTIFY).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_PACKET));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(ide.irq_line());
    }

    #[test]
    fn packet_nien_masks_irq_on_abort() {
        // Spec: ATA device control — nIEN=1 masks INTRQ (match WRITE SECTORS).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_CTRL, 1, u32::from(ATA_DC_NIEN));
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_PACKET));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(!ide.irq_line());
    }

    #[test]
    fn packet_absent_drive_status_zero() {
        // Spec: OSDev ATA PIO — missing device → status 0 (not ABRT from catch-all).
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_PACKET));
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
        assert!(!ide.irq_line());
    }

    /// Spec: ATA/ATAPI Command Set — FLUSH CACHE (`0xE7`) non-data success on
    /// ATA master: DRDY|DSC, error=0, no DRQ; INTRQ when nIEN=0.
    #[test]
    fn flush_cache_succeeds_on_ata_master() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_FLUSH_CACHE));
        // Spec: ATA — INTRQ asserted on completion; status read clears it.
        assert!(ide.irq_line());
        let st = ide.port_read(IDE_PRIMARY_CTRL, 1) as u8; // alt status: no IRQ clear
        assert_eq!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_ne!(st & ATA_SR_DSC, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
        assert!(ide.irq_line(), "alt status must not clear INTRQ");
    }

    #[test]
    fn flush_cache_nien_masks_irq() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_CTRL, 1, u32::from(ATA_DC_NIEN));
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_FLUSH_CACHE));
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
        assert!(!ide.irq_line());
    }

    /// Spec: ATA — EXECUTE DEVICE DIAGNOSTIC (`0x90`) → error=0x01 passed.
    #[test]
    fn diagnostic_passes_on_ata_master() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_DIAGNOSTIC));
        assert!(ide.irq_line());
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_DIAG_PASSED);
        let st = ide.port_read(IDE_PRIMARY_CTRL, 1) as u8;
        assert_eq!(st & ATA_SR_ERR, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
    }

    /// Spec: ATA — SET FEATURES (`0xEF`) succeeds on ATA master (no side effects).
    #[test]
    fn set_features_succeeds_on_ata_master() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_ERROR, 1, 0x03); // features write via error port alias
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_SET_FEATURES));
        assert!(ide.irq_line());
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
        let st = ide.port_read(IDE_PRIMARY_CTRL, 1) as u8;
        assert_eq!(st & ATA_SR_ERR, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
    }

    #[test]
    fn diagnostic_absent_drive_status_zero() {
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_DIAGNOSTIC));
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
    }

    #[test]
    fn set_features_absent_drive_status_zero() {
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_SET_FEATURES));
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
    }

    #[test]
    fn flush_cache_absent_drive_status_zero() {
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_FLUSH_CACHE));
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
        assert!(!ide.irq_line());
    }

    /// Spec: ATA/ATAPI — NOP (`0x00`) non-data success on ATA master.
    #[test]
    fn nop_succeeds_on_ata_master() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_NOP));
        assert!(ide.irq_line());
        let st = ide.port_read(IDE_PRIMARY_CTRL, 1) as u8;
        assert_eq!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_ne!(st & ATA_SR_DSC, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
    }

    #[test]
    fn nop_absent_drive_status_zero() {
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_NOP));
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
        assert!(!ide.irq_line());
    }

    /// Spec: ATA — READ MULTIPLE (`0xC4`) without SET MULTIPLE MODE → ERR+ABRT.
    #[test]
    fn read_multiple_aborts_on_ata_master() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_MULTIPLE));
        assert!(ide.irq_line());
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_ER_ABRT);
        let st = ide.port_read(IDE_PRIMARY_CTRL, 1) as u8;
        assert_ne!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
    }

    #[test]
    fn read_multiple_absent_drive_status_zero() {
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_MULTIPLE));
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
        assert!(!ide.irq_line());
    }

    /// Spec: ATA — WRITE MULTIPLE (`0xC5`) without SET MULTIPLE MODE → ERR+ABRT.
    #[test]
    fn write_multiple_aborts_on_ata_master() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_WRITE_MULTIPLE));
        assert!(ide.irq_line());
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_ER_ABRT);
        let st = ide.port_read(IDE_PRIMARY_CTRL, 1) as u8;
        assert_ne!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
    }

    /// Spec: ATA — IDLE / IDLE IMMEDIATE / STANDBY IMMEDIATE / STANDBY / SLEEP
    /// succeed; CHECK POWER MODE sets sector_count=`0xFF` (Active/Idle).
    #[test]
    fn idle_standby_sleep_and_check_power_mode() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        for cmd in [
            ATA_CMD_IDLE,
            ATA_CMD_IDLE_IMMEDIATE,
            ATA_CMD_STANDBY_IMMEDIATE,
            ATA_CMD_STANDBY,
            ATA_CMD_SLEEP,
        ] {
            ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(cmd));
            assert!(ide.irq_line());
            let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8; // clears IRQ
            assert_eq!(st & ATA_SR_ERR, 0);
            assert_ne!(st & ATA_SR_DRDY, 0);
        }
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 0x00);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_CHECK_POWER_MODE));
        assert!(ide.irq_line());
        assert_eq!(
            ide.port_read(IDE_PRIMARY_SECCOUNT, 1) as u8,
            ATA_POWER_ACTIVE_OR_IDLE
        );
        let st = ide.port_read(IDE_PRIMARY_CTRL, 1) as u8;
        assert_eq!(st & ATA_SR_ERR, 0);
    }

    /// Spec: ATA — RECALIBRATE (`0x10`) and SEEK (`0x70`) non-data success.
    #[test]
    fn recalibrate_and_seek_succeed() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        for cmd in [ATA_CMD_RECALIBRATE, ATA_CMD_SEEK] {
            ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(cmd));
            assert!(ide.irq_line());
            let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
            assert_eq!(st & ATA_SR_ERR, 0);
            assert_ne!(st & ATA_SR_DSC, 0);
        }
    }

    /// Spec: ATA — INITIALIZE DEVICE PARAMETERS (`0x91`) non-data success.
    #[test]
    fn init_device_parameters_succeeds() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 63);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_INIT_DEV_PARAMS));
        assert!(ide.irq_line());
        let st = ide.port_read(IDE_PRIMARY_CTRL, 1) as u8;
        assert_eq!(st & ATA_SR_ERR, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
    }

    /// Spec: ATA — FLUSH CACHE EXT (`0xEA`) same success path as FLUSH CACHE.
    #[test]
    fn flush_cache_ext_succeeds_like_flush_cache() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_FLUSH_CACHE_EXT));
        assert!(ide.irq_line());
        let st = ide.port_read(IDE_PRIMARY_CTRL, 1) as u8;
        assert_eq!(st & ATA_SR_ERR, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
    }

    /// Spec: ATA — READ NATIVE MAX ADDRESS (`0xF8`) writes max LBA into task file.
    #[test]
    fn read_native_max_address_writes_task_file() {
        // 4 sectors → max LBA = 3.
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE * 4]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_NATIVE_MAX));
        assert!(ide.irq_line());
        assert_eq!(ide.port_read(IDE_PRIMARY_LBA_LO, 1) as u8, 3);
        assert_eq!(ide.port_read(IDE_PRIMARY_LBA_MID, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_LBA_HI, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_DRIVE, 1) as u8 & 0x0F, 0);
        let st = ide.port_read(IDE_PRIMARY_CTRL, 1) as u8;
        assert_eq!(st & ATA_SR_ERR, 0);
    }

    /// Spec: ATA — SET MULTIPLE MODE (`0xC6`) → ERR+ABRT (no multiple state).
    #[test]
    fn set_multiple_mode_aborts() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 16);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_SET_MULTIPLE_MODE));
        assert!(ide.irq_line());
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_ER_ABRT);
    }

    #[test]
    fn secondary_absent_drive_status_is_zero() {
        // Spec: OSDev ATA PIO — secondary missing drive → status 0.
        let mut ide = IdeSecondary::new();
        assert!(IdeSecondary::owns_port(IDE_SECONDARY_STATUS));
        assert!(IdeSecondary::owns_port(IDE_SECONDARY_CTRL));
        assert!(!IdeSecondary::owns_port(IDE_PRIMARY_STATUS));
        assert_eq!(ide.port_read(IDE_SECONDARY_STATUS, 1) as u8, 0);
        ide.port_write(IDE_SECONDARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_SECONDARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY));
        assert_eq!(ide.port_read(IDE_SECONDARY_STATUS, 1) as u8, 0);
    }

    #[test]
    fn secondary_identify_and_read_sectors() {
        // Spec: ATA IDENTIFY + READ on secondary ports 0x170–0x177.
        let mut sector = vec![0u8; SECTOR_SIZE];
        sector[0] = 0x11;
        sector[1] = 0x22;
        let mut ide = IdeSecondary::with_image(sector);
        ide.port_write(IDE_SECONDARY_CTRL, 1, 0); // clear nIEN
        ide.port_write(IDE_SECONDARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_SECONDARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY));
        // Alt status does not clear IRQ15.
        assert_ne!(ide.port_read(IDE_SECONDARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
        assert!(ide.irq_line());
        let _ = ide.port_read(IDE_SECONDARY_STATUS, 1); // ack IRQ
        assert!(!ide.irq_line());
        for _ in 0..256 {
            let _ = ide.port_read(IDE_SECONDARY_DATA, 2);
        }
        assert_eq!(ide.port_read(IDE_SECONDARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);

        ide.port_write(IDE_SECONDARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_SECONDARY_SECCOUNT, 1, 1);
        ide.port_write(IDE_SECONDARY_LBA_LO, 1, 0);
        ide.port_write(IDE_SECONDARY_LBA_MID, 1, 0);
        ide.port_write(IDE_SECONDARY_LBA_HI, 1, 0);
        ide.port_write(IDE_SECONDARY_STATUS, 1, u32::from(ATA_CMD_READ_SECTORS));
        assert_eq!(ide.port_read(IDE_SECONDARY_DATA, 2) as u16, 0x2211);
    }

    #[test]
    fn secondary_alt_status_does_not_clear_irq() {
        // Spec: OSDev ATA PIO — alt status at 0x376 does not clear IRQ15.
        let mut ide = IdeSecondary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_SECONDARY_CTRL, 1, 0);
        ide.port_write(IDE_SECONDARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_SECONDARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY));
        assert!(ide.irq_line());
        let _ = ide.port_read(IDE_SECONDARY_CTRL, 1);
        assert!(ide.irq_line());
        let _ = ide.port_read(IDE_SECONDARY_STATUS, 1);
        assert!(!ide.irq_line());
    }
}
