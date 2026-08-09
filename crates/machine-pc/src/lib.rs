//! Classic PC machine: CPU lab, serial HELLO ROM, and M2 PIC/PIT/CMOS/8042/DMA/VGA/PCI/IDE/FDC wiring.
//!
//! Floppy media for tests/host setup: [`Machine::attach_floppy_image`] /
//! [`Machine::with_floppy`] wrap [`Fdc82077::attach_image`].
//!
//! IDE / boot handoff: [`Machine::attach_ide_image`] / [`Machine::with_ide`] and
//! [`Machine::load_mbr_to_7c00`] (see [`mbr`]).

#![forbid(unsafe_code)]

mod hello_rom;
mod mbr;
mod mem;
mod ports;
mod post_code;
mod post_probe;

pub use hello_rom::{build_hello_rom, EXPECTED_HELLO};
pub use mbr::{MBR_PHYS_ADDR, MBR_SECTOR_SIZE, MBR_SIGNATURE_HI, MBR_SIGNATURE_LO};
pub use mem::{
    PamAttributes, PamRead, PamWrite, PhysMem, PAM_BIOS_REGION, PAM_FIELD_MASK, PAM_FIELD_RE,
    PAM_FIELD_WE, PAM_REGIONS, PAM_REGION_COUNT, PAM_REGISTER_FIRST, PAM_REGISTER_LAST,
    PAM_WINDOW_BASE, PAM_WINDOW_END,
};
pub use ports::{
    UnclaimedPortAccess, UnmappedMmioAccess, UNCLAIMED_PORT_LIMIT, UNMAPPED_MMIO_LIMIT,
    UNMAPPED_MMIO_PAGE_SIZE,
};
pub use post_code::{PostCodePort, POST_CODE_HISTORY_LIMIT, POST_DIAG_PORT};
pub use post_probe::{
    seabios_image_path, PostFailure, PostFailureKind, PostReport, PostStopReason,
    DEFAULT_POST_PROBE_STEPS, POST_OPCODE_WINDOW_LEN, SEABIOS_IMAGE_ENV, SEABIOS_IMAGE_RELATIVE,
};

use devices::{
    CmosRtc, DebugConsole, Dma8237, DmaTransferError, DualPic, Fdc82077, FwCfg, FwCfgDmaOutcome,
    IdePrimary, IdeSecondary, PciConfig, Pit8254, Port92, PortDevice, Serial16550, VgaText,
    CMOS_DATA, CMOS_INDEX, FDC_DOR_DMA_IRQ, I8042, I8042_DATA, I8042_STATUS_CMD, PIC_MASTER_CMD,
    PIC_MASTER_DATA, PIC_SLAVE_CMD, PIC_SLAVE_DATA, PIIX_ELCR_MASTER, PIIX_ELCR_SLAVE,
    PIT_CH0_DATA, PIT_CH1_DATA, PIT_CH2_DATA, PIT_CONTROL, PORT_SYSTEM_CONTROL,
    PORT_SYSTEM_CONTROL_A,
};
use firmware_interface::{prepare_bios_rom, BiosRomError, RomImage};
use ports::PortBus;
use thiserror::Error;
use x86_core::CpuState;
use x86_interpreter::{step, Bus, ExecError};

#[derive(Debug, Error)]
pub enum MachineError {
    #[error(transparent)]
    Exec(#[from] ExecError),
    #[error("ROM too large for window")]
    RomTooLarge,
    #[error(transparent)]
    BiosRom(#[from] BiosRomError),
    /// No IDE LBA0 / floppy CHS (0,0,1) available for [`Machine::load_mbr_to_7c00`].
    #[error("no boot media attached (IDE LBA0 or floppy CHS 0,0,1)")]
    NoBootMedia,
    /// IDE image present but shorter than one 512-byte sector.
    #[error("boot sector shorter than 512 bytes")]
    IncompleteBootSector,
    /// Bytes 510–511 are not the classic `0x55AA` MBR/VBR signature.
    #[error("invalid MBR signature (expected 0x55AA)")]
    InvalidMbrSignature,
    /// Guest RAM must cover `0x7C00`..`0x7DFF` for the boot-sector copy.
    #[error("RAM too small for MBR at 0x7C00")]
    MbrRamTooSmall,
}

pub struct Machine {
    pub cpu: CpuState,
    pub mem: PhysMem,
    pub com1: Serial16550,
    /// COM2 (`0x2F8`–`0x2FF`) — same 16550 debug-UART stub as COM1.
    pub com2: Serial16550,
    pub debug: DebugConsole,
    /// Dual 8259A — ICW + OCW/IRQ (ports 0x20/0x21/0xA0/0xA1).
    pub pic: DualPic,
    /// 8254 PIT — channel-0 programming + OUT tick (ports 0x40–0x43); OUT → IRQ0.
    pub pit: Pit8254,
    /// MC146818 CMOS/RTC (ports 0x70/0x71); PIE/AIE/UIE → IRQ8.
    pub cmos: CmosRtc,
    /// 8042 / PS/2 controller (ports 0x60/0x64); OBF+INT1 → IRQ1,
    /// AUX OBF+INT12 → IRQ12 (second-port controller side; no mouse device).
    pub kbd: I8042,
    /// System Control Port A (`0x92`) — Fast Gate A20 + fast reset pulse.
    pub port92: Port92,
    /// Dual 8237A DMA — register/page stubs (ports 0x00–0x0F, 0xC0–0xDE, pages).
    pub dma: Dma8237,
    /// VGA color text plane at 0xB8000 + CRTC/Seq/GC/ATC/DAC/Misc stubs.
    pub vga: VgaText,
    /// PCI configuration mechanism #1 (ports 0xCF8 / 0xCFC–0xCFF).
    pub pci: PciConfig,
    /// Primary IDE — IDENTIFY + READ SECTORS PIO (ports 0x1F0–0x1F7 / 0x3F6).
    pub ide: IdePrimary,
    /// Secondary IDE — same PIO stub remapped (ports 0x170–0x177 / 0x376); IRQ15.
    pub ide_secondary: IdeSecondary,
    /// 82077AA FDC — ports 0x3F0–0x3F5 / 0x3F7; media READ DATA + DMA ch2 wire.
    pub fdc: Fdc82077,
    /// QEMU fw_cfg — signature, ID, configured RAM size, and test file.
    pub fw_cfg: FwCfg,
    /// POST checkpoint latch on the manufacturing diagnostic port `0x80`.
    pub post_diag: PostCodePort,
    ports: PortBus,
}

impl Machine {
    pub fn new(ram_size: usize) -> Self {
        Self {
            cpu: CpuState::reset(),
            mem: PhysMem::new(ram_size),
            com1: Serial16550::new(0x3F8),
            com2: Serial16550::new(0x2F8),
            debug: DebugConsole::new(),
            pic: DualPic::new(),
            pit: Pit8254::new(),
            cmos: CmosRtc::new(),
            kbd: I8042::new(),
            port92: Port92::new(),
            dma: Dma8237::new(),
            vga: VgaText::new(),
            pci: PciConfig::new(),
            ide: IdePrimary::new(),
            ide_secondary: IdeSecondary::new(),
            fdc: Fdc82077::new(),
            fw_cfg: FwCfg::with_ram_size(ram_size as u64),
            post_diag: PostCodePort::new(),
            ports: PortBus::new(),
        }
    }

    /// Attach a raw 1.44MB floppy image to [`Self::fdc`].
    ///
    /// Wraps [`Fdc82077::attach_image`] (exact [`devices::FDC_1440_IMAGE_SIZE`]
    /// bytes). Spec: IBM PC 1.44MB MFM. Does not clear DIR DSKCHG.
    pub fn attach_floppy_image(&mut self, image: Vec<u8>) -> Result<(), &'static str> {
        self.fdc.attach_image(image)
    }

    /// Construct a machine with a 1.44MB floppy image already attached.
    ///
    /// Wraps [`Self::new`] + [`Self::attach_floppy_image`].
    pub fn with_floppy(ram_size: usize, image: Vec<u8>) -> Result<Self, &'static str> {
        let mut m = Self::new(ram_size);
        m.attach_floppy_image(image)?;
        Ok(m)
    }

    /// Load a 64 KiB (or smaller) ROM at `0xFFFF_0000` for the Intel reset vector.
    ///
    /// Lab / HELLO path — high map only. Prefer [`Self::load_bios_rom`] when a
    /// classic below-1 MiB (`0xF0000`) alias is required.
    pub fn load_rom(&mut self, data: &[u8]) -> Result<(), MachineError> {
        if data.len() > 64 * 1024 {
            return Err(MachineError::RomTooLarge);
        }
        let mut rom = vec![0u8; 64 * 1024];
        if data.len() == 64 * 1024 {
            rom.copy_from_slice(data);
        } else {
            // Small images start at ROM offset 0; caller must include a reset
            // vector at 0xFFF0 when using a full 64 KiB buffer (HELLO does).
            rom[..data.len()].copy_from_slice(data);
        }
        self.mem.map_rom(0xFFFF_0000, rom);
        Ok(())
    }

    pub fn load_rom_image(&mut self, image: &RomImage) -> Result<(), MachineError> {
        self.mem.map_rom(image.phys_base, image.data.clone());
        Ok(())
    }

    /// Map a legacy BIOS image at top-of-4 GiB and the below-1 MiB alias.
    ///
    /// Uses [`firmware_interface::prepare_bios_rom`]: a 64 KiB image lands at
    /// `0xFFFF_0000` and `0x000F_0000` (same reset-vector region as HELLO ROM).
    /// Does not boot SeaBIOS POST — mapping only.
    ///
    /// Spec: `docs/machine-model-pc-v1.md` ROM alias; SeaBIOS memory map notes
    /// in `docs/sources.md`.
    pub fn load_bios_rom(&mut self, data: &[u8]) -> Result<(), MachineError> {
        let map = prepare_bios_rom(data)?;
        self.mem.clear_roms();
        self.mem.add_rom(map.high.phys_base, map.high.data);
        self.mem.add_rom(map.low.phys_base, map.low.data);
        Ok(())
    }

    /// Construct a machine with a BIOS ROM already dual-mapped.
    ///
    /// Wraps [`Self::new`] + [`Self::load_bios_rom`].
    pub fn with_bios_rom(ram_size: usize, data: &[u8]) -> Result<Self, MachineError> {
        let mut m = Self::new(ram_size);
        m.load_bios_rom(data)?;
        Ok(m)
    }

    /// Apply one i440FX PAM configuration register byte (`0x59`-`0x5F`).
    ///
    /// **This is the host entry point a PCI-side PAM caller drives.** The PMC
    /// configuration registers live in `devices::PciConfig`; after a config
    /// write to host-bridge `00:00.0` offsets `0x59`-`0x5F` the machine layer
    /// forwards the byte here so [`PhysMem`] re-attributes the two regions the
    /// register owns. Returns `false` when `offset` is not a PAM register.
    ///
    /// Spec: Intel 440FX PMC datasheet, Programmable Attribute Map.
    pub fn apply_pam_register(&mut self, offset: u8, value: u8) -> bool {
        self.mem.apply_pam_register(offset, value)
    }

    /// Decoded view of a PAM configuration register (reserved bits read 0).
    pub fn pam_register(&self, offset: u8) -> Option<u8> {
        self.mem.pam_register_value(offset)
    }

    /// Set one PAM region's attributes directly (host / test path).
    pub fn set_pam_attributes(
        &mut self,
        region: usize,
        readable_from: PamRead,
        writable_to: PamWrite,
    ) -> bool {
        self.mem
            .set_region_attributes(region, readable_from, writable_to)
    }

    /// Current attributes of a PAM region.
    pub fn pam_attributes(&self, region: usize) -> Option<PamAttributes> {
        self.mem.region_attributes(region)
    }

    pub fn reset(&mut self) {
        self.cpu = CpuState::reset();
        self.com1 = Serial16550::new(0x3F8);
        self.com2 = Serial16550::new(0x2F8);
        self.debug = DebugConsole::new();
        self.pic.reset();
        self.pit.reset();
        self.cmos.reset();
        self.kbd.reset();
        self.port92.reset();
        self.dma.reset();
        self.vga.reset();
        self.pci.reset();
        self.ide.reset();
        self.ide_secondary.reset();
        self.fdc.reset();
        self.fw_cfg.reset();
        self.post_diag.reset();
        // Spec: Intel 440FX PMC — PAM0-PAM6 reset to 0x00 (read from ROM, no
        // DRAM writes). Shadow contents survive, like DRAM across PCIRST#.
        self.mem.reset_pam();
        // Spec: IBM PC AT — A20 open at reset; follow 8042 / port 0x92 defaults.
        self.mem.set_a20_enabled(self.kbd.a20_enabled());
        self.port92.set_a20_enabled(self.kbd.a20_enabled());
    }
    /// Apply a latched system-reset via [`Self::reset`].
    ///
    /// Sources (OR'd; shared latch pattern):
    /// - 8042 pulse-reset `0xFE` on `0x64` ([`I8042::take_system_reset_request`])
    /// - 8042 output-port write (`0xD1`) with bit0 clear (same kbd latch)
    /// - System Control Port A `0x92` bit0 write-1 ([`Port92::take_system_reset_request`])
    ///
    /// Spec: OSDev I8042 / IBM PC AT + OSDev A20 Line (fast reset). Returns `true` when a
    /// request was taken and reset ran. Called automatically after each
    /// [`Self::step`]. Distinct from keyboard Resend `0xFE` on data port `0x60`.
    pub fn service_8042_pulse_reset(&mut self) -> bool {
        let from_kbd = self.kbd.take_system_reset_request();
        let from_port92 = self.port92.take_system_reset_request();
        if from_kbd || from_port92 {
            self.reset();
            true
        } else {
            false
        }
    }

    /// Borrow the decode view for tests (`step`/`run` keep split borrows of `cpu`).
    #[cfg(test)]
    fn bus_mut(&mut self) -> MachineBus<'_> {
        MachineBus {
            mem: &mut self.mem,
            com1: &mut self.com1,
            com2: &mut self.com2,
            debug: &mut self.debug,
            pic: &mut self.pic,
            pit: &mut self.pit,
            cmos: &mut self.cmos,
            kbd: &mut self.kbd,
            port92: &mut self.port92,
            dma: &mut self.dma,
            vga: &mut self.vga,
            pci: &mut self.pci,
            ide: &mut self.ide,
            ide_secondary: &mut self.ide_secondary,
            fdc: &mut self.fdc,
            fw_cfg: &mut self.fw_cfg,
            post_diag: &mut self.post_diag,
            ports: &mut self.ports,
        }
    }

    /// Sync [`PhysMem`] A20 mask from the 8042 output-port bit1 and mirror to port `0x92`.
    pub fn sync_a20_from_kbd(&mut self) {
        let enabled = self.kbd.a20_enabled();
        self.mem.set_a20_enabled(enabled);
        self.port92.set_a20_enabled(enabled);
    }

    pub fn step(&mut self) -> Result<(), MachineError> {
        // Constructed inline so `cpu` stays independently borrowable from the bus view.
        {
            let mut view = MachineBus {
                mem: &mut self.mem,
                com1: &mut self.com1,
                com2: &mut self.com2,
                debug: &mut self.debug,
                pic: &mut self.pic,
                pit: &mut self.pit,
                cmos: &mut self.cmos,
                kbd: &mut self.kbd,
                port92: &mut self.port92,
                dma: &mut self.dma,
                vga: &mut self.vga,
                pci: &mut self.pci,
                ide: &mut self.ide,
                ide_secondary: &mut self.ide_secondary,
                fdc: &mut self.fdc,
                fw_cfg: &mut self.fw_cfg,
                post_diag: &mut self.post_diag,
                ports: &mut self.ports,
            };
            step(&mut self.cpu, &mut view)?;
        }
        // Spec: OSDev I8042 / A20 Line — system-reset after OUT (CPU not in bus view).
        let _ = self.service_8042_pulse_reset();
        Ok(())
    }

    pub fn run(&mut self, max_steps: u64) -> Result<u64, MachineError> {
        // Per-instruction loop so 8042 pulse-reset can restore the CPU between
        // steps (interpreter `run` cannot observe Machine-level reset).
        let mut n = 0u64;
        while n < max_steps && !self.cpu.halted {
            self.step()?;
            n += 1;
        }
        Ok(n)
    }

    /// Combined guest console (COM1 then debug port bytes are tracked separately).
    pub fn com1_text(&self) -> String {
        self.com1.output().as_str_lossy()
    }

    /// COM2 THR sink (separate from COM1 / debug port).
    pub fn com2_text(&self) -> String {
        self.com2.output().as_str_lossy()
    }

    pub fn debug_text(&self) -> String {
        self.debug.output().as_str_lossy()
    }

    pub fn load_hello_rom(&mut self) -> Result<(), MachineError> {
        let rom = build_hello_rom();
        self.load_rom(&rom)
    }

    /// Advance PIT channel 0 (and ch2 speaker timer) by `clocks` model ticks
    /// and sync ch0 OUT → PIC IRQ0.
    ///
    /// Spec: Intel 8254 ch0 OUT; Intel 8259A edge IR (low→high latches IRR).
    /// Guest wall-clock rate is **not** host-real-time — callers choose the quantum.
    ///
    /// When `tick_ch0` reports a rising OUT edge, IR0 is pulsed (deassert then
    /// assert) so modes 2/3 (OUT already high between periods) still latch IRR.
    // --- PIT→IRQ0 (slice/device-pit-irq0); keep MachineBus edits minimal for 8042 merge ---
    pub fn tick_pit(&mut self, clocks: u64) {
        let rising = self.pit.tick_ch0(clocks);
        let _ = self.pit.tick_ch2(clocks);
        if rising {
            self.pic.set_irq_line(0, false);
            self.pic.set_irq_line(0, true);
        } else {
            self.sync_pit_irq0();
        }
    }

    /// Drive PIC IRQ0 from the current PIT ch0 OUT level (level follow).
    pub fn sync_pit_irq0(&mut self) {
        self.pic.set_irq_line(0, self.pit.out_ch0());
    }

    /// Advance CMOS/RTC by `periods` model quanta and sync IRQF → PIC IRQ8.
    ///
    /// Spec: MC146818 IRQ pin; IBM PC AT → 8259A slave IR0 (ISA IRQ8).
    /// Guest wall-clock rate is **not** host-real-time — callers choose the quantum.
    ///
    /// When `tick` reports a rising IRQ edge, IR8 is pulsed (deassert then assert)
    /// so a still-asserted line after EOI can re-latch IRR on the next period.
    pub fn tick_cmos(&mut self, periods: u64) {
        let rising = self.cmos.tick(periods);
        if rising {
            self.pic.set_irq_line(8, false);
            self.pic.set_irq_line(8, true);
        } else {
            self.sync_cmos_irq8();
        }
    }

    /// One CMOS second update cycle (UIP + BCD seconds) → IRQ8 on rising edge.
    ///
    /// Spec: MC146818 update cycle; IBM PC AT → ISA IRQ8. Independent of
    /// [`Self::tick_cmos`] periodic quantum. Not host-real-time.
    pub fn tick_cmos_second(&mut self) {
        let rising = self.cmos.tick_second();
        if rising {
            self.pic.set_irq_line(8, false);
            self.pic.set_irq_line(8, true);
        } else {
            self.sync_cmos_irq8();
        }
    }

    /// Drive PIC IRQ8 from the current CMOS IRQ pin (level follow).
    pub fn sync_cmos_irq8(&mut self) {
        self.pic.set_irq_line(8, self.cmos.irq_line());
    }

    /// Place a byte in the 8042 output buffer and sync OBF∧INT1 → PIC IRQ1.
    ///
    /// Spec: OSDev I8042 / IBM PC AT — keyboard IRQ1 when output buffer full and
    /// configuration bit0 (INT1) is set. Returns true on IRQ1 rising edge.
    pub fn kbd_place_output(&mut self, value: u8) -> bool {
        let rising = self.kbd.place_output(value);
        if rising {
            self.pic.set_irq_line(1, false);
            self.pic.set_irq_line(1, true);
        } else {
            self.sync_kbd_irq1();
        }
        rising
    }

    /// Inject a keyboard make-code via the 8042 and sync OBF∧INT1 → PIC IRQ1.
    ///
    /// Spec: OSDev I8042 / IBM PC AT — device make-code → output buffer when the
    /// keyboard clock is enabled; IRQ1 when config INT1 is set. Dropped when
    /// clock disabled. Returns true on IRQ1 rising edge.
    pub fn kbd_inject_scancode(&mut self, make_code: u8) -> bool {
        let rising = self.kbd.inject_scancode(make_code);
        if rising {
            self.pic.set_irq_line(1, false);
            self.pic.set_irq_line(1, true);
        } else {
            self.sync_kbd_irq1();
        }
        rising
    }

    /// Drive PIC IRQ1 from the current 8042 IRQ1 line (level follow).
    pub fn sync_kbd_irq1(&mut self) {
        self.pic.set_irq_line(1, self.kbd.irq1_line());
    }

    /// Inject an auxiliary (second PS/2 port) byte and sync AUX OBF∧INT12 → IRQ12.
    ///
    /// Spec: IBM PS/2 keyboard controller — second-port data sets AUX OBF and
    /// raises IRQ12 (8259A slave IR4) when config bit1 is set. Dropped when the
    /// aux clock is disabled (config bit5). Returns true on IRQ12 rising edge.
    /// No mouse device is modeled; the caller supplies the byte.
    pub fn kbd_inject_aux_byte(&mut self, value: u8) -> bool {
        let rising = self.kbd.inject_aux_byte(value);
        if rising {
            self.pic.set_irq_line(12, false);
            self.pic.set_irq_line(12, true);
        } else {
            self.sync_kbd_irq12();
        }
        rising
    }

    /// Drive PIC IRQ12 from the current 8042 IRQ12 line (level follow).
    pub fn sync_kbd_irq12(&mut self) {
        self.pic.set_irq_line(12, self.kbd.irq12_line());
    }

    /// Service a pending fw_cfg DMA operation against [`Self::mem`].
    ///
    /// Spec: QEMU fw_cfg "Guest-side DMA Interface". `MachineBus` already does
    /// this after every fw_cfg port write; this is the host-side entry point
    /// for tests and diagnostics. Returns `None` when nothing is pending.
    pub fn service_fw_cfg_dma(&mut self) -> Option<FwCfgDmaOutcome> {
        let mut view = MachineBus {
            mem: &mut self.mem,
            com1: &mut self.com1,
            com2: &mut self.com2,
            debug: &mut self.debug,
            pic: &mut self.pic,
            pit: &mut self.pit,
            cmos: &mut self.cmos,
            kbd: &mut self.kbd,
            port92: &mut self.port92,
            dma: &mut self.dma,
            vga: &mut self.vga,
            pci: &mut self.pci,
            ide: &mut self.ide,
            ide_secondary: &mut self.ide_secondary,
            fdc: &mut self.fdc,
            fw_cfg: &mut self.fw_cfg,
            post_diag: &mut self.post_diag,
            ports: &mut self.ports,
        };
        view.try_fw_cfg_dma()
    }

    /// Drive PIC IRQ4 from the current COM1 16550 interrupt line (level follow).
    ///
    /// Spec: IBM PC/AT ISA interrupt assignment — COM1 (`0x3F8`) is IRQ4 (master
    /// IR4). The 16550 subset only raises THRE (NS16550A IER bit1 / IIR `010b`);
    /// receive-data-available is never asserted because there is no receive path.
    pub fn sync_com1_irq4(&mut self) {
        self.pic.set_irq_line(4, self.com1.irq_line());
    }

    /// Drive PIC IRQ3 from the current COM2 16550 interrupt line (level follow).
    ///
    /// Spec: IBM PC/AT ISA interrupt assignment — COM2 (`0x2F8`) is IRQ3 (master
    /// IR3). Same THRE-only source as [`Self::sync_com1_irq4`].
    pub fn sync_com2_irq3(&mut self) {
        self.pic.set_irq_line(3, self.com2.irq_line());
    }

    /// Assert/deassert a software PIRQA–PIRQD line and sync through PIRQRC to DualPic.
    ///
    /// Spec: Intel 82371SB — PIRQ# → ISA IRQ selected by PIRQRC[A:D] when bit7
    /// is clear. PCI INTx stub for tests (not a full device interrupt storm).
    /// `pirq` is 0=A … 3=D.
    pub fn assert_pirq(&mut self, pirq: u8, high: bool) {
        self.pci.set_pirq_line(pirq, high);
        self.pci.sync_pirq_to_pic(&mut self.pic);
    }

    /// Re-apply latched PIRQ levels through current PIRQRC routes onto DualPic.
    ///
    /// Spec: Intel 82371SB — call after PIRQRC config writes while a PIRQ is held.
    pub fn sync_pirq_to_pic(&mut self) {
        self.pci.sync_pirq_to_pic(&mut self.pic);
    }

    /// Whether the platform NMI delivery path is unmasked.
    ///
    /// Spec: IBM PC/AT — CMOS index port `0x70` bit7 = 1 disables NMI.
    pub fn nmi_delivery_enabled(&self) -> bool {
        !self.cmos.nmi_masked()
    }

    /// Inject a platform `#NMI` (IVT vector 2) if CMOS NMI mask is clear.
    ///
    /// Returns `true` when latched on the CPU; `false` when dropped because
    /// port `0x70` bit7 masks NMI. Delivery occurs on the next [`Self::step`] /
    /// [`Self::run`] (not gated by `RFLAGS.IF`).
    ///
    /// Spec: IBM PC/AT CMOS NMI disable; Intel SDM Vol. 3 §6.3.3 / §6.7 (`#NMI`).
    /// Stub: no SMRAM/SMI, no post-delivery NMI blocking window.
    pub fn inject_nmi(&mut self) -> bool {
        if !self.nmi_delivery_enabled() {
            return false;
        }
        self.cpu.request_nmi();
        true
    }

    /// Bounded PIIX BMIDE one-PRD Read stub against [`PhysMem`].
    ///
    /// Spec: Intel Programming Interface for Bus Master IDE / 82371SB PRD —
    /// wraps [`PciConfig::start_bm_read`] with PhysMem callbacks. Optional
    /// Machine helper (not auto-wired on ATA commands). **Not** a full ATA
    /// READ DMA engine or multi-PRD walk.
    pub fn bmide_prd_read(
        &mut self,
        device_buf: &[u8],
    ) -> Result<devices::BmidePrdTransfer, devices::BmidePrdError> {
        use std::cell::RefCell;
        let mem = RefCell::new(std::mem::replace(&mut self.mem, PhysMem::new(0)));
        let result = self.pci.start_bm_read(
            device_buf,
            |phys| mem.borrow().read_u8(u64::from(phys)).unwrap_or(0xFF),
            |phys, b| {
                let _ = mem.borrow_mut().write_u8(u64::from(phys), b);
            },
        );
        self.mem = mem.into_inner();
        result
    }

    /// Run [`Dma8237::transfer_block`] for an ISA channel against [`PhysMem`].
    ///
    /// Spec: Intel 8237A + OSDev ISA DMA — 8-bit ch0–3 (`count+1` bytes, phys
    /// `(page << 16) | addr`) or 16-bit ch4–7 (`2*(count+1)` bytes, word addr,
    /// phys `(page << 16) | (addr << 1)`); Single+Inc/Dec+Verify/Read/Write
    /// (+ optional Autoinit). Memory callbacks use [`PhysMem::read_u8`] /
    /// [`PhysMem::write_u8`] (A20 applied).
    ///
    /// Used by [`Self::fdc_dma_read_sector`] / [`Self::fdc_dma_write_sector`] /
    /// MachineBus FDC auto-wire (ISA ch2 Write = device→memory, Read =
    /// memory→device). **Not** DREQ/DACK cycle timing or IDE BM-DMA.
    pub fn dma_transfer(
        &mut self,
        isa_channel: usize,
        io_buf: &mut [u8],
    ) -> Result<usize, DmaTransferError> {
        // `transfer_block` holds read+write closures simultaneously; stage PhysMem
        // in a RefCell so both can capture it without overlapping `&mut` borrows.
        use std::cell::RefCell;
        let mem = RefCell::new(std::mem::replace(&mut self.mem, PhysMem::new(0)));
        let result = self.dma.transfer_block(
            isa_channel,
            io_buf,
            |phys| mem.borrow().read_u8(u64::from(phys)).unwrap_or(0xFF),
            |phys, b| {
                let _ = mem.borrow_mut().write_u8(u64::from(phys), b);
            },
        );
        self.mem = mem.into_inner();
        result
    }

    /// If FDC has a pending READ DATA sector (DOR DMA/IRQ was enabled at latch),
    /// copy it through ISA DMA channel 2 in Write mode (I/O device → memory).
    ///
    /// Spec: Intel 82077AA DMA mode; Intel 8237A Write transfer; OSDev ISA DMA
    /// floppy channel 2. Prefer the MachineBus auto-wire after FDC FIFO writes;
    /// this helper is the shared implementation.
    ///
    /// Returns `None` when nothing is pending; `Some(Ok(n))` / `Some(Err(_))`
    /// from [`Self::dma_transfer`].
    pub fn fdc_dma_read_sector(&mut self) -> Option<Result<usize, DmaTransferError>> {
        if self.fdc.dor & FDC_DOR_DMA_IRQ == 0 {
            return None;
        }
        let mut buf = self.fdc.take_pending_dma_sector()?;
        let expected = buf.len();
        match self.dma_transfer(2, &mut buf) {
            Ok(n) => {
                // Spec: 8237A TC before full FDC latch → documented ST1 EN early-stop.
                if n < expected {
                    let _ = self.fdc.apply_dma_read_tc_early_stop(n);
                }
                Some(Ok(n))
            }
            Err(e) => Some(Err(e)),
        }
    }

    /// If FDC has a pending WRITE DATA DMA arm (DOR DMA/IRQ at command complete),
    /// fill a buffer through ISA DMA channel 2 in Read mode
    /// (memory → I/O device) and commit it into the floppy image.
    ///
    /// Spec: Intel 82077AA §5.1.2 WRITE DATA multi-sector / EOT; Intel 8237A
    /// Read transfer; OSDev ISA DMA floppy channel 2. Buffer length is
    /// [`Fdc82077::pending_dma_write_byte_count`] (MT=0 R..=EOT × 512).
    /// Prefer the MachineBus auto-wire after FDC FIFO writes.
    ///
    /// Returns `None` when nothing is pending; `Some(Ok(n))` / `Some(Err(_))`
    /// from [`Self::dma_transfer`]. On transfer error the pending arm is still
    /// consumed and the image is left unchanged.
    pub fn fdc_dma_write_sector(&mut self) -> Option<Result<usize, DmaTransferError>> {
        if self.fdc.dor & FDC_DOR_DMA_IRQ == 0 {
            return None;
        }
        let len = self.fdc.pending_dma_write_byte_count();
        if len == 0 || !self.fdc.take_pending_dma_write() {
            return None;
        }
        let mut buf = vec![0u8; len];
        match self.dma_transfer(2, &mut buf) {
            Ok(n) => {
                // Prefix only: short TC → partial commit + ST1 EN inside FDC.
                let _ = self.fdc.commit_dma_write_sector(&buf[..n]);
                Some(Ok(n))
            }
            Err(e) => Some(Err(e)),
        }
    }
}

struct MachineBus<'a> {
    mem: &'a mut PhysMem,
    com1: &'a mut Serial16550,
    com2: &'a mut Serial16550,
    debug: &'a mut DebugConsole,
    pic: &'a mut DualPic,
    pit: &'a mut Pit8254,
    cmos: &'a mut CmosRtc,
    kbd: &'a mut I8042,
    port92: &'a mut Port92,
    dma: &'a mut Dma8237,
    vga: &'a mut VgaText,
    pci: &'a mut PciConfig,
    ide: &'a mut IdePrimary,
    ide_secondary: &'a mut IdeSecondary,
    fdc: &'a mut Fdc82077,
    fw_cfg: &'a mut FwCfg,
    post_diag: &'a mut PostCodePort,
    ports: &'a mut PortBus,
}

impl MachineBus<'_> {
    /// Same as [`Machine::dma_transfer`]: `transfer_block` → [`PhysMem`].
    fn dma_transfer(
        &mut self,
        isa_channel: usize,
        io_buf: &mut [u8],
    ) -> Result<usize, DmaTransferError> {
        use std::cell::RefCell;
        let mem = RefCell::new(std::mem::replace(self.mem, PhysMem::new(0)));
        let result = self.dma.transfer_block(
            isa_channel,
            io_buf,
            |phys| mem.borrow().read_u8(u64::from(phys)).unwrap_or(0xFF),
            |phys, b| {
                let _ = mem.borrow_mut().write_u8(u64::from(phys), b);
            },
        );
        *self.mem = mem.into_inner();
        result
    }

    /// Auto-wire: pending FDC READ DATA sector → ISA DMA ch2 Write → PhysMem.
    ///
    /// Spec: Intel 82077AA DMA mode + 8237A Write + OSDev ISA DMA floppy ch2.
    /// Invoked after every FDC `port_write` so a guest FIFO parameter completion
    /// that latches `last_sector` triggers DMA without an extra Machine API call.
    fn try_fdc_dma_ch2_write(&mut self) {
        if self.fdc.dor & FDC_DOR_DMA_IRQ == 0 {
            return;
        }
        let Some(mut buf) = self.fdc.take_pending_dma_sector() else {
            return;
        };
        let expected = buf.len();
        // Guest must have programmed ch2 (page/addr/count/mode Write|Single).
        // Errors (masked/wrong mode) leave PhysMem unchanged; latch already consumed.
        // Spec: 8237A TC before full FDC pending length → ST1 EN early-stop model.
        if let Ok(n) = self.dma_transfer(2, &mut buf) {
            if n < expected {
                let _ = self.fdc.apply_dma_read_tc_early_stop(n);
            }
        }
    }

    /// Auto-wire: pending FDC WRITE DATA → ISA DMA ch2 Read → buffer → image.
    ///
    /// Spec: Intel 82077AA §5.1.2 WRITE DATA multi-sector / EOT + 8237A Read +
    /// OSDev ISA DMA floppy ch2. Invoked after every FDC `port_write` so WRITE
    /// DATA FIFO completion with media + DOR DMA/IRQ (no device pre-latch)
    /// pulls [`Fdc82077::pending_dma_write_byte_count`] bytes from PhysMem
    /// (MT=0 R..=EOT × 512) and commits via [`Fdc82077::commit_dma_write_sector`].
    fn try_fdc_dma_ch2_read(&mut self) {
        if self.fdc.dor & FDC_DOR_DMA_IRQ == 0 {
            return;
        }
        let len = self.fdc.pending_dma_write_byte_count();
        if len == 0 || !self.fdc.take_pending_dma_write() {
            return;
        }
        let mut buf = vec![0u8; len];
        // Guest must have programmed ch2 (page/addr/count/mode Read|Single).
        // Errors leave the image unchanged; pending arm already consumed.
        // Short TC commits a prefix and rewrites ST1 EN / partial ENDaddress.
        if let Ok(n) = self.dma_transfer(2, &mut buf) {
            let _ = self.fdc.commit_dma_write_sector(&buf[..n]);
        }
    }

    /// Service a triggered fw_cfg DMA operation against [`PhysMem`].
    ///
    /// Spec: QEMU fw_cfg "Guest-side DMA Interface". The device supplies the
    /// state machine; the machine supplies guest-physical byte accessors, so
    /// the A20 gate applies to fw_cfg DMA exactly as it does to CPU accesses.
    fn try_fw_cfg_dma(&mut self) -> Option<FwCfgDmaOutcome> {
        if !self.fw_cfg.dma_pending() {
            return None;
        }
        use std::cell::RefCell;
        let mem = RefCell::new(std::mem::replace(self.mem, PhysMem::new(0)));
        let outcome = self.fw_cfg.service_dma(
            |phys| mem.borrow().read_u8(phys).unwrap_or(0xFF),
            |phys, b| {
                let _ = mem.borrow_mut().write_u8(phys, b);
            },
        );
        *self.mem = mem.into_inner();
        outcome
    }

    /// Decode classic PC port ownership. Spec: `docs/machine-model-pc-v1.md`.
    fn port_read(&mut self, port: u16, size: u8) -> u32 {
        if IdePrimary::owns_port(port) {
            return self.ide.port_read(port, size);
        }
        if IdeSecondary::owns_port(port) {
            return self.ide_secondary.port_read(port, size);
        }
        if Fdc82077::owns_port(port) {
            return self.fdc.port_read(port, size);
        }
        if FwCfg::owns_port(port) {
            return self.fw_cfg.port_read(port, size);
        }
        if Dma8237::owns_port(port) {
            return self.dma.port_read(port, size);
        }
        // Spec: Intel 82371SB/AB — BMIDE at BMIBA / ACPI PM at PMBASE / UHCI at BAR0 when Command.IO.
        if self.pci.bmide_owns_port(port)
            || self.pci.acpi_pm_owns_port(port)
            || self.pci.uhci_owns_port(port)
            || PciConfig::owns_port(port)
        {
            return self.pci.port_read(port, size);
        }
        if self.vga.owns_port(port) {
            return self.vga.port_read(port, size);
        }
        match port {
            PIC_MASTER_CMD | PIC_MASTER_DATA | PIC_SLAVE_CMD | PIC_SLAVE_DATA => {
                self.pic.port_read(port, size)
            }
            PIT_CH0_DATA | PIT_CH1_DATA | PIT_CH2_DATA | PIT_CONTROL => {
                self.pit.port_read(port, size)
            }
            // Spec: IBM PC/AT — manufacturing diagnostic port; no read data.
            POST_DIAG_PORT => self.post_diag.port_read(port, size),
            PORT_SYSTEM_CONTROL => u32::from(self.pit.port61_read()),
            PORT_SYSTEM_CONTROL_A => self.port92.port_read(port, size),
            CMOS_INDEX | CMOS_DATA => self.cmos.port_read(port, size),
            I8042_DATA | I8042_STATUS_CMD => self.kbd.port_read(port, size),
            0x2F8..0x300 => self.com2.port_read(port, size),
            0x3F8..0x400 => self.com1.port_read(port, size),
            0x402 => self.debug.port_read(port, size),
            _ => self.ports.port_read(port, size),
        }
    }

    fn port_write(&mut self, port: u16, size: u8, value: u32) {
        if IdePrimary::owns_port(port) {
            self.ide.port_write(port, size, value);
            return;
        }
        if IdeSecondary::owns_port(port) {
            self.ide_secondary.port_write(port, size, value);
            return;
        }
        if Fdc82077::owns_port(port) {
            self.fdc.port_write(port, size, value);
            // Spec: 82077AA DMA mode — after READ DATA FIFO completion latches a
            // sector with DOR DMA/IRQ enable, copy via ISA DMA ch2 Write (I/O→mem).
            self.try_fdc_dma_ch2_write();
            // Spec: 82077AA DMA mode — after WRITE DATA FIFO completion with media
            // + DOR DMA/IRQ, pull via ISA DMA ch2 Read (mem→I/O) into the image.
            self.try_fdc_dma_ch2_read();
            return;
        }
        if FwCfg::owns_port(port) {
            self.fw_cfg.port_write(port, size, value);
            // Spec: QEMU fw_cfg DMA — a write to the low half of the address
            // register at `0x518` triggers the operation immediately.
            self.try_fw_cfg_dma();
            return;
        }
        if Dma8237::owns_port(port) {
            self.dma.port_write(port, size, value);
            return;
        }
        // Spec: Intel 82371SB/AB — BMIDE / ACPI PM / UHCI decode; CF8/CFC + ELCR via owns_port.
        if self.pci.bmide_owns_port(port)
            || self.pci.acpi_pm_owns_port(port)
            || self.pci.uhci_owns_port(port)
            || PciConfig::owns_port(port)
        {
            // Spec: Intel 82371SB — detect PIRQRC CONFIG_DATA overlap before write
            // so the latched Type-1 address still describes the target register.
            let pirqrc_touch = self.pci.pirqrc_config_write_overlaps(port, size);
            self.pci.port_write(port, size, value);
            // Spec: Intel 82371 / OSDev ELCR — 0x4D0/0x4D1 bits select DualPic
            // per-IR level vs edge (SeaBIOS/PIIX); OR'd with ICW1.LTIM in Pic8259.
            if port == PIIX_ELCR_MASTER || port == PIIX_ELCR_SLAVE {
                let [master, slave] = self.pci.elcr;
                self.pic.set_elcr_level_mask(master, slave);
            }
            // Spec: Intel 82371SB — PIRQRC route change while PIRQ held updates PIC.
            if pirqrc_touch {
                self.pci.sync_pirq_to_pic(self.pic);
            }
            return;
        }
        if self.vga.owns_port(port) {
            self.vga.port_write(port, size, value);
            return;
        }
        match port {
            PIC_MASTER_CMD | PIC_MASTER_DATA | PIC_SLAVE_CMD | PIC_SLAVE_DATA => {
                self.pic.port_write(port, size, value);
            }
            PIT_CH0_DATA | PIT_CH1_DATA | PIT_CH2_DATA | PIT_CONTROL => {
                self.pit.port_write(port, size, value);
            }
            // Spec: IBM PC/AT — POST checkpoint latch for host diagnostics.
            POST_DIAG_PORT => self.post_diag.port_write(port, size, value),
            PORT_SYSTEM_CONTROL => self.pit.port61_write(value as u8),
            PORT_SYSTEM_CONTROL_A => {
                self.port92.port_write(port, size, value);
                // Spec: OSDev A20 Line — Fast Gate A20 (bit1) → PhysMem; mirror
                // to 8042 output-port bit1. Bit0 reset latches on `port92` for
                // [`Machine::service_8042_pulse_reset`] after the bus borrow ends.
                let enabled = self.port92.a20_enabled();
                self.mem.set_a20_enabled(enabled);
                self.kbd.set_a20_enabled(enabled);
            }
            CMOS_INDEX | CMOS_DATA => self.cmos.port_write(port, size, value),
            I8042_DATA | I8042_STATUS_CMD => {
                self.kbd.port_write(port, size, value);
                // Spec: IBM PC AT 8042 output port bit1 → A20 gate on phys mem;
                // Spec: IBM PC AT 8042 output port bit1 → A20 gate on phys mem;
                // mirror to System Control Port A (`0x92`) bit1.
                // Pulse-reset `0xFE` on `0x64` and `0xD1` writes with bit0 clear
                // latch on `kbd` for [`Machine::service_8042_pulse_reset`] after
                // the bus borrow ends.
                let enabled = self.kbd.a20_enabled();
                self.mem.set_a20_enabled(enabled);
                self.port92.set_a20_enabled(enabled);
            }
            0x2F8..0x300 => self.com2.port_write(port, size, value),
            0x3F8..0x400 => self.com1.port_write(port, size, value),
            0x402 => self.debug.port_write(port, size, value),
            _ => self.ports.port_write(port, size, value),
        }
    }
}

impl Bus for MachineBus<'_> {
    fn read_u8(&mut self, addr: u64) -> Result<u8, ExecError> {
        // Spec: IBM VGA text — 0xB8000 plane overlays RAM on the CPU bus.
        // A20 mask matches PhysMem before VGA vs RAM decode.
        let effective = if self.mem.a20_enabled() {
            addr
        } else {
            addr & !(1u64 << 20)
        };
        if let Some(b) = self.vga.read_u8(effective) {
            return Ok(b);
        }
        // Probe-only: anything decoding to neither RAM nor ROM is open bus.
        if self.ports.probe_enabled() && !self.mem.is_mapped(addr) {
            self.ports.record_unmapped_mmio(effective, false);
        }
        self.mem
            .read_u8(addr)
            .map_err(|_| ExecError::MemoryFault(addr))
    }

    fn write_u8(&mut self, addr: u64, val: u8) -> Result<(), ExecError> {
        let effective = if self.mem.a20_enabled() {
            addr
        } else {
            addr & !(1u64 << 20)
        };
        if self.vga.write_u8(effective, val) {
            return Ok(());
        }
        if self.ports.probe_enabled() && !self.mem.is_mapped(addr) {
            self.ports.record_unmapped_mmio(effective, true);
        }
        self.mem
            .write_u8(addr, val)
            .map_err(|_| ExecError::MemoryFault(addr))
    }

    fn port_in_u8(&mut self, port: u16) -> Result<u8, ExecError> {
        Ok(self.port_read(port, 1) as u8)
    }

    fn port_out_u8(&mut self, port: u16, val: u8) -> Result<(), ExecError> {
        self.port_write(port, 1, u32::from(val));
        Ok(())
    }

    fn port_in_u16(&mut self, port: u16) -> Result<u16, ExecError> {
        // Spec: Intel SDM Vol. 2 INS/OUTS/IN/OUT — I/O address in DX, size = operand size.
        Ok(self.port_read(port, 2) as u16)
    }

    fn port_out_u16(&mut self, port: u16, val: u16) -> Result<(), ExecError> {
        self.port_write(port, 2, u32::from(val));
        Ok(())
    }

    fn port_in_u32(&mut self, port: u16) -> Result<u32, ExecError> {
        Ok(self.port_read(port, 4))
    }

    fn port_out_u32(&mut self, port: u16, val: u32) -> Result<(), ExecError> {
        self.port_write(port, 4, val);
        Ok(())
    }

    /// Spec: Intel 8259A INTA vectoring; SDM Vol. 3 §6.8.1 maskable interrupts.
    ///
    /// Syncs PIT ch0 OUT → IRQ0, 8042 OBF∧INT1 → IRQ1, COM2 THRE → IRQ3, COM1
    /// THRE → IRQ4, FDC IRQ6, CMOS IRQF → IRQ8, 8042 AUX OBF∧INT12 → IRQ12,
    /// primary IDE INTRQ∧¬nIEN → IRQ14, and secondary IDE → IRQ15 (level follow)
    /// before acknowledge so edges from prior
    /// [`Machine::tick_pit`] / [`Machine::kbd_place_output`] /
    /// [`Machine::kbd_inject_aux_byte`] / [`Machine::tick_cmos`] / FDC
    /// [`Fdc82077::assert_irq6`] / IDE completion / 16550 THR drain are visible.
    ///
    /// Spec: IBM PC/AT ISA interrupt assignment — COM1 `0x3F8` → IRQ4, COM2
    /// `0x2F8` → IRQ3. Only the NS16550A THRE source exists in this UART subset.
    fn poll_external_irq(&mut self) -> Option<u8> {
        self.pic.set_irq_line(0, self.pit.out_ch0());
        self.pic.set_irq_line(1, self.kbd.irq1_line());
        self.pic.set_irq_line(3, self.com2.irq_line());
        self.pic.set_irq_line(4, self.com1.irq_line());
        self.pic.set_irq_line(6, self.fdc.irq_line());
        self.pic.set_irq_line(8, self.cmos.irq_line());
        self.pic.set_irq_line(12, self.kbd.irq12_line());
        self.pic.set_irq_line(14, self.ide.irq_line());
        self.pic.set_irq_line(15, self.ide_secondary.irq_line());
        self.pic.poll_irq()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use devices::{
        CmosRtc, DualPic, Fdc82077, PciConfig, Pit8254, Port92, CFG_INT1, CFG_INT12, CFG_TRANSLATE,
        CMD_ENABLE_KBD, CMD_PULSE_RESET, CMD_READ_CONFIG, CMD_SELF_TEST, CMD_WRITE_CONFIG,
        CMD_WRITE_OUTPUT_PORT, CMOS_DATA, CMOS_INDEX, FDC_1440_IMAGE_SIZE, FDC_CMD_CONFIGURE,
        FDC_CMD_MFM, FDC_CMD_READ_DATA, FDC_CMD_RECALIBRATE, FDC_CMD_SEEK,
        FDC_CMD_SENSE_DRIVE_STATUS, FDC_CMD_SENSE_INT, FDC_CMD_SPECIFY, FDC_CMD_WRITE_DATA,
        FDC_DOR, FDC_DOR_DMA_IRQ, FDC_DOR_RESET_N, FDC_FIFO, FDC_MSR, FDC_MSR_DIO, FDC_MSR_RQM,
        FDC_SECTOR_SIZE, FDC_ST0_IC_ABNORMAL, FDC_ST0_SEEK_END, FDC_ST1_EN, FDC_ST3_RESERVED_BIT3,
        FDC_ST3_RESERVED_BIT5, FDC_ST3_TRACK0, FW_CFG_DATA, FW_CFG_DMA_ADDR_HIGH,
        FW_CFG_DMA_ADDR_LOW, FW_CFG_DMA_CTL_ERROR, FW_CFG_DMA_CTL_READ, FW_CFG_DMA_CTL_SELECT,
        FW_CFG_DMA_CTL_WRITE, FW_CFG_DMA_SIGNATURE, FW_CFG_ID, FW_CFG_RAM_SIZE, FW_CFG_SELECTOR,
        FW_CFG_SIGNATURE, FW_CFG_SIGNATURE_BYTES, FW_CFG_VERSION, FW_CFG_VERSION_DMA, I8042,
        I8042_DATA, I8042_STATUS_CMD, PCI_CONFIG_ADDRESS, PCI_CONFIG_DATA,
        PCI_PIIX_ISA_PIRQRC_OFFSET, PIC_MASTER_CMD, PIC_MASTER_DATA, PIC_SLAVE_CMD, PIC_SLAVE_DATA,
        PIIX_ELCR_MASTER, PIIX_ELCR_SLAVE, PIT_CH0_DATA, PIT_CH2_DATA, PIT_CONTROL, PORT61_GATE2,
        PORT61_OUT2, PORT61_SPKR_DATA, PORT92_A20, PORT92_RESET, PORT_SYSTEM_CONTROL,
        PORT_SYSTEM_CONTROL_A, REG_STATUS_A, REG_STATUS_B, REG_STATUS_C, SELF_TEST_OK,
        STATUS_AUX_OBF, STATUS_IBF, STATUS_OBF, STB_PIE, STC_IRQF, STC_PF, VGA_CRTC_DATA,
        VGA_CRTC_INDEX, VGA_DAC_DATA, VGA_DAC_READ_INDEX, VGA_DAC_WRITE_INDEX,
        VGA_MISC_OUTPUT_DEFAULT, VGA_MISC_OUTPUT_READ, VGA_MISC_OUTPUT_WRITE,
    };

    #[test]
    fn hello_rom_prints_on_com1_and_debug() {
        let mut m = Machine::new(16 * 1024 * 1024);
        m.load_hello_rom().unwrap();
        m.reset();
        let steps = m.run(10_000).unwrap();
        assert!(steps > 0);
        assert!(m.cpu.halted, "ROM should HLT");
        assert_eq!(m.com1_text(), EXPECTED_HELLO);
        assert_eq!(m.debug_text(), EXPECTED_HELLO);
    }

    #[test]
    fn reset_fetch_is_rom() {
        let mut m = Machine::new(1024 * 1024);
        m.load_hello_rom().unwrap();
        m.reset();
        let b = m.mem.read_u8(0xFFFF_FFF0).unwrap();
        assert_eq!(b, 0xE9, "near JMP at reset vector");
    }

    /// Tiny synthetic BIOS (not SeaBIOS) — signature + HLT at reset offset.
    fn synthetic_bios_64k() -> Vec<u8> {
        let mut rom = vec![0u8; 64 * 1024];
        rom[0] = 0xEA;
        rom[0xFFF0] = 0xF4; // HLT at Intel reset vector offset
        rom[0xFFFF] = 0x55;
        rom
    }

    /// Spec: SeaBIOS / classic PC — BIOS dual-mapped at top-of-4 GiB and `0xF0000`.
    #[test]
    fn load_bios_rom_maps_high_and_f0000_alias() {
        let mut m = Machine::new(1024 * 1024);
        m.load_bios_rom(&synthetic_bios_64k()).unwrap();

        assert_eq!(m.mem.read_u8(0xFFFF_0000).unwrap(), 0xEA);
        assert_eq!(m.mem.read_u8(0x000F_0000).unwrap(), 0xEA);
        assert_eq!(m.mem.read_u8(0xFFFF_FFF0).unwrap(), 0xF4);
        assert_eq!(m.mem.read_u8(0x000F_FFF0).unwrap(), 0xF4);
        assert_eq!(m.mem.read_u8(0xFFFF_FFFF).unwrap(), 0x55);
        assert_eq!(m.mem.read_u8(0x000F_FFFF).unwrap(), 0x55);
        // Spec: Intel 440FX PMC — the alias sits under PAM, whose reset
        // attributes forward the write to PCI (dropped) rather than faulting.
        assert_eq!(m.mem.write_u8(0x000F_0000, 0x00), Ok(()));
        assert_eq!(m.mem.read_u8(0x000F_0000).unwrap(), 0xEA);
        assert_eq!(
            m.mem.write_u8(0xFFFF_0000, 0x00),
            Err(crate::mem::MemError::RomWrite)
        );
    }

    /// Spec: Intel 440FX PMC — a PCI-side PAM register write re-attributes both
    /// regions of the nibble pair through `Machine::apply_pam_register`, and a
    /// machine reset restores the `0x00` default.
    #[test]
    fn pam_register_write_reattributes_bios_region() {
        let mut m = Machine::with_bios_rom(1024 * 1024, &synthetic_bios_64k()).unwrap();
        assert_eq!(
            m.pam_attributes(PAM_BIOS_REGION),
            Some(PamAttributes {
                read: PamRead::Rom,
                write: PamWrite::Ignored,
            })
        );

        // PAM0 (0x59) high nibble RE|WE → BIOS area reads and writes DRAM.
        assert!(m.apply_pam_register(0x59, (PAM_FIELD_RE | PAM_FIELD_WE) << 4));
        assert_eq!(
            m.pam_attributes(PAM_BIOS_REGION),
            Some(PamAttributes {
                read: PamRead::ShadowRam,
                write: PamWrite::ShadowRam,
            })
        );
        assert_eq!(m.pam_register(0x59), Some(0x30));
        m.mem.write_u8(0x000F_0000, 0x5A).unwrap();
        assert_eq!(m.mem.read_u8(0x000F_0000).unwrap(), 0x5A);

        m.reset();
        assert_eq!(m.pam_register(0x59), Some(0x00));
        assert_eq!(m.mem.read_u8(0x000F_0000).unwrap(), 0xEA);
    }

    /// Spec: `with_bios_rom` mirrors [`Machine::with_floppy`] constructor shape.
    #[test]
    fn with_bios_rom_constructor_maps_alias() {
        let m = Machine::with_bios_rom(1024 * 1024, &synthetic_bios_64k()).unwrap();
        assert_eq!(m.mem.read_u8(0xFFFF_FFF0).unwrap(), 0xF4);
        assert_eq!(m.mem.read_u8(0x000F_FFF0).unwrap(), 0xF4);
    }

    #[test]
    fn load_bios_rom_rejects_empty() {
        let mut m = Machine::new(64 * 1024);
        assert!(matches!(
            m.load_bios_rom(&[]),
            Err(MachineError::BiosRom(BiosRomError::Empty))
        ));
    }

    /// Spec: classic PC PIC ports on MachineBus (Intel 8259A ICW1–ICW4; docs/machine-model-pc-v1.md).
    #[test]
    fn machine_bus_programs_dual_pic_icw() {
        let mut m = Machine::new(64 * 1024);
        {
            let mut bus = m.bus_mut();
            // Cascaded AT init: master 0x11/0x08/0x04/0x01, slave 0x11/0x70/0x02/0x01.
            bus.port_out_u8(PIC_MASTER_CMD, 0x11).unwrap();
            bus.port_out_u8(PIC_MASTER_DATA, 0x08).unwrap();
            bus.port_out_u8(PIC_MASTER_DATA, 0x04).unwrap();
            bus.port_out_u8(PIC_MASTER_DATA, 0x01).unwrap();
            bus.port_out_u8(PIC_SLAVE_CMD, 0x11).unwrap();
            bus.port_out_u8(PIC_SLAVE_DATA, 0x70).unwrap();
            bus.port_out_u8(PIC_SLAVE_DATA, 0x02).unwrap();
            bus.port_out_u8(PIC_SLAVE_DATA, 0x01).unwrap();
            // After init: command port = IRR (0), data port = IMR (all masked).
            assert_eq!(bus.port_in_u8(PIC_MASTER_CMD).unwrap(), 0x00);
            assert_eq!(bus.port_in_u8(PIC_MASTER_DATA).unwrap(), 0xFF);
        }
        assert!(m.pic.master.initialized);
        assert!(m.pic.slave.initialized);
        assert_eq!(m.pic.master.vector_base, 0x08);
        assert_eq!(m.pic.master.slave_ir_mask(), 0x04);
        assert_eq!(m.pic.slave.vector_base, 0x70);
        assert_eq!(m.pic.slave.slave_id(), 2);
        assert!(m.pic.master.mode_8086);
        assert!(m.pic.slave.mode_8086);
    }

    /// Spec: DualPic assert → MachineBus::poll_external_irq returns 8259 vector.
    ///
    /// Drive IRQ0 via PIT ch0 OUT so `poll_external_irq`'s line sync keeps IR0
    /// high through first INTA (Intel 8259A: pin low at INTA → DEFAULT IR7).
    #[test]
    fn machine_bus_poll_external_irq_from_pic() {
        let mut m = Machine::new(64 * 1024);
        {
            let mut bus = m.bus_mut();
            bus.port_out_u8(PIC_MASTER_CMD, 0x11).unwrap();
            bus.port_out_u8(PIC_MASTER_DATA, 0x08).unwrap();
            bus.port_out_u8(PIC_MASTER_DATA, 0x04).unwrap();
            bus.port_out_u8(PIC_MASTER_DATA, 0x01).unwrap();
            bus.port_out_u8(PIC_SLAVE_CMD, 0x11).unwrap();
            bus.port_out_u8(PIC_SLAVE_DATA, 0x70).unwrap();
            bus.port_out_u8(PIC_SLAVE_DATA, 0x02).unwrap();
            bus.port_out_u8(PIC_SLAVE_DATA, 0x01).unwrap();
            bus.port_out_u8(PIC_MASTER_DATA, 0xFE).unwrap(); // unmask IR0
        }
        // Mode 0 count=1 → load CLK + terminal → OUT rises and stays high.
        m.pit.port_write(PIT_CONTROL, 1, 0x30);
        m.pit.port_write(PIT_CH0_DATA, 1, 0x01);
        m.pit.port_write(PIT_CH0_DATA, 1, 0x00);
        m.tick_pit(2);
        assert!(m.pit.out_ch0());
        {
            let mut bus = m.bus_mut();
            assert_eq!(bus.poll_external_irq(), Some(0x08));
            assert_eq!(bus.poll_external_irq(), None);
        }
        assert_eq!(m.pic.master.isr, 0x01);
    }

    /// Helper: classic AT DualPic cascade init + unmask master IR0.
    fn init_at_pic_unmask_irq0(m: &mut Machine) {
        m.pic.port_write(PIC_MASTER_CMD, 1, 0x11);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x08);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x04);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x01);
        m.pic.port_write(PIC_SLAVE_CMD, 1, 0x11);
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0x70);
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0x02);
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0x01);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0xFE); // unmask IR0
    }

    /// Helper: classic AT DualPic cascade + unmask master IR2 (cascade) and slave IR0 (IRQ8).
    fn init_at_pic_unmask_irq8(m: &mut Machine) {
        m.pic.port_write(PIC_MASTER_CMD, 1, 0x11);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x08);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x04);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x01);
        m.pic.port_write(PIC_SLAVE_CMD, 1, 0x11);
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0x70);
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0x02);
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0x01);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0xFB); // unmask IR2 (cascade)
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0xFE); // unmask slave IR0 (IRQ8)
    }

    /// Helper: classic AT DualPic cascade init + unmask master IR1 (IRQ1).
    fn init_at_pic_unmask_irq1(m: &mut Machine) {
        m.pic.port_write(PIC_MASTER_CMD, 1, 0x11);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x08);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x04);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x01);
        m.pic.port_write(PIC_SLAVE_CMD, 1, 0x11);
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0x70);
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0x02);
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0x01);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0xFD); // unmask IR1
    }

    /// Helper: classic AT DualPic cascade + unmask master IR2 and slave IR4 (IRQ12).
    fn init_at_pic_unmask_irq12(m: &mut Machine) {
        m.pic.port_write(PIC_MASTER_CMD, 1, 0x11);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x08);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x04);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x01);
        m.pic.port_write(PIC_SLAVE_CMD, 1, 0x11);
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0x70);
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0x02);
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0x01);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0xFB); // unmask IR2 (cascade)
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0xEF); // unmask slave IR4 (IRQ12)
    }

    /// Helper: classic AT DualPic cascade + unmask master IR2 and slave IR6 (IRQ14).
    fn init_at_pic_unmask_irq14(m: &mut Machine) {
        m.pic.port_write(PIC_MASTER_CMD, 1, 0x11);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x08);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x04);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x01);
        m.pic.port_write(PIC_SLAVE_CMD, 1, 0x11);
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0x70);
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0x02);
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0x01);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0xFB); // unmask IR2 (cascade)
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0xBF); // unmask slave IR6 (IRQ14)
    }

    /// Helper: classic AT DualPic cascade + unmask master IR2 and slave IR7 (IRQ15).
    fn init_at_pic_unmask_irq15(m: &mut Machine) {
        m.pic.port_write(PIC_MASTER_CMD, 1, 0x11);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x08);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x04);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x01);
        m.pic.port_write(PIC_SLAVE_CMD, 1, 0x11);
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0x70);
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0x02);
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0x01);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0xFB); // unmask IR2 (cascade)
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0x7F); // unmask slave IR7 (IRQ15)
    }

    /// Helper: classic AT DualPic cascade + unmask master IR6 (IRQ6 / FDC).
    fn init_at_pic_unmask_irq6(m: &mut Machine) {
        m.pic.port_write(PIC_MASTER_CMD, 1, 0x11);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x08);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x04);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x01);
        m.pic.port_write(PIC_SLAVE_CMD, 1, 0x11);
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0x70);
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0x02);
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0x01);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0xBF); // unmask IR6 (IRQ6)
    }

    /// Helper: classic AT DualPic cascade + unmask master IR4 (IRQ4 / COM1).
    fn init_at_pic_unmask_irq4(m: &mut Machine) {
        m.pic.port_write(PIC_MASTER_CMD, 1, 0x11);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x08);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x04);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x01);
        m.pic.port_write(PIC_SLAVE_CMD, 1, 0x11);
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0x70);
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0x02);
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0x01);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0xEF); // unmask IR4 (IRQ4)
    }

    /// Helper: classic AT DualPic cascade + unmask master IR3 (IRQ3 / COM2).
    fn init_at_pic_unmask_irq3(m: &mut Machine) {
        m.pic.port_write(PIC_MASTER_CMD, 1, 0x11);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x08);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x04);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x01);
        m.pic.port_write(PIC_SLAVE_CMD, 1, 0x11);
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0x70);
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0x02);
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0x01);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0xF7); // unmask IR3 (IRQ3)
    }

    /// Spec: NS16550A IER bit1 (ETBEI) + IIR THRE ID `010b`; IBM PC/AT ISA
    /// interrupt assignment COM1 `0x3F8` → IRQ4 (master IR4, vector `0x0C`).
    /// The 16550 subset has no receive path, so ERBFI/RDA never drives the line.
    #[test]
    fn com1_thre_asserts_irq4_eoi_clears() {
        let mut m = Machine::new(64 * 1024);
        init_at_pic_unmask_irq4(&mut m);
        {
            let mut bus = m.bus_mut();
            // No THRE enable → no IRQ4 even though LSR.THRE is set at reset.
            assert_eq!(bus.poll_external_irq(), None);

            bus.port_out_u8(0x3F9, 0x02).unwrap(); // IER ETBEI
            assert_eq!(bus.poll_external_irq(), Some(0x0C));
            assert_eq!(bus.poll_external_irq(), None);
            bus.port_out_u8(PIC_MASTER_CMD, 0x20).unwrap(); // non-specific EOI
        }
        assert_eq!(m.pic.master.isr, 0);
        assert!(m.com1.irq_line());
    }

    /// Spec: IBM PC/AT ISA interrupt assignment COM2 `0x2F8` → IRQ3 (master IR3,
    /// vector `0x0B`); NS16550A register behavior is base-relative.
    #[test]
    fn com2_thre_asserts_irq3_eoi_clears() {
        let mut m = Machine::new(64 * 1024);
        init_at_pic_unmask_irq3(&mut m);
        {
            let mut bus = m.bus_mut();
            assert_eq!(bus.poll_external_irq(), None);

            bus.port_out_u8(0x2F9, 0x02).unwrap(); // IER ETBEI on COM2
            assert_eq!(bus.poll_external_irq(), Some(0x0B));
            assert_eq!(bus.poll_external_irq(), None);
            bus.port_out_u8(PIC_MASTER_CMD, 0x20).unwrap();
        }
        assert_eq!(m.pic.master.isr, 0);
    }

    /// Spec: NS16550A — reading IIR while THRE is the reported source clears the
    /// interrupt, dropping the ISA IR pin; a later THR write re-arms it. IRQ4 is
    /// edge-triggered (ICW1.LTIM=0, ELCR clear), so redelivery needs that edge.
    #[test]
    fn com1_iir_read_drops_irq4_until_next_thr_write() {
        let mut m = Machine::new(64 * 1024);
        init_at_pic_unmask_irq4(&mut m);
        {
            let mut bus = m.bus_mut();
            bus.port_out_u8(0x3F9, 0x02).unwrap();
            assert_eq!(bus.poll_external_irq(), Some(0x0C));
            bus.port_out_u8(PIC_MASTER_CMD, 0x20).unwrap();

            // Line still high but no new edge → no redelivery.
            assert_eq!(bus.poll_external_irq(), None);

            assert_eq!(bus.port_in_u8(0x3FA).unwrap(), 0x02); // IIR THRE, clears
            assert_eq!(bus.poll_external_irq(), None);

            bus.port_out_u8(0x3F8, u32::from(b'A') as u8).unwrap();
            assert_eq!(bus.poll_external_irq(), Some(0x0C));
            bus.port_out_u8(PIC_MASTER_CMD, 0x20).unwrap();
        }
        assert_eq!(m.com1_text(), "A");
    }

    /// Spec: COM1 and COM2 are independent ISA IR sources (IRQ4 vs IRQ3).
    #[test]
    fn com1_and_com2_irq_lines_are_independent() {
        let mut m = Machine::new(64 * 1024);
        // Unmask both IR3 and IR4 on the master.
        init_at_pic_unmask_irq4(&mut m);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0xE7);
        {
            let mut bus = m.bus_mut();
            bus.port_out_u8(0x2F9, 0x02).unwrap(); // COM2 only
            assert_eq!(bus.poll_external_irq(), Some(0x0B));
            bus.port_out_u8(PIC_MASTER_CMD, 0x20).unwrap();
            assert_eq!(bus.poll_external_irq(), None);
        }
        assert!(!m.com1.irq_line());
        assert!(m.com2.irq_line());
    }

    /// Host-side sync helpers mirror the [`MachineBus::poll_external_irq`] wiring.
    #[test]
    fn sync_com_irq_helpers_follow_device_lines() {
        let mut m = Machine::new(64 * 1024);
        init_at_pic_unmask_irq4(&mut m);
        m.com1.port_write(0x3F9, 1, 0x02);
        m.com2.port_write(0x2F9, 1, 0x02);
        m.sync_com1_irq4();
        m.sync_com2_irq3();
        assert_eq!(m.pic.master.irr & 0x18, 0x18);
    }

    /// Spec: SDM Vol. 3 §6.8.1 + Intel 8259A vector = ICW2 base | IR — a THRE
    /// interrupt with IF=1 vectors through real-mode IVT entry `0x0C`.
    #[test]
    fn guest_sti_delivers_com1_irq4_via_ivt() {
        let mut m = Machine::new(64 * 1024);
        // IVT[0x0C] → 0000:0E00; handler HLT.
        m.mem.write_u8(0x0C * 4, 0x00).unwrap();
        m.mem.write_u8(0x0C * 4 + 1, 0x0E).unwrap();
        m.mem.write_u8(0x0C * 4 + 2, 0x00).unwrap();
        m.mem.write_u8(0x0C * 4 + 3, 0x00).unwrap();
        m.mem.write_u8(0x0E00, 0xF4).unwrap();
        m.mem.write_u8(0, 0x90).unwrap(); // NOP
        m.mem.write_u8(1, 0xF4).unwrap(); // HLT
        init_at_pic_unmask_irq4(&mut m);
        m.com1.port_write(0x3F9, 1, 0x02); // IER ETBEI → THRE interrupt pending

        m.cpu = CpuState::reset();
        m.cpu.cs = x86_core::SegmentReg::real_mode_code(0x0000);
        m.cpu.ss = x86_core::SegmentReg::real_mode(0x0000);
        m.cpu.set_ip16(0);
        m.cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        m.cpu.halted = false;
        m.cpu.set_interrupt_flag(true);

        m.step().unwrap();

        assert_eq!(m.cpu.ip16(), 0x0E00);
        assert!(!m.cpu.interrupt_flag());
        assert_eq!(m.pic.master.isr, 0x10);
    }

    /// Write a big-endian `FWCfgDmaAccess { control, length, address }` into RAM.
    fn put_fw_cfg_dma_access(m: &mut Machine, at: u64, control: u32, length: u32, address: u64) {
        let mut bytes = Vec::with_capacity(16);
        bytes.extend_from_slice(&control.to_be_bytes());
        bytes.extend_from_slice(&length.to_be_bytes());
        bytes.extend_from_slice(&address.to_be_bytes());
        for (i, b) in bytes.into_iter().enumerate() {
            m.mem.write_u8(at + i as u64, b).unwrap();
        }
    }

    /// Spec: QEMU fw_cfg "Guest-side DMA Interface" — writing the low half of
    /// the big-endian address register at `0x518` triggers the operation
    /// described by the `FWCfgDmaAccess` structure in guest RAM.
    #[test]
    fn machine_bus_fw_cfg_dma_read_copies_item_into_phys_mem() {
        let mut m = Machine::new(64 * 1024);
        put_fw_cfg_dma_access(
            &mut m,
            0x1000,
            FW_CFG_DMA_CTL_SELECT | FW_CFG_DMA_CTL_READ | (u32::from(FW_CFG_SIGNATURE) << 16),
            4,
            0x2000,
        );
        {
            let mut bus = m.bus_mut();
            bus.port_out_u32(FW_CFG_DMA_ADDR_HIGH, 0).unwrap();
            bus.port_out_u32(FW_CFG_DMA_ADDR_LOW, 0x1000u32.swap_bytes())
                .unwrap();
        }

        let copied: Vec<u8> = (0..4).map(|i| m.mem.read_u8(0x2000 + i).unwrap()).collect();
        assert_eq!(copied, FW_CFG_SIGNATURE_BYTES);
        // Control writeback: all bits clear on success.
        let control: Vec<u8> = (0..4).map(|i| m.mem.read_u8(0x1000 + i).unwrap()).collect();
        assert_eq!(control, [0, 0, 0, 0]);
        assert!(!m.fw_cfg.dma_pending());
        assert_eq!(m.fw_cfg.dma_address(), 0);
    }

    /// The RAM-size item the machine configures is reachable over DMA too.
    #[test]
    fn machine_bus_fw_cfg_dma_reads_configured_ram_size() {
        let ram_size = 64 * 1024usize;
        let mut m = Machine::new(ram_size);
        put_fw_cfg_dma_access(
            &mut m,
            0x1000,
            FW_CFG_DMA_CTL_SELECT | FW_CFG_DMA_CTL_READ | (u32::from(FW_CFG_RAM_SIZE) << 16),
            8,
            0x2000,
        );
        {
            let mut bus = m.bus_mut();
            bus.port_out_u32(FW_CFG_DMA_ADDR_HIGH, 0).unwrap();
            bus.port_out_u32(FW_CFG_DMA_ADDR_LOW, 0x1000u32.swap_bytes())
                .unwrap();
        }

        let copied: Vec<u8> = (0..8).map(|i| m.mem.read_u8(0x2000 + i).unwrap()).collect();
        assert_eq!(copied, (ram_size as u64).to_le_bytes());
    }

    /// Spec: reading the DMA address register returns `QEMU CFG` big-endian, and
    /// ID bit1 advertises the interface now that the machine services it.
    #[test]
    fn machine_bus_fw_cfg_dma_signature_and_id_bit() {
        let mut m = Machine::new(64 * 1024);
        let mut bus = m.bus_mut();
        assert_eq!(
            bus.port_in_u32(FW_CFG_DMA_ADDR_HIGH).unwrap(),
            ((FW_CFG_DMA_SIGNATURE >> 32) as u32).swap_bytes()
        );
        bus.port_out_u16(FW_CFG_SELECTOR, FW_CFG_ID).unwrap();
        let id: Vec<u8> = (0..4)
            .map(|_| bus.port_in_u8(FW_CFG_DATA).unwrap())
            .collect();
        assert_eq!(
            u32::from_le_bytes(id.try_into().unwrap()),
            FW_CFG_VERSION | FW_CFG_VERSION_DMA
        );
    }

    /// A DMA write request is refused with the spec error bit (no item
    /// writeability in this tree) and leaves guest RAM and the item alone.
    #[test]
    fn machine_bus_fw_cfg_dma_write_direction_sets_error_bit() {
        let mut m = Machine::new(64 * 1024);
        put_fw_cfg_dma_access(
            &mut m,
            0x1000,
            FW_CFG_DMA_CTL_SELECT | FW_CFG_DMA_CTL_WRITE | (u32::from(FW_CFG_SIGNATURE) << 16),
            4,
            0x2000,
        );
        for i in 0..4 {
            m.mem.write_u8(0x2000 + i, b'Z').unwrap();
        }
        {
            let mut bus = m.bus_mut();
            bus.port_out_u32(FW_CFG_DMA_ADDR_HIGH, 0).unwrap();
            bus.port_out_u32(FW_CFG_DMA_ADDR_LOW, 0x1000u32.swap_bytes())
                .unwrap();
        }

        let control: Vec<u8> = (0..4).map(|i| m.mem.read_u8(0x1000 + i).unwrap()).collect();
        assert_eq!(
            u32::from_be_bytes(control.try_into().unwrap()),
            FW_CFG_DMA_CTL_ERROR
        );
        let mut bus = m.bus_mut();
        bus.port_out_u16(FW_CFG_SELECTOR, FW_CFG_SIGNATURE).unwrap();
        let sig: Vec<u8> = (0..4)
            .map(|_| bus.port_in_u8(FW_CFG_DATA).unwrap())
            .collect();
        assert_eq!(sig, FW_CFG_SIGNATURE_BYTES);
    }

    /// Spec: Intel 8254 mode 0 OUT rising → 8259A IRQ0 → vector 0x08; EOI clears ISR.
    #[test]
    fn pit_mode0_tick_asserts_irq0_eoi_clears() {
        let mut m = Machine::new(64 * 1024);
        init_at_pic_unmask_irq0(&mut m);
        // Program PIT ch0 mode 0, count = 4 (one-CLK CR→CE load + 4 countdown).
        m.pit.port_write(PIT_CONTROL, 1, 0x30);
        m.pit.port_write(PIT_CH0_DATA, 1, 0x04);
        m.pit.port_write(PIT_CH0_DATA, 1, 0x00);
        assert!(!m.pit.out_ch0());

        m.tick_pit(5);
        assert!(m.pit.out_ch0());
        {
            let mut bus = m.bus_mut();
            // Spec: AT master ICW2 base 0x08 → IRQ0 vector 0x08.
            assert_eq!(bus.poll_external_irq(), Some(0x08));
            assert_eq!(bus.poll_external_irq(), None);
            // Non-specific EOI (OCW2 0x20) clears ISR.
            bus.port_out_u8(PIC_MASTER_CMD, 0x20).unwrap();
        }
        assert_eq!(m.pic.master.isr, 0);
    }

    /// Spec: Intel 8254 mode 2 rate-generator OUT edge → IRQ0 → vector 0x08.
    #[test]
    fn pit_mode2_tick_asserts_irq0() {
        let mut m = Machine::new(64 * 1024);
        init_at_pic_unmask_irq0(&mut m);
        m.pit.port_write(PIT_CONTROL, 1, 0x34);
        m.pit.port_write(PIT_CH0_DATA, 1, 0x03);
        m.pit.port_write(PIT_CH0_DATA, 1, 0x00); // count = 3
        m.tick_pit(5); // load + period + low-pulse rise
        {
            let mut bus = m.bus_mut();
            assert_eq!(bus.poll_external_irq(), Some(0x08));
            bus.port_out_u8(PIC_MASTER_CMD, 0x20).unwrap();
        }
        assert_eq!(m.pic.master.isr, 0);
    }

    /// Spec: Intel 8254 mode 3 square-wave OUT edge → IRQ0 → vector 0x08.
    #[test]
    fn pit_mode3_tick_asserts_irq0() {
        let mut m = Machine::new(64 * 1024);
        init_at_pic_unmask_irq0(&mut m);
        m.pit.port_write(PIT_CONTROL, 1, 0x36);
        m.pit.port_write(PIT_CH0_DATA, 1, 0x04);
        m.pit.port_write(PIT_CH0_DATA, 1, 0x00); // count = 4
        m.tick_pit(5);
        {
            let mut bus = m.bus_mut();
            assert_eq!(bus.poll_external_irq(), Some(0x08));
            bus.port_out_u8(PIC_MASTER_CMD, 0x20).unwrap();
        }
        assert_eq!(m.pic.master.isr, 0);
    }

    /// Spec: MC146818 PIE + tick → IRQ8 → 8259A slave vector 0x70; EOI clears ISR.
    #[test]
    fn cmos_pie_tick_asserts_irq8_eoi_clears() {
        let mut m = Machine::new(64 * 1024);
        init_at_pic_unmask_irq8(&mut m);
        m.cmos.port_write(CMOS_INDEX, 1, u32::from(REG_STATUS_B));
        m.cmos.port_write(CMOS_DATA, 1, u32::from(0x02 | STB_PIE)); // 24h + PIE
        assert!(!m.cmos.irq_line());

        m.tick_cmos(1);
        assert!(m.cmos.irq_line());
        {
            let mut bus = m.bus_mut();
            // Spec: AT slave ICW2 base 0x70 → IRQ8 vector 0x70.
            assert_eq!(bus.poll_external_irq(), Some(0x70));
            assert_eq!(bus.poll_external_irq(), None);
            // Status C read-to-clear deasserts CMOS IRQ pin.
            bus.port_out_u8(CMOS_INDEX, REG_STATUS_C).unwrap();
            let stc = bus.port_in_u8(CMOS_DATA).unwrap();
            assert_ne!(stc & STC_PF, 0);
            assert_ne!(stc & STC_IRQF, 0);
            assert!(!bus.cmos.irq_line());
            // Non-specific EOI slave then master (PC AT cascade convention).
            bus.port_out_u8(PIC_SLAVE_CMD, 0x20).unwrap();
            bus.port_out_u8(PIC_MASTER_CMD, 0x20).unwrap();
        }
        assert_eq!(m.pic.slave.isr, 0);
        assert_eq!(m.pic.master.isr, 0);
    }

    /// Spec: without PIE, tick does not deliver IRQ8 via poll_external_irq.
    #[test]
    fn cmos_tick_without_pie_no_irq8() {
        let mut m = Machine::new(64 * 1024);
        init_at_pic_unmask_irq8(&mut m);
        m.tick_cmos(1);
        assert!(!m.cmos.irq_line());
        {
            let mut bus = m.bus_mut();
            assert_eq!(bus.poll_external_irq(), None);
        }
    }

    /// Spec: 8042 OBF + INT1 → IRQ1 → vector 0x09; EOI + read 0x60 clears path.
    #[test]
    fn kbd_obf_irq_enable_asserts_irq1_eoi_clears() {
        let mut m = Machine::new(64 * 1024);
        init_at_pic_unmask_irq1(&mut m);
        // Enable keyboard IRQ in 8042 config (bit0).
        m.kbd
            .port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_CONFIG));
        m.kbd.port_write(I8042_DATA, 1, u32::from(CFG_INT1));
        assert!(m.kbd_place_output(0x1C));
        assert!(m.kbd.irq1_line());
        {
            let mut bus = m.bus_mut();
            // Spec: AT master ICW2 base 0x08 → IRQ1 vector 0x09.
            assert_eq!(bus.poll_external_irq(), Some(0x09));
            assert_eq!(bus.poll_external_irq(), None);
            assert_eq!(bus.port_in_u8(I8042_DATA).unwrap(), 0x1C);
            assert!(!bus.kbd.irq1_line());
            bus.port_out_u8(PIC_MASTER_CMD, 0x20).unwrap(); // EOI
        }
        assert_eq!(m.pic.master.isr, 0);
        assert!(!m.kbd.irq1_line());
    }

    /// Spec: OSDev I8042 — make-code inject → OBF → IRQ1 → vector 0x09; EOI + read clears.
    #[test]
    fn kbd_inject_scancode_asserts_irq1_eoi_clears() {
        let mut m = Machine::new(64 * 1024);
        init_at_pic_unmask_irq1(&mut m);
        m.kbd
            .port_write(I8042_STATUS_CMD, 1, u32::from(CMD_ENABLE_KBD));
        m.kbd
            .port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_CONFIG));
        // Clock enabled, INT1 + translate (Set 2 → Set 1 on keyboard OBF).
        m.kbd
            .port_write(I8042_DATA, 1, u32::from(CFG_INT1 | CFG_TRANSLATE));
        assert!(m.kbd_inject_scancode(0x1C)); // Set 2 'A'
        assert!(m.kbd.irq1_line());
        {
            let mut bus = m.bus_mut();
            // Spec: AT master ICW2 base 0x08 → IRQ1 vector 0x09.
            assert_eq!(bus.poll_external_irq(), Some(0x09));
            assert_eq!(bus.poll_external_irq(), None);
            assert_eq!(bus.port_in_u8(I8042_DATA).unwrap(), 0x1E); // Set 1 'A'
            assert!(!bus.kbd.irq1_line());
            bus.port_out_u8(PIC_MASTER_CMD, 0x20).unwrap(); // EOI
        }
        assert_eq!(m.pic.master.isr, 0);
        assert!(!m.kbd.irq1_line());
    }

    /// Spec: keyboard clock disabled drops inject; no OBF / IRQ1.
    #[test]
    fn kbd_inject_scancode_dropped_when_clock_disabled() {
        let mut m = Machine::new(64 * 1024);
        init_at_pic_unmask_irq1(&mut m);
        // Default reset: clock disabled.
        assert!(m.kbd.keyboard_clock_disabled());
        assert!(!m.kbd_inject_scancode(0x1C));
        assert!(!m.kbd.irq1_line());
        assert_eq!(m.kbd.status() & STATUS_OBF, 0);
        {
            let mut bus = m.bus_mut();
            assert_eq!(bus.poll_external_irq(), None);
        }
    }

    /// Spec: OBF without config INT1 does not deliver IRQ1.
    #[test]
    fn kbd_obf_without_irq_enable_no_irq1() {
        let mut m = Machine::new(64 * 1024);
        init_at_pic_unmask_irq1(&mut m);
        // Default config: INT1 clear.
        assert!(!m.kbd_place_output(0xAA));
        assert!(!m.kbd.irq1_line());
        {
            let mut bus = m.bus_mut();
            assert_eq!(bus.poll_external_irq(), None);
        }
    }

    /// Spec: IBM PS/2 KBC — AUX OBF + config INT12 → IRQ12 → slave vector 0x74;
    /// EOI + read `0x60` clears the path. Keyboard IRQ1 stays idle.
    #[test]
    fn kbd_aux_byte_with_int12_asserts_irq12_eoi_clears() {
        let mut m = Machine::new(64 * 1024);
        init_at_pic_unmask_irq12(&mut m);
        // Enable second-port interrupt in the 8042 config (bit1).
        m.kbd
            .port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_CONFIG));
        m.kbd.port_write(I8042_DATA, 1, u32::from(CFG_INT12));
        assert!(m.kbd_inject_aux_byte(0x08));
        assert!(m.kbd.irq12_line());
        assert!(!m.kbd.irq1_line());
        assert_ne!(m.kbd.status() & STATUS_AUX_OBF, 0);
        {
            let mut bus = m.bus_mut();
            // Spec: AT slave ICW2 base 0x70 → IRQ12 (slave IR4) vector 0x74.
            assert_eq!(bus.poll_external_irq(), Some(0x74));
            assert_eq!(bus.poll_external_irq(), None);
            assert_eq!(bus.port_in_u8(I8042_DATA).unwrap(), 0x08);
            assert!(!bus.kbd.irq12_line());
            // Non-specific EOI slave then master (PC AT cascade convention).
            bus.port_out_u8(PIC_SLAVE_CMD, 0x20).unwrap();
            bus.port_out_u8(PIC_MASTER_CMD, 0x20).unwrap();
        }
        assert_eq!(m.pic.slave.isr, 0);
        assert_eq!(m.pic.master.isr, 0);
        assert!(!m.kbd.irq12_line());
    }

    /// Spec: IBM PS/2 KBC — AUX OBF without config INT12 does not deliver IRQ12.
    #[test]
    fn kbd_aux_byte_without_int12_no_irq12() {
        let mut m = Machine::new(64 * 1024);
        init_at_pic_unmask_irq12(&mut m);
        // Default config: INT12 clear (aux clock enabled).
        assert!(!m.kbd_inject_aux_byte(0x08));
        assert_ne!(m.kbd.status() & STATUS_AUX_OBF, 0);
        assert!(!m.kbd.irq12_line());
        {
            let mut bus = m.bus_mut();
            assert_eq!(bus.poll_external_irq(), None);
        }
    }

    /// Spec: IBM PS/2 KBC — keyboard data raises IRQ1 only, never IRQ12, even
    /// with both interrupt enables set.
    #[test]
    fn kbd_scancode_does_not_deliver_irq12() {
        let mut m = Machine::new(64 * 1024);
        init_at_pic_unmask_irq12(&mut m);
        m.kbd
            .port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_CONFIG));
        // Both enables set; both clocks enabled (config bits 4/5 clear).
        m.kbd
            .port_write(I8042_DATA, 1, u32::from(CFG_INT1 | CFG_INT12));
        assert!(m.kbd_inject_scancode(0x1C));
        assert!(m.kbd.irq1_line());
        assert!(!m.kbd.irq12_line());
        assert_eq!(m.kbd.status() & STATUS_AUX_OBF, 0);
        {
            let mut bus = m.bus_mut();
            // Slave IR4 idle: only master IR1 is unmasked-and-pending here, and
            // this helper masks IR1, so no vector is delivered.
            assert_eq!(bus.poll_external_irq(), None);
        }
    }

    /// Guest STI + PIC IRQ0 → IVT delivery via poll_external_irq.
    /// Spec: SDM Vol. 3 §6.8.1; Intel 8259A vector = ICW2 base | IR.
    ///
    /// Raise IRQ0 with PIT ch0 OUT held high so MachineBus line sync does not
    /// drop the pin before first INTA (DEFAULT IR7).
    #[test]
    fn guest_sti_delivers_pic_irq0_via_ivt() {
        let mut m = Machine::new(64 * 1024);
        // IVT[0x08] → 0000:0E00; handler HLT.
        m.mem.write_u8(0x08 * 4, 0x00).unwrap();
        m.mem.write_u8(0x08 * 4 + 1, 0x0E).unwrap();
        m.mem.write_u8(0x08 * 4 + 2, 0x00).unwrap();
        m.mem.write_u8(0x08 * 4 + 3, 0x00).unwrap();
        m.mem.write_u8(0x0E00, 0xF4).unwrap();
        // Program: NOP then HLT; IRQ delivered before NOP when IF=1.
        m.mem.write_u8(0, 0x90).unwrap(); // NOP
        m.mem.write_u8(1, 0xF4).unwrap(); // HLT
        m.cpu = CpuState::reset();
        m.cpu.cs = x86_core::SegmentReg::real_mode_code(0x0000);
        m.cpu.ss = x86_core::SegmentReg::real_mode(0x0000);
        m.cpu.set_ip16(0);
        m.cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        m.cpu.halted = false;
        m.cpu.set_interrupt_flag(true);

        // Init PIC + unmask IRQ0
        m.pic.port_write(PIC_MASTER_CMD, 1, 0x11);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x08);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x04);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x01);
        m.pic.port_write(PIC_SLAVE_CMD, 1, 0x11);
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0x70);
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0x02);
        m.pic.port_write(PIC_SLAVE_DATA, 1, 0x01);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0xFE);
        m.pit.port_write(PIT_CONTROL, 1, 0x30);
        m.pit.port_write(PIT_CH0_DATA, 1, 0x01);
        m.pit.port_write(PIT_CH0_DATA, 1, 0x00);
        m.tick_pit(2); // load + terminal
        assert!(m.pit.out_ch0());

        m.step().unwrap();
        assert_eq!(m.cpu.ip16(), 0x0E00);
        assert!(!m.cpu.interrupt_flag());
        assert_eq!(m.pic.master.isr, 0x01);
    }

    /// Spec: 8254 channel-0 programming via MachineBus ports 0x40/0x43.
    #[test]
    fn machine_bus_programs_pit_channel0() {
        let mut m = Machine::new(64 * 1024);
        {
            let mut bus = m.bus_mut();
            // Mode 3 square wave, lo/hi access: control 0x36, count 0x1000.
            bus.port_out_u8(PIT_CONTROL, 0x36).unwrap();
            bus.port_out_u8(PIT_CH0_DATA, 0x00).unwrap();
            bus.port_out_u8(PIT_CH0_DATA, 0x10).unwrap();
            assert_eq!(bus.port_in_u8(PIT_CH0_DATA).unwrap(), 0x00);
            assert_eq!(bus.port_in_u8(PIT_CH0_DATA).unwrap(), 0x10);
            assert_eq!(bus.port_in_u8(PIT_CONTROL).unwrap(), 0xFF);
        }
        assert_eq!(m.pit.channel0().mode, 3);
        assert!(m.pit.channel0().count_loaded);
        assert_eq!(m.pit.channel0().count, 0x1000);
    }

    /// Spec: MC146818 CMOS index/data via MachineBus 0x70/0x71.
    #[test]
    fn machine_bus_cmos_index_data() {
        let mut m = Machine::new(64 * 1024);
        {
            let mut bus = m.bus_mut();
            assert_eq!(bus.port_in_u8(CMOS_INDEX).unwrap() & 0x7F, 0);
            bus.port_out_u8(CMOS_INDEX, 0x80 | 0x10).unwrap(); // NMI disable + index 0x10
            bus.port_out_u8(CMOS_DATA, 0x5A).unwrap();
            bus.port_out_u8(CMOS_INDEX, 0x10).unwrap();
            assert_eq!(bus.port_in_u8(CMOS_DATA).unwrap(), 0x5A);
            bus.port_out_u8(CMOS_INDEX, REG_STATUS_A).unwrap();
            assert_eq!(bus.port_in_u8(CMOS_DATA).unwrap(), 0x26);
        }
        assert!(!m.cmos.nmi_disabled);
        assert_eq!(m.cmos.read_reg(0x10), 0x5A);
    }

    /// Spec: IBM PC/AT — port 0x70 bit7 masks NMI; Machine exposes delivery stub.
    #[test]
    fn machine_cmos_nmi_mask_gates_delivery_stub() {
        let mut m = Machine::new(64 * 1024);
        assert!(m.nmi_delivery_enabled());
        {
            let mut bus = m.bus_mut();
            bus.port_out_u8(CMOS_INDEX, 0x80 | 0x0B).unwrap();
            assert_eq!(bus.port_in_u8(CMOS_INDEX).unwrap(), 0x80 | 0x0B);
        }
        assert!(m.cmos.nmi_masked());
        assert!(!m.nmi_delivery_enabled());
        {
            let mut bus = m.bus_mut();
            bus.port_out_u8(CMOS_INDEX, 0x0B).unwrap();
            assert_eq!(bus.port_in_u8(CMOS_INDEX).unwrap(), 0x0B);
        }
        assert!(!m.cmos.nmi_masked());
        assert!(m.nmi_delivery_enabled());
    }

    /// Spec: SDM Vol. 3 §6.3.3 / §6.7 — `#NMI` → IVT[2]; CMOS bit7 clear allows inject.
    #[test]
    fn inject_nmi_delivers_vector_2_via_ivt() {
        let mut m = Machine::new(64 * 1024);
        // IVT[2] at phys 8: handler 0000:0x0800
        m.mem.write_u8(8, 0x00).unwrap();
        m.mem.write_u8(9, 0x08).unwrap();
        m.mem.write_u8(10, 0x00).unwrap();
        m.mem.write_u8(11, 0x00).unwrap();
        m.mem.write_u8(0x800, 0xF4).unwrap(); // HLT
        m.cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        m.cpu.ss = x86_core::SegmentReg::real_mode(0);
        m.cpu.rip = 0x1000;
        m.cpu.set_gpr_u16(x86_core::CpuState::RSP, 0xFFFE);
        m.cpu.set_interrupt_flag(false); // NMI must ignore IF
        assert!(m.inject_nmi());
        assert!(m.cpu.pending_nmi);
        m.step().unwrap();
        assert!(!m.cpu.pending_nmi);
        assert_eq!(m.cpu.cs.selector, 0);
        assert_eq!(m.cpu.ip16(), 0x0800);
        assert!(!m.cpu.interrupt_flag());
    }

    /// Spec: IBM PC/AT — CMOS `0x70` bit7 = 1 drops platform NMI (CPU unchanged).
    #[test]
    fn inject_nmi_dropped_when_cmos_masked() {
        let mut m = Machine::new(64 * 1024);
        m.cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        m.cpu.ss = x86_core::SegmentReg::real_mode(0);
        m.cpu.rip = 0x1234;
        m.cpu.set_gpr_u16(x86_core::CpuState::RSP, 0xFFFE);
        let rip_before = m.cpu.rip;
        let rsp_before = m.cpu.gpr_u16(x86_core::CpuState::RSP);
        {
            let mut bus = m.bus_mut();
            bus.port_out_u8(CMOS_INDEX, 0x80).unwrap();
        }
        assert!(!m.nmi_delivery_enabled());
        assert!(!m.inject_nmi());
        assert!(!m.cpu.pending_nmi);
        assert_eq!(m.cpu.rip, rip_before);
        assert_eq!(m.cpu.gpr_u16(x86_core::CpuState::RSP), rsp_before);
        assert_eq!(m.cpu.cs.selector, 0);
    }

    /// Guest OUT/IN through interpreter → MachineBus programs PIC, PIT, CMOS.
    #[test]
    fn guest_out_in_programs_pic_pit_cmos() {
        let mut m = Machine::new(64 * 1024);
        // Real-mode program at 0000:0000 — Spec: SDM Vol. 2 OUT/IN imm8 forms.
        let prog: &[u8] = &[
            // PIC master cascade ICW: 0x11, 0x20, 0x04, 0x01
            0xB0, 0x11, // mov al, 0x11
            0xE6, 0x20, // out 0x20, al
            0xB0, 0x20, // mov al, 0x20
            0xE6, 0x21, // out 0x21, al
            0xB0, 0x04, // mov al, 0x04
            0xE6, 0x21, // out 0x21, al
            0xB0, 0x01, // mov al, 0x01
            0xE6, 0x21, // out 0x21, al
            // PIT ch0 mode3 count 0x0040
            0xB0, 0x36, // mov al, 0x36
            0xE6, 0x43, // out 0x43, al
            0xB0, 0x40, // mov al, 0x40
            0xE6, 0x40, // out 0x40, al
            0xB0, 0x00, // mov al, 0x00
            0xE6, 0x40, // out 0x40, al
            // CMOS write reg 0x14 = 0xA5
            0xB0, 0x14, // mov al, 0x14
            0xE6, 0x70, // out 0x70, al
            0xB0, 0xA5, // mov al, 0xA5
            0xE6, 0x71, // out 0x71, al
            // CMOS read back into AL
            0xB0, 0x14, // mov al, 0x14
            0xE6, 0x70, // out 0x70, al
            0xE4, 0x71, // in al, 0x71
            0xF4, // hlt
        ];
        for (i, b) in prog.iter().enumerate() {
            m.mem.write_u8(i as u64, *b).unwrap();
        }
        m.cpu = CpuState::reset();
        m.cpu.cs = x86_core::SegmentReg::real_mode_code(0x0000);
        m.cpu.set_ip16(0);
        m.cpu.halted = false;
        let steps = m.run(100).unwrap();
        assert!(steps > 0);
        assert!(m.cpu.halted);
        assert!(m.pic.master.initialized);
        assert_eq!(m.pic.master.vector_base, 0x20);
        assert_eq!(m.pic.master.slave_ir_mask(), 0x04);
        assert_eq!(m.pit.channel0().mode, 3);
        assert_eq!(m.pit.channel0().count, 0x0040);
        assert_eq!(m.cmos.read_reg(0x14), 0xA5);
        assert_eq!(m.cpu.al(), 0xA5);
    }

    /// Unrelated ports stay open-bus; COM1 / COM2 / debug port 0x402 unchanged.
    /// Spec: 0x60 is owned by I8042 — empty-buffer read returns 0, status OBF/IBF clear.
    #[test]
    fn unrelated_ports_open_bus_serial_unchanged() {
        let mut m = Machine::new(64 * 1024);
        {
            let mut bus = m.bus_mut();
            // 8042 empty output buffer: data read 0; status has no OBF/IBF.
            assert_eq!(bus.port_in_u8(I8042_DATA).unwrap(), 0);
            assert_eq!(
                bus.port_in_u8(I8042_STATUS_CMD).unwrap() & (STATUS_OBF | STATUS_IBF),
                0
            );
            // POST diagnostic port: write-only latch, reads stay open bus.
            assert_eq!(bus.port_in_u8(0x80).unwrap(), 0xFF);
            bus.port_out_u8(0x80, 0xAA).unwrap();
            bus.port_out_u8(0x3F8, b'Z').unwrap();
            bus.port_out_u8(0x2F8, b'Y').unwrap();
            bus.port_out_u8(0x402, b'!').unwrap();
            // LSR THR empty bit still present on COM1 / COM2
            assert_ne!(bus.port_in_u8(0x3FD).unwrap() & 0x20, 0);
            assert_ne!(bus.port_in_u8(0x2FD).unwrap() & 0x20, 0);
        }
        assert_eq!(m.com1_text(), "Z");
        assert_eq!(m.com2_text(), "Y");
        assert_eq!(m.debug_text(), "!");
        assert_eq!(m.post_diag.last_code(), Some(0xAA));
        assert!(!m.pic.master.initialized);
        assert!(!m.pit.channel0().count_loaded);
    }

    /// Spec: IBM PC/AT Technical Reference — port `0x80` is the manufacturing
    /// diagnostic (POST checkpoint) port. Writes latch a code for a POST card;
    /// the system board defines no read data, so reads stay ISA open bus.
    #[test]
    fn machine_bus_post_code_port_80_captures_history() {
        let mut m = Machine::new(64 * 1024);
        {
            let mut bus = m.bus_mut();
            for code in [0x01u8, 0x0D, 0x2C] {
                bus.port_out_u8(POST_DIAG_PORT, code).unwrap();
            }
            assert_eq!(bus.port_in_u8(POST_DIAG_PORT).unwrap(), 0xFF);
            // Wider accesses latch the low byte only (byte-wide port).
            bus.port_out_u16(POST_DIAG_PORT, 0xBEEF).unwrap();
        }
        assert_eq!(m.post_diag.history(), [0x01, 0x0D, 0x2C, 0xEF]);
        assert_eq!(m.post_diag.last_code(), Some(0xEF));
        assert_eq!(m.post_diag.write_count(), 4);
        assert!(!m.post_diag.history_overflow());
    }

    /// The bounded history flags overflow instead of growing without limit.
    #[test]
    fn post_code_history_is_bounded_and_reset_clears_it() {
        let mut m = Machine::new(64 * 1024);
        {
            let mut bus = m.bus_mut();
            for i in 0..(POST_CODE_HISTORY_LIMIT + 3) {
                bus.port_out_u8(POST_DIAG_PORT, i as u8).unwrap();
            }
        }
        assert_eq!(m.post_diag.history().len(), POST_CODE_HISTORY_LIMIT);
        assert!(m.post_diag.history_overflow());
        assert_eq!(
            m.post_diag.write_count(),
            POST_CODE_HISTORY_LIMIT as u64 + 3
        );

        m.reset();

        assert!(m.post_diag.history().is_empty());
        assert_eq!(m.post_diag.last_code(), None);
        assert!(!m.post_diag.history_overflow());
    }

    /// Spec: NS16550A / classic PC COM2 `0x2F8`–`0x2FF` — THR OUT + LSR poll on MachineBus.
    #[test]
    fn machine_bus_com2_thr_and_lsr() {
        let mut m = Machine::new(64 * 1024);
        {
            let mut bus = m.bus_mut();
            assert_eq!(bus.port_in_u8(0x2FD).unwrap() & 0x60, 0x60);
            bus.port_out_u8(0x2F8, b'O').unwrap();
            bus.port_out_u8(0x2F8, b'K').unwrap();
            // COM1 sink stays independent.
            bus.port_out_u8(0x3F8, b'1').unwrap();
        }
        assert_eq!(m.com2_text(), "OK");
        assert_eq!(m.com1_text(), "1");
    }

    /// Spec: OSDev I8042 — OUT 0x64,0xAA → IN 0x60 == 0x55; OBF around the path.
    #[test]
    fn machine_bus_i8042_self_test() {
        let mut m = Machine::new(64 * 1024);
        {
            let mut bus = m.bus_mut();
            assert_eq!(bus.port_in_u8(I8042_STATUS_CMD).unwrap() & STATUS_OBF, 0);
            bus.port_out_u8(I8042_STATUS_CMD, CMD_SELF_TEST).unwrap();
            assert_ne!(bus.port_in_u8(I8042_STATUS_CMD).unwrap() & STATUS_OBF, 0);
            assert_eq!(bus.port_in_u8(I8042_DATA).unwrap(), SELF_TEST_OK);
            assert_eq!(bus.port_in_u8(I8042_STATUS_CMD).unwrap() & STATUS_OBF, 0);
        }
    }

    /// Spec: OSDev I8042 — config byte via commands 0x20 (read) / 0x60 (write).
    #[test]
    fn machine_bus_i8042_config_read_write() {
        let mut m = Machine::new(64 * 1024);
        {
            let mut bus = m.bus_mut();
            bus.port_out_u8(I8042_STATUS_CMD, CMD_READ_CONFIG).unwrap();
            let default_cfg = bus.port_in_u8(I8042_DATA).unwrap();
            assert_eq!(default_cfg, 0x50); // clock disable + translate

            let new_cfg = 0x45;
            bus.port_out_u8(I8042_STATUS_CMD, CMD_WRITE_CONFIG).unwrap();
            bus.port_out_u8(I8042_DATA, new_cfg).unwrap();
            bus.port_out_u8(I8042_STATUS_CMD, CMD_READ_CONFIG).unwrap();
            assert_eq!(bus.port_in_u8(I8042_DATA).unwrap(), new_cfg);
        }
        assert_eq!(m.kbd.config, 0x45);
    }

    /// Guest OUT/IN through interpreter → MachineBus 8042 self-test.
    /// Spec: SDM Vol. 2 OUT/IN imm8; OSDev I8042 self-test 0xAA→0x55.
    #[test]
    fn guest_out_in_i8042_self_test() {
        let mut m = Machine::new(64 * 1024);
        let prog: &[u8] = &[
            0xB0,
            CMD_SELF_TEST, // mov al, 0xAA
            0xE6,
            0x64, // out 0x64, al
            0xE4,
            0x60, // in al, 0x60
            0xF4, // hlt
        ];
        for (i, b) in prog.iter().enumerate() {
            m.mem.write_u8(i as u64, *b).unwrap();
        }
        m.cpu = CpuState::reset();
        m.cpu.cs = x86_core::SegmentReg::real_mode_code(0x0000);
        m.cpu.set_ip16(0);
        m.cpu.halted = false;
        let steps = m.run(100).unwrap();
        assert!(steps > 0);
        assert!(m.cpu.halted);
        assert_eq!(m.cpu.al(), SELF_TEST_OK);
    }

    /// Spec: IBM PC/AT port 0x61 — GATE2 + speaker data + ch2 OUT readback on MachineBus.
    #[test]
    fn machine_bus_port61_speaker_gate_and_out2() {
        let mut m = Machine::new(64 * 1024);
        m.pit.port_write(PIT_CONTROL, 1, 0xB0); // ch2 mode 0 lohi
        m.pit.port_write(PIT_CH2_DATA, 1, 0x02);
        m.pit.port_write(PIT_CH2_DATA, 1, 0x00);
        {
            let mut bus = m.bus_mut();
            assert_eq!(bus.port_in_u8(PORT_SYSTEM_CONTROL).unwrap() & 0x03, 0);
            bus.port_out_u8(PORT_SYSTEM_CONTROL, PORT61_GATE2 | PORT61_SPKR_DATA)
                .unwrap();
            assert_eq!(
                bus.port_in_u8(PORT_SYSTEM_CONTROL).unwrap() & (PORT61_GATE2 | PORT61_SPKR_DATA),
                PORT61_GATE2 | PORT61_SPKR_DATA
            );
        }
        assert!(m.pit.channel2().gate);
        assert!(m.pit.speaker_data_enabled());
        m.tick_pit(3); // load + countdown to terminal for count=2
        assert!(m.pit.out_ch2());
        {
            let mut bus = m.bus_mut();
            assert_ne!(
                bus.port_in_u8(PORT_SYSTEM_CONTROL).unwrap() & PORT61_OUT2,
                0
            );
        }
    }

    /// Guest OUT/IN port 0x61 speaker bits through interpreter.
    #[test]
    fn guest_out_in_port61_speaker_bits() {
        let mut m = Machine::new(64 * 1024);
        let prog: &[u8] = &[
            0xB0, 0x03, // mov al, 3 (GATE2|SPKR)
            0xE6, 0x61, // out 0x61, al
            0xE4, 0x61, // in al, 0x61
            0xF4, // hlt
        ];
        for (i, b) in prog.iter().enumerate() {
            m.mem.write_u8(i as u64, *b).unwrap();
        }
        m.cpu = CpuState::reset();
        m.cpu.cs = x86_core::SegmentReg::real_mode_code(0x0000);
        m.cpu.set_ip16(0);
        m.cpu.halted = false;
        let steps = m.run(100).unwrap();
        assert!(steps > 0);
        assert!(m.cpu.halted);
        assert_eq!(m.cpu.al() & 0x03, 0x03);
        assert!(m.pit.channel2().gate);
        assert!(m.pit.speaker_data_enabled());
    }

    /// Reset clears PIC/PIT/CMOS/8042/PCI device state like serial recreation.
    #[test]
    fn reset_clears_pic_pit_cmos_kbd() {
        let mut m = Machine::new(64 * 1024);
        m.pic.port_write(PIC_MASTER_CMD, 1, 0x13);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x08);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0x01);
        m.pit.port_write(PIT_CONTROL, 1, 0x36);
        m.pit.port_write(PIT_CH0_DATA, 1, 0x00);
        m.pit.port_write(PIT_CH0_DATA, 1, 0x10);
        m.cmos.port_write(CMOS_INDEX, 1, 0x10);
        m.cmos.port_write(CMOS_DATA, 1, 0xAB);
        m.kbd
            .port_write(I8042_STATUS_CMD, 1, u32::from(CMD_SELF_TEST));
        m.kbd
            .port_write(I8042_STATUS_CMD, 1, u32::from(CMD_WRITE_CONFIG));
        m.kbd.port_write(I8042_DATA, 1, 0x45);
        m.pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 0, 0, 0x00, true),
        );
        m.com1.port_write(0x3F8, 1, u32::from(b'X'));
        m.com2.port_write(0x2F8, 1, u32::from(b'Y'));

        m.reset();
        assert_eq!(m.pic, DualPic::new());
        assert_eq!(m.pit, Pit8254::new());
        assert_eq!(m.cmos, CmosRtc::new());
        assert_eq!(m.kbd, I8042::new());
        assert_eq!(m.port92, Port92::new());
        assert_eq!(m.pci, PciConfig::new());
        assert!(m.mem.a20_enabled());
        assert_eq!(m.com1_text(), "");
        assert_eq!(m.com2_text(), "");
        assert_eq!(m.debug_text(), "");
    }

    /// Spec: OSDev I8042 — OUT 0x64,0xFE latches pulse-reset; service → Machine::reset
    /// restores CS:IP / reset-vector CPU state and device defaults.
    #[test]
    fn machine_bus_8042_pulse_reset_restores_cpu_reset_vector() {
        let mut m = Machine::new(64 * 1024);
        m.cpu.set_ip16(0x1234);
        m.cpu.cs = x86_core::SegmentReg::real_mode_code(0x1000);
        m.cpu.gpr[CpuState::RAX] = 0xDEAD_BEEF;
        m.kbd
            .port_write(I8042_STATUS_CMD, 1, u32::from(CMD_SELF_TEST));
        assert_ne!(m.kbd, I8042::new());
        {
            let mut bus = m.bus_mut();
            bus.port_out_u8(I8042_STATUS_CMD, CMD_PULSE_RESET).unwrap();
        }
        assert!(m.service_8042_pulse_reset());
        let fresh = CpuState::reset();
        assert_eq!(m.cpu.rip, fresh.rip);
        assert_eq!(m.cpu.cs.selector, fresh.cs.selector);
        assert_eq!(m.cpu.cs.base, fresh.cs.base);
        assert_eq!(m.cpu.gpr[CpuState::RAX], 0);
        assert_eq!(m.kbd, I8042::new());
        assert!(!m.service_8042_pulse_reset());
    }

    /// Spec: IBM PC AT / OSDev I8042 — OUT 0x64,0xD1 / OUT 0x60 with bit0 clear
    /// latches the same system-reset path as pulse-reset `0xFE`.
    #[test]
    fn machine_bus_8042_d1_bit0_low_restores_cpu_reset_vector() {
        let mut m = Machine::new(64 * 1024);
        m.cpu.set_ip16(0x1234);
        m.cpu.cs = x86_core::SegmentReg::real_mode_code(0x1000);
        m.cpu.gpr[CpuState::RAX] = 0xDEAD_BEEF;
        m.kbd
            .port_write(I8042_STATUS_CMD, 1, u32::from(CMD_SELF_TEST));
        assert_ne!(m.kbd, I8042::new());
        {
            let mut bus = m.bus_mut();
            bus.port_out_u8(I8042_STATUS_CMD, CMD_WRITE_OUTPUT_PORT)
                .unwrap();
            bus.port_out_u8(I8042_DATA, 0xDE).unwrap(); // bit0 low → system reset
        }
        assert!(m.service_8042_pulse_reset());
        let fresh = CpuState::reset();
        assert_eq!(m.cpu.rip, fresh.rip);
        assert_eq!(m.cpu.cs.selector, fresh.cs.selector);
        assert_eq!(m.cpu.cs.base, fresh.cs.base);
        assert_eq!(m.cpu.gpr[CpuState::RAX], 0);
        assert_eq!(m.kbd, I8042::new());
        assert!(!m.service_8042_pulse_reset());
    }

    /// Spec: OSDev I8042 — guest OUT 0x64,0xFE after step restores reset vector.
    #[test]
    fn guest_out_8042_pulse_reset_restores_reset_vector() {
        let mut m = Machine::new(64 * 1024);
        let prog: &[u8] = &[
            0xB0,
            CMD_PULSE_RESET, // mov al, 0xFE
            0xE6,
            0x64, // out 0x64, al
        ];
        for (i, b) in prog.iter().enumerate() {
            m.mem.write_u8(i as u64, *b).unwrap();
        }
        m.cpu = CpuState::reset();
        m.cpu.cs = x86_core::SegmentReg::real_mode_code(0x0000);
        m.cpu.set_ip16(0);
        m.cpu.halted = false;
        m.cpu.gpr[CpuState::RAX] = 0x1111;
        m.step().unwrap(); // mov al, 0xFE
        assert_eq!(m.cpu.al(), CMD_PULSE_RESET);
        m.step().unwrap(); // out → pulse-reset → Machine::reset
        let fresh = CpuState::reset();
        assert_eq!(m.cpu.rip, fresh.rip);
        assert_eq!(m.cpu.cs.selector, fresh.cs.selector);
        assert_eq!(m.cpu.cs.base, fresh.cs.base);
        assert_eq!(m.kbd, I8042::new());
    }

    /// Spec: IBM PC AT — OUT 0x64,0xD1 / OUT 0x60,data updates A20 on PhysMem.
    #[test]
    fn machine_bus_8042_a20_gate_masks_phys_bit20() {
        let mut m = Machine::new(2 * 1024 * 1024);
        assert!(m.mem.a20_enabled());
        m.mem.write_u8(0, 0x11).unwrap();
        m.mem.write_u8(1 << 20, 0x22).unwrap();
        {
            let mut bus = m.bus_mut();
            bus.port_out_u8(I8042_STATUS_CMD, CMD_WRITE_OUTPUT_PORT)
                .unwrap();
            bus.port_out_u8(I8042_DATA, 0xDD).unwrap(); // A20 off
        }
        assert!(!m.kbd.a20_enabled());
        assert!(!m.mem.a20_enabled());
        assert_eq!(m.mem.read_u8(1 << 20).unwrap(), 0x11);

        {
            let mut bus = m.bus_mut();
            bus.port_out_u8(I8042_STATUS_CMD, CMD_WRITE_OUTPUT_PORT)
                .unwrap();
            bus.port_out_u8(I8042_DATA, 0xDF).unwrap(); // A20 on
        }
        assert!(m.mem.a20_enabled());
        assert_eq!(m.mem.read_u8(1 << 20).unwrap(), 0x22);
    }

    /// Guest OUT path: 8042 `0xD1` disables A20; mem alias observable via PhysMem.
    #[test]
    fn guest_out_8042_disables_a20() {
        let mut m = Machine::new(2 * 1024 * 1024);
        const MARK: u64 = 0x1000;
        m.mem.write_u8(MARK, 0xAA).unwrap();
        m.mem.write_u8(MARK | (1 << 20), 0xBB).unwrap();
        let prog: &[u8] = &[
            0xB0,
            CMD_WRITE_OUTPUT_PORT, // mov al, 0xD1
            0xE6,
            0x64, // out 0x64, al
            0xB0,
            0xDD, // mov al, 0xDD (A20 off)
            0xE6,
            0x60, // out 0x60, al
            0xF4, // hlt
        ];
        for (i, b) in prog.iter().enumerate() {
            m.mem.write_u8(i as u64, *b).unwrap();
        }
        m.cpu = CpuState::reset();
        m.cpu.cs = x86_core::SegmentReg::real_mode_code(0x0000);
        m.cpu.set_ip16(0);
        m.cpu.halted = false;
        let steps = m.run(100).unwrap();
        assert!(steps > 0);
        assert!(m.cpu.halted);
        assert!(!m.mem.a20_enabled());
        assert!(!m.port92.a20_enabled());
        assert_eq!(m.mem.read_u8(MARK | (1 << 20)).unwrap(), 0xAA);
    }

    /// Spec: OSDev A20 Line — OUT 0x92 bit1 Fast Gate A20 masks PhysMem bit20;
    /// mirrors to 8042 output-port bit1.
    #[test]
    fn machine_bus_port92_a20_gate_masks_phys_bit20() {
        let mut m = Machine::new(2 * 1024 * 1024);
        assert!(m.mem.a20_enabled());
        assert!(m.port92.a20_enabled());
        m.mem.write_u8(0, 0x11).unwrap();
        m.mem.write_u8(1 << 20, 0x22).unwrap();
        {
            let mut bus = m.bus_mut();
            bus.port_out_u8(PORT_SYSTEM_CONTROL_A, 0x00).unwrap(); // A20 off
        }
        assert!(!m.port92.a20_enabled());
        assert!(!m.kbd.a20_enabled());
        assert!(!m.mem.a20_enabled());
        assert_eq!(m.mem.read_u8(1 << 20).unwrap(), 0x11);

        {
            let mut bus = m.bus_mut();
            bus.port_out_u8(PORT_SYSTEM_CONTROL_A, PORT92_A20).unwrap();
        }
        assert!(m.mem.a20_enabled());
        assert!(m.kbd.a20_enabled());
        assert_eq!(m.mem.read_u8(1 << 20).unwrap(), 0x22);
    }

    /// Spec: OSDev A20 Line — port 0x92 bit0 write-1 latches system-reset → Machine::reset.
    #[test]
    fn machine_bus_port92_bit0_restores_cpu_reset_vector() {
        let mut m = Machine::new(64 * 1024);
        m.cpu.set_ip16(0x1234);
        m.cpu.cs = x86_core::SegmentReg::real_mode_code(0x1000);
        m.cpu.gpr[CpuState::RAX] = 0xDEAD_BEEF;
        m.kbd
            .port_write(I8042_STATUS_CMD, 1, u32::from(CMD_SELF_TEST));
        assert_ne!(m.kbd, I8042::new());
        {
            let mut bus = m.bus_mut();
            bus.port_out_u8(PORT_SYSTEM_CONTROL_A, PORT92_RESET | PORT92_A20)
                .unwrap();
        }
        assert!(m.service_8042_pulse_reset());
        let fresh = CpuState::reset();
        assert_eq!(m.cpu.rip, fresh.rip);
        assert_eq!(m.cpu.cs.selector, fresh.cs.selector);
        assert_eq!(m.cpu.cs.base, fresh.cs.base);
        assert_eq!(m.cpu.gpr[CpuState::RAX], 0);
        assert_eq!(m.kbd, I8042::new());
        assert_eq!(m.port92, Port92::new());
        assert!(!m.service_8042_pulse_reset());
    }

    /// Guest OUT path: port 0x92 clears A20 then pulses fast reset.
    #[test]
    fn guest_out_port92_disables_a20_then_reset() {
        let mut m = Machine::new(2 * 1024 * 1024);
        const MARK: u64 = 0x1000;
        m.mem.write_u8(MARK, 0xAA).unwrap();
        m.mem.write_u8(MARK | (1 << 20), 0xBB).unwrap();
        let prog: &[u8] = &[
            0xB0, 0x00, // mov al, 0 (A20 off)
            0xE6, 0x92, // out 0x92, al
            0xF4, // hlt
        ];
        for (i, b) in prog.iter().enumerate() {
            m.mem.write_u8(i as u64, *b).unwrap();
        }
        m.cpu = CpuState::reset();
        m.cpu.cs = x86_core::SegmentReg::real_mode_code(0x0000);
        m.cpu.set_ip16(0);
        m.cpu.halted = false;
        let steps = m.run(100).unwrap();
        assert!(steps > 0);
        assert!(m.cpu.halted);
        assert!(!m.mem.a20_enabled());
        assert!(!m.kbd.a20_enabled());
        assert_eq!(m.mem.read_u8(MARK | (1 << 20)).unwrap(), 0xAA);
    }

    /// Spec: OSDev ISA DMA / Intel 8237A — MachineBus routes master addr + page; 0x80 stays POST.
    #[test]
    fn machine_bus_dma_addr_and_page_preserves_post_80() {
        use devices::DMA_PAGE_CH2;
        let mut m = Machine::new(64 * 1024);
        {
            let mut bus = m.bus_mut();
            bus.port_out_u8(0x00, 0xCD).unwrap();
            bus.port_out_u8(0x00, 0xAB).unwrap();
            // Flip-flop advanced; clear via 0x0C then read back.
            bus.port_out_u8(0x0C, 0x00).unwrap();
            assert_eq!(bus.port_in_u8(0x00).unwrap(), 0xCD);
            assert_eq!(bus.port_in_u8(0x00).unwrap(), 0xAB);
            bus.port_out_u8(DMA_PAGE_CH2, 0x55).unwrap();
            assert_eq!(bus.port_in_u8(DMA_PAGE_CH2).unwrap(), 0x55);
            // IBM PC/AT POST port remains open-bus (not a DMA page).
            assert_eq!(bus.port_in_u8(0x80).unwrap(), 0xFF);
            bus.port_out_u8(0x80, 0x11).unwrap();
            assert_eq!(bus.port_in_u8(0x80).unwrap(), 0xFF);
        }
        assert_eq!(m.dma.master.channels[0].addr, 0xABCD);
        assert_eq!(m.dma.page[2], 0x55);
    }

    #[test]
    fn machine_reset_clears_dma() {
        let mut m = Machine::new(64 * 1024);
        m.dma.port_write(0x00, 1, 0x11);
        m.dma.port_write(0x00, 1, 0x22);
        m.dma.page[2] = 0x99;
        m.reset();
        assert_eq!(m.dma.master.channels[0].addr, 0);
        assert_eq!(m.dma.page[2], 0);
        assert_eq!(m.dma.master.mask, 0x0F);
    }

    /// Program master ch2: page, addr, count=N−1, Single+Inc+Write (`0x46`), unmasked.
    fn program_dma_ch2_write(m: &mut Machine, page: u8, addr: u16, count_minus_one: u16) {
        use devices::DMA_PAGE_CH2;
        m.dma.port_write(0x0C, 1, 0);
        m.dma.port_write(0x04, 1, u32::from(addr & 0xFF));
        m.dma.port_write(0x04, 1, u32::from(addr >> 8));
        m.dma.port_write(0x0C, 1, 0);
        m.dma.port_write(0x05, 1, u32::from(count_minus_one & 0xFF));
        m.dma.port_write(0x05, 1, u32::from(count_minus_one >> 8));
        m.dma.port_write(DMA_PAGE_CH2, 1, u32::from(page));
        m.dma.port_write(0x0B, 1, 0x46); // Single | Inc | Write | ch2
        m.dma.port_write(0x0A, 1, 0x02); // unmask ch2
    }

    /// Program master ch2: page, addr, count=N−1, Single+Inc+Read (`0x4A`), unmasked.
    fn program_dma_ch2_read(m: &mut Machine, page: u8, addr: u16, count_minus_one: u16) {
        use devices::DMA_PAGE_CH2;
        m.dma.port_write(0x0C, 1, 0);
        m.dma.port_write(0x04, 1, u32::from(addr & 0xFF));
        m.dma.port_write(0x04, 1, u32::from(addr >> 8));
        m.dma.port_write(0x0C, 1, 0);
        m.dma.port_write(0x05, 1, u32::from(count_minus_one & 0xFF));
        m.dma.port_write(0x05, 1, u32::from(count_minus_one >> 8));
        m.dma.port_write(DMA_PAGE_CH2, 1, u32::from(page));
        m.dma.port_write(0x0B, 1, 0x4A); // Single | Inc | Read | ch2
        m.dma.port_write(0x0A, 1, 0x02); // unmask ch2
    }

    /// Spec: Intel 8237A + OSDev ISA DMA — `Machine::dma_transfer` Write moves
    /// I/O buffer into PhysMem at `(page<<16)|addr` and latches TC.
    #[test]
    fn machine_dma_transfer_ch2_write_into_physmem_latches_tc() {
        let mut m = Machine::new(256 * 1024);
        program_dma_ch2_write(&mut m, 0x01, 0x1000, 3); // 4 bytes @ 0x1_1000
        let mut io = [0xAAu8, 0xBB, 0xCC, 0xDD];
        let n = m.dma_transfer(2, &mut io).expect("ch2 write into PhysMem");
        assert_eq!(n, 4);
        assert_eq!(m.mem.read_u8(0x1_1000).unwrap(), 0xAA);
        assert_eq!(m.mem.read_u8(0x1_1001).unwrap(), 0xBB);
        assert_eq!(m.mem.read_u8(0x1_1002).unwrap(), 0xCC);
        assert_eq!(m.mem.read_u8(0x1_1003).unwrap(), 0xDD);
        assert_eq!(m.dma.master.channels[2].addr, 0x1004);
        assert_eq!(m.dma.master.channels[2].count, 0xFFFF);
        assert_eq!(m.dma.port_read(0x08, 1) as u8 & 0x0F, 0x04); // TC ch2
        assert_eq!(m.dma.port_read(0x08, 1) as u8 & 0x0F, 0); // clear-on-read
    }

    /// Spec: Intel 8237A Read mode — `Machine::dma_transfer` fills I/O from PhysMem.
    #[test]
    fn machine_dma_transfer_ch2_read_from_physmem() {
        use devices::DMA_PAGE_CH2;
        let mut m = Machine::new(64 * 1024);
        m.mem.write_u8(0x2000, 0x11).unwrap();
        m.mem.write_u8(0x2001, 0x22).unwrap();
        m.dma.port_write(0x0C, 1, 0);
        m.dma.port_write(0x04, 1, 0x00);
        m.dma.port_write(0x04, 1, 0x20); // addr 0x2000
        m.dma.port_write(0x0C, 1, 0);
        m.dma.port_write(0x05, 1, 0x01); // count 1 → 2 bytes
        m.dma.port_write(0x05, 1, 0x00);
        m.dma.port_write(DMA_PAGE_CH2, 1, 0x00);
        m.dma.port_write(0x0B, 1, 0x4A); // Single | Inc | Read | ch2
        m.dma.port_write(0x0A, 1, 0x02);
        let mut io = [0u8; 2];
        let n = m.dma_transfer(2, &mut io).expect("ch2 read from PhysMem");
        assert_eq!(n, 2);
        assert_eq!(io, [0x11, 0x22]);
        assert_eq!(m.dma.master.channels[2].addr, 0x2002);
        assert_eq!(m.dma.master.status & 0x0F, 0x04);
    }

    /// Spec: IBM PC AT A20 — DMA PhysMem callbacks honor the A20 gate mask.
    #[test]
    fn machine_dma_transfer_honors_a20_gate() {
        use devices::DMA_PAGE_CH0;
        let mut m = Machine::new(2 * 1024 * 1024);
        // Distinct values at aliased addresses when A20 is off.
        m.mem.write_u8(0x1000, 0x11).unwrap();
        m.mem.write_u8(0x1000 | (1 << 20), 0x22).unwrap();
        m.mem.set_a20_enabled(false);
        // Program ch0 Write to phys 0x10_1000 (= page 0x10, addr 0x1000).
        m.dma.port_write(0x0C, 1, 0);
        m.dma.port_write(0x00, 1, 0x00);
        m.dma.port_write(0x00, 1, 0x10); // addr 0x1000
        m.dma.port_write(0x0C, 1, 0);
        m.dma.port_write(0x01, 1, 0x00); // count 0 → 1 byte
        m.dma.port_write(0x01, 1, 0x00);
        m.dma.port_write(DMA_PAGE_CH0, 1, 0x10);
        m.dma.port_write(0x0B, 1, 0x44); // Single | Inc | Write | ch0
        m.dma.port_write(0x0A, 1, 0x00); // unmask ch0
        let mut io = [0xAAu8];
        m.dma_transfer(0, &mut io).expect("ch0 write with A20 off");
        // With A20 off, write aliases to 0x1000, not 0x10_1000.
        assert_eq!(m.mem.read_u8(0x1000).unwrap(), 0xAA);
        m.mem.set_a20_enabled(true);
        assert_eq!(m.mem.read_u8(0x1000 | (1 << 20)).unwrap(), 0x22); // untouched
    }

    /// MachineBus path shares the same PhysMem wiring as [`Machine::dma_transfer`].
    #[test]
    fn machine_bus_dma_transfer_ch2_write_into_physmem() {
        let mut m = Machine::new(256 * 1024);
        program_dma_ch2_write(&mut m, 0x01, 0x0800, 1); // 2 bytes @ 0x1_0800
        let mut io = [0xEEu8, 0xFF];
        {
            let mut bus = m.bus_mut();
            let n = bus.dma_transfer(2, &mut io).expect("bus ch2 write");
            assert_eq!(n, 2);
        }
        assert_eq!(m.mem.read_u8(0x1_0800).unwrap(), 0xEE);
        assert_eq!(m.mem.read_u8(0x1_0801).unwrap(), 0xFF);
        assert_eq!(m.dma.master.channels[2].count, 0xFFFF);
        assert_eq!(m.dma.master.status & 0x0F, 0x04);
    }

    /// Spec: IBM VGA text — MachineBus overlays 0xB8000 plane (not PhysMem RAM).
    #[test]
    fn machine_bus_vga_text_mmio_overlay() {
        use devices::VGA_TEXT_BASE;
        let mut m = Machine::new(1024 * 1024);
        // Poison underlying RAM at the same physical address.
        m.mem.write_u8(VGA_TEXT_BASE, 0xEE).unwrap();
        {
            let mut bus = m.bus_mut();
            // Reset default is space, not the RAM poison.
            assert_eq!(bus.read_u8(VGA_TEXT_BASE).unwrap(), b' ');
            bus.write_u8(VGA_TEXT_BASE, b'X').unwrap();
            bus.write_u8(VGA_TEXT_BASE + 1, 0x1F).unwrap();
            assert_eq!(bus.read_u8(VGA_TEXT_BASE).unwrap(), b'X');
            assert_eq!(bus.read_u8(VGA_TEXT_BASE + 1).unwrap(), 0x1F);
            // Outside VGA window still uses PhysMem / open-bus path.
            bus.write_u8(0xA0000, 0x5A).unwrap();
            assert_eq!(bus.read_u8(0xA0000).unwrap(), 0x5A);
        }
        assert_eq!(m.vga.char_at(0, 0), Some(b'X'));
        assert_eq!(m.vga.attr_at(0, 0), Some(0x1F));
        // Overlay did not write through to RAM.
        assert_eq!(m.mem.read_u8(VGA_TEXT_BASE).unwrap(), 0xEE);
    }

    #[test]
    fn machine_reset_clears_vga_text() {
        let mut m = Machine::new(1024 * 1024);
        m.vga.put_char(0, 0, b'Z', 0x4E);
        m.reset();
        assert_eq!(m.vga.char_at(0, 0), Some(b' '));
        assert_eq!(m.vga.attr_at(0, 0), Some(0x07));
    }

    /// Spec: OSDev VGA Hardware — CRTC index/data via MachineBus `0x3D4`/`0x3D5`.
    #[test]
    fn machine_bus_vga_crtc_index_data() {
        let mut m = Machine::new(64 * 1024);
        {
            let mut bus = m.bus_mut();
            bus.port_out_u8(VGA_CRTC_INDEX, 0x0E).unwrap();
            bus.port_out_u8(VGA_CRTC_DATA, 0x12).unwrap();
            bus.port_out_u8(VGA_CRTC_INDEX, 0x0E).unwrap();
            assert_eq!(bus.port_in_u8(VGA_CRTC_DATA).unwrap(), 0x12);
            assert_eq!(bus.port_in_u8(VGA_CRTC_INDEX).unwrap(), 0x0E);
        }
        assert_eq!(m.vga.crtc_regs[0x0E], 0x12);
        m.reset();
        assert_eq!(m.vga.crtc_regs[0x0E], 0);
    }

    /// Spec: FreeVGA / OSDev VGA Hardware — Misc Output write `0x3C2`, readback `0x3CC`.
    #[test]
    fn machine_bus_vga_misc_output_round_trip() {
        let mut m = Machine::new(64 * 1024);
        assert_eq!(m.vga.misc_output, VGA_MISC_OUTPUT_DEFAULT);
        {
            let mut bus = m.bus_mut();
            assert_eq!(
                bus.port_in_u8(VGA_MISC_OUTPUT_READ).unwrap(),
                VGA_MISC_OUTPUT_DEFAULT
            );
            bus.port_out_u8(VGA_MISC_OUTPUT_WRITE, 0xA5).unwrap();
            assert_eq!(bus.port_in_u8(VGA_MISC_OUTPUT_READ).unwrap(), 0xA5);
            // Write-only at 0x3C2: open-bus-style 0xFF on read.
            assert_eq!(bus.port_in_u8(VGA_MISC_OUTPUT_WRITE).unwrap(), 0xFF);
        }
        assert_eq!(m.vga.misc_output, 0xA5);
        m.reset();
        assert_eq!(m.vga.misc_output, VGA_MISC_OUTPUT_DEFAULT);
    }

    /// Spec: FreeVGA Color Registers / OSDev VGA Hardware — DAC PEL `0x3C8`/`0x3C9`/`0x3C7`.
    #[test]
    fn machine_bus_vga_dac_pel_round_trip() {
        let mut m = Machine::new(64 * 1024);
        {
            let mut bus = m.bus_mut();
            bus.port_out_u8(VGA_DAC_WRITE_INDEX, 0x10).unwrap();
            bus.port_out_u8(VGA_DAC_DATA, 0x3F).unwrap();
            bus.port_out_u8(VGA_DAC_DATA, 0x2A).unwrap();
            bus.port_out_u8(VGA_DAC_DATA, 0x15).unwrap();
            bus.port_out_u8(VGA_DAC_READ_INDEX, 0x10).unwrap();
            assert_eq!(bus.port_in_u8(VGA_DAC_DATA).unwrap(), 0x3F);
            assert_eq!(bus.port_in_u8(VGA_DAC_DATA).unwrap(), 0x2A);
            assert_eq!(bus.port_in_u8(VGA_DAC_DATA).unwrap(), 0x15);
        }
        assert_eq!(m.vga.dac_ram[0x10], [0x3F, 0x2A, 0x15]);
        m.reset();
        assert_eq!(m.vga.dac_ram[0x10], [0x00, 0x00, 0x00]);
    }

    /// Spec: Intel 82371 / OSDev ELCR — MachineBus `0x4D0`/`0x4D1` writes update
    /// DualPic per-IR level masks; level IRQ redelivers while held after EOI.
    #[test]
    fn machine_bus_elcr_wires_dual_pic_level_select() {
        let mut m = Machine::new(64 * 1024);
        init_at_pic_unmask_irq0(&mut m);
        // Remask IRQ0; unmask IRQ3 for this test.
        m.pic.port_write(PIC_MASTER_DATA, 1, 0xF7);
        {
            let mut bus = m.bus_mut();
            bus.port_out_u8(PIIX_ELCR_MASTER, 1 << 3).unwrap();
            bus.port_out_u8(PIIX_ELCR_SLAVE, 0).unwrap();
            assert_eq!(bus.port_in_u8(PIIX_ELCR_MASTER).unwrap(), 1 << 3);
        }
        assert_eq!(m.pci.elcr, [1 << 3, 0]);
        assert_eq!(m.pic.elcr_level_mask(), (1 << 3, 0));
        assert!(m.pic.master.ir_is_level(3));

        m.pic.set_irq_line(3, true);
        assert_eq!(m.pic.poll_irq(), Some(0x0B));
        m.pic.port_write(PIC_MASTER_CMD, 1, 0x20);
        // Level still high via ELCR → redelivery without a new edge.
        assert_eq!(m.pic.poll_irq(), Some(0x0B));

        m.reset();
        assert_eq!(m.pci.elcr, [0, 0]);
        assert_eq!(m.pic.elcr_level_mask(), (0, 0));
    }

    /// Spec: Intel 82371 / IFB ELCR — IRQ0/1/2/8/13 reserved hardwired edge;
    /// MachineBus write of 0xFF must not level those IRQs.
    #[test]
    fn machine_bus_elcr_reserved_irqs_stay_edge() {
        use devices::{PIIX_ELCR_MASTER_WRITABLE, PIIX_ELCR_SLAVE_WRITABLE};

        let mut m = Machine::new(64 * 1024);
        init_at_pic_unmask_irq0(&mut m);
        {
            let mut bus = m.bus_mut();
            bus.port_out_u8(PIIX_ELCR_MASTER, 0xFF).unwrap();
            bus.port_out_u8(PIIX_ELCR_SLAVE, 0xFF).unwrap();
            assert_eq!(
                bus.port_in_u8(PIIX_ELCR_MASTER).unwrap(),
                PIIX_ELCR_MASTER_WRITABLE
            );
            assert_eq!(
                bus.port_in_u8(PIIX_ELCR_SLAVE).unwrap(),
                PIIX_ELCR_SLAVE_WRITABLE
            );
        }
        assert_eq!(
            m.pci.elcr,
            [PIIX_ELCR_MASTER_WRITABLE, PIIX_ELCR_SLAVE_WRITABLE]
        );
        assert_eq!(
            m.pic.elcr_level_mask(),
            (PIIX_ELCR_MASTER_WRITABLE, PIIX_ELCR_SLAVE_WRITABLE)
        );
        assert!(!m.pic.master.ir_is_level(0));
        assert!(!m.pic.master.ir_is_level(1));
        assert!(!m.pic.master.ir_is_level(2));
        assert!(!m.pic.slave.ir_is_level(0));
        assert!(!m.pic.slave.ir_is_level(5));

        // Held-high IRQ0: edge semantics (no post-EOI redelivery).
        m.pic.set_irq_line(0, true);
        assert_eq!(m.pic.poll_irq(), Some(0x08));
        m.pic.port_write(PIC_MASTER_CMD, 1, 0x20);
        assert_eq!(m.pic.poll_irq(), None);
    }

    /// Spec: Intel 82371SB PIRQRC — disabled (bit7) `assert_pirq` does not raise DualPic.
    #[test]
    fn machine_assert_pirq_disabled_does_not_route() {
        let mut m = Machine::new(64 * 1024);
        init_at_pic_unmask_irq0(&mut m);
        // Unmask IRQ5; default PIRQRC[A]=0x80 (disabled).
        m.pic.port_write(PIC_MASTER_DATA, 1, 0xDF);
        m.assert_pirq(0, true);
        assert_eq!(m.pic.poll_irq(), None);
        assert_eq!(m.pci.pirq_pic_driven, 0);
    }

    /// Spec: Intel 82371SB PIRQRC — unmasked route + `assert_pirq` → DualPic ISA IRQ.
    #[test]
    fn machine_assert_pirq_routes_to_isa_irq() {
        let mut m = Machine::new(64 * 1024);
        init_at_pic_unmask_irq0(&mut m);
        m.pic.port_write(PIC_MASTER_DATA, 1, 0xDF); // unmask IRQ5
        {
            let mut bus = m.bus_mut();
            bus.port_out_u32(
                PCI_CONFIG_ADDRESS,
                PciConfig::make_address(0, 1, 0, PCI_PIIX_ISA_PIRQRC_OFFSET, true),
            )
            .unwrap();
            bus.port_out_u8(PCI_CONFIG_DATA, 0x05).unwrap(); // PIRQA → IRQ5
            assert_eq!(bus.port_in_u8(PCI_CONFIG_DATA).unwrap(), 0x05);
        }
        m.assert_pirq(0, true);
        assert_eq!(m.pic.poll_irq(), Some(0x0D));
        m.pic.port_write(PIC_MASTER_CMD, 1, 0x20);

        // Disable via MachineBus CONFIG_DATA while still asserted → line drops.
        {
            let mut bus = m.bus_mut();
            bus.port_out_u8(PCI_CONFIG_DATA, 0x80).unwrap();
        }
        assert_eq!(m.pci.pirq_pic_driven, 0);
        assert_eq!(m.pic.poll_irq(), None);
    }

    /// Spec: PCI Local Bus Mechanism #1 — host bridge vendor/device via MachineBus.
    #[test]
    fn machine_bus_pci_host_bridge_vendor() {
        let mut m = Machine::new(64 * 1024);
        let mut bus = m.bus_mut();
        bus.port_out_u32(
            PCI_CONFIG_ADDRESS,
            PciConfig::make_address(0, 0, 0, 0x00, true),
        )
        .unwrap();
        assert_eq!(bus.port_in_u32(PCI_CONFIG_DATA).unwrap(), 0x1237_8086);
        assert_eq!(bus.port_in_u8(PCI_CONFIG_DATA).unwrap(), 0x86);
    }

    /// Spec: OSDev PCI — absent slot reads 0xFFFFFFFF; enable-clear open-bus.
    #[test]
    fn machine_bus_pci_absent_and_enable_clear() {
        let mut m = Machine::new(64 * 1024);
        {
            let mut bus = m.bus_mut();
            bus.port_out_u32(
                PCI_CONFIG_ADDRESS,
                PciConfig::make_address(0, 0x1F, 0, 0x00, true),
            )
            .unwrap();
            assert_eq!(bus.port_in_u32(PCI_CONFIG_DATA).unwrap(), 0xFFFF_FFFF);
            bus.port_out_u32(
                PCI_CONFIG_ADDRESS,
                PciConfig::make_address(0, 0, 0, 0x00, false),
            )
            .unwrap();
            assert_eq!(bus.port_in_u32(PCI_CONFIG_DATA).unwrap(), 0xFFFF_FFFF);
        }
        assert_eq!(m.pci.address & (1 << 31), 0);
    }

    /// Spec: PCI + PIIX public IDs — MachineBus enumerates 00:01.0..00:01.3.
    #[test]
    fn machine_bus_pci_piix_isa_and_ide() {
        use devices::{
            PCI_CLASS_BRIDGE, PCI_CLASS_SERIAL_BUS, PCI_DEVICE_PIIX3_IDE, PCI_DEVICE_PIIX3_ISA,
            PCI_DEVICE_PIIX3_USB, PCI_DEVICE_PIIX_ACPI, PCI_HEADER_MULTIFUNCTION, PCI_PROG_IF_UHCI,
            PCI_SUBCLASS_OTHER_BRIDGE, PCI_SUBCLASS_USB,
        };
        let mut m = Machine::new(64 * 1024);
        let mut bus = m.bus_mut();
        bus.port_out_u32(
            PCI_CONFIG_ADDRESS,
            PciConfig::make_address(0, 1, 0, 0x00, true),
        )
        .unwrap();
        assert_eq!(
            bus.port_in_u32(PCI_CONFIG_DATA).unwrap(),
            (u32::from(PCI_DEVICE_PIIX3_ISA) << 16) | 0x8086
        );
        bus.port_out_u32(
            PCI_CONFIG_ADDRESS,
            PciConfig::make_address(0, 1, 0, 0x0C, true),
        )
        .unwrap();
        assert_eq!(
            ((bus.port_in_u32(PCI_CONFIG_DATA).unwrap() >> 16) & 0xFF) as u8,
            PCI_HEADER_MULTIFUNCTION
        );
        bus.port_out_u32(
            PCI_CONFIG_ADDRESS,
            PciConfig::make_address(0, 1, 1, 0x00, true),
        )
        .unwrap();
        assert_eq!(
            bus.port_in_u32(PCI_CONFIG_DATA).unwrap(),
            (u32::from(PCI_DEVICE_PIIX3_IDE) << 16) | 0x8086
        );
        bus.port_out_u32(
            PCI_CONFIG_ADDRESS,
            PciConfig::make_address(0, 1, 2, 0x00, true),
        )
        .unwrap();
        assert_eq!(
            bus.port_in_u32(PCI_CONFIG_DATA).unwrap(),
            (u32::from(PCI_DEVICE_PIIX3_USB) << 16) | 0x8086
        );
        bus.port_out_u32(
            PCI_CONFIG_ADDRESS,
            PciConfig::make_address(0, 1, 2, 0x08, true),
        )
        .unwrap();
        let usb_class = bus.port_in_u32(PCI_CONFIG_DATA).unwrap();
        assert_eq!((usb_class >> 24) as u8, PCI_CLASS_SERIAL_BUS);
        assert_eq!((usb_class >> 16) as u8, PCI_SUBCLASS_USB);
        assert_eq!((usb_class >> 8) as u8, PCI_PROG_IF_UHCI);
        bus.port_out_u32(
            PCI_CONFIG_ADDRESS,
            PciConfig::make_address(0, 1, 3, 0x00, true),
        )
        .unwrap();
        assert_eq!(
            bus.port_in_u32(PCI_CONFIG_DATA).unwrap(),
            (u32::from(PCI_DEVICE_PIIX_ACPI) << 16) | 0x8086
        );
        bus.port_out_u32(
            PCI_CONFIG_ADDRESS,
            PciConfig::make_address(0, 1, 3, 0x08, true),
        )
        .unwrap();
        let acpi_class = bus.port_in_u32(PCI_CONFIG_DATA).unwrap();
        assert_eq!((acpi_class >> 24) as u8, PCI_CLASS_BRIDGE);
        assert_eq!((acpi_class >> 16) as u8, PCI_SUBCLASS_OTHER_BRIDGE);
    }

    /// Spec: Intel 82371SB BMIDE — MachineBus decodes BMIBA when Command.IO set.
    #[test]
    fn machine_bus_piix_ide_bmide_port_decode() {
        use devices::{PCI_COMMAND_IO, PCI_COMMAND_OFFSET, PCI_PIIX_IDE_BMIBA_OFFSET};
        let mut m = Machine::new(64 * 1024);
        {
            let mut bus = m.bus_mut();
            bus.port_out_u32(
                PCI_CONFIG_ADDRESS,
                PciConfig::make_address(0, 1, 1, PCI_PIIX_IDE_BMIBA_OFFSET, true),
            )
            .unwrap();
            bus.port_out_u32(PCI_CONFIG_DATA, 0x0000_E000).unwrap();
            bus.port_out_u32(
                PCI_CONFIG_ADDRESS,
                PciConfig::make_address(0, 1, 1, PCI_COMMAND_OFFSET, true),
            )
            .unwrap();
            bus.port_out_u16(PCI_CONFIG_DATA, PCI_COMMAND_IO).unwrap();

            bus.port_out_u8(0xE000, 0x08).unwrap();
            bus.port_out_u8(0xE002, 0x40).unwrap();
            bus.port_out_u32(0xE004, 0x1000_2000).unwrap();
            assert_eq!(bus.port_in_u8(0xE000).unwrap(), 0x08);
            assert_eq!(bus.port_in_u8(0xE002).unwrap(), 0x40);
            assert_eq!(bus.port_in_u32(0xE004).unwrap(), 0x1000_2000);

            // Clear IO → decode off (generic port sink, not BMIDE readback).
            bus.port_out_u16(PCI_CONFIG_DATA, 0).unwrap();
            bus.port_out_u8(0xE000, 0xFF).unwrap();
        }
        // Re-open with IO: prior disabled write must not have overwritten 0x08.
        {
            let mut bus = m.bus_mut();
            bus.port_out_u32(
                PCI_CONFIG_ADDRESS,
                PciConfig::make_address(0, 1, 1, PCI_COMMAND_OFFSET, true),
            )
            .unwrap();
            bus.port_out_u16(PCI_CONFIG_DATA, PCI_COMMAND_IO).unwrap();
            assert_eq!(bus.port_in_u8(0xE000).unwrap(), 0x08);
        }
        m.reset();
        assert_eq!(m.pci.bmide_io, [0; 16]);
        assert_eq!(m.pci.bmide_io_base(), None);
    }

    /// Spec: Intel BMIDE PRD — `Machine::bmide_prd_read` one-PRD walk via PhysMem.
    #[test]
    fn machine_bmide_prd_read_copies_into_phys_mem() {
        use devices::{
            PCI_COMMAND_BUS_MASTER, PCI_COMMAND_IO, PCI_COMMAND_OFFSET, PCI_PIIX_IDE_BMIBA_OFFSET,
            PCI_PIIX_IDE_BMIDTP_PRIMARY, PCI_PIIX_IDE_PRD_EOT,
        };
        let mut m = Machine::new(64 * 1024);
        {
            let mut bus = m.bus_mut();
            bus.port_out_u32(
                PCI_CONFIG_ADDRESS,
                PciConfig::make_address(0, 1, 1, PCI_PIIX_IDE_BMIBA_OFFSET, true),
            )
            .unwrap();
            bus.port_out_u32(PCI_CONFIG_DATA, 0x0000_E000).unwrap();
            bus.port_out_u32(
                PCI_CONFIG_ADDRESS,
                PciConfig::make_address(0, 1, 1, PCI_COMMAND_OFFSET, true),
            )
            .unwrap();
            bus.port_out_u16(PCI_CONFIG_DATA, PCI_COMMAND_IO | PCI_COMMAND_BUS_MASTER)
                .unwrap();
            bus.port_out_u32(0xE000 + u16::from(PCI_PIIX_IDE_BMIDTP_PRIMARY), 0x1000)
                .unwrap();
        }
        const BUF: u64 = 0x2000;
        let mut prd = [0u8; 8];
        prd[0..4].copy_from_slice(&(BUF as u32).to_le_bytes());
        prd[4..6].copy_from_slice(&4u16.to_le_bytes());
        prd[7] = PCI_PIIX_IDE_PRD_EOT;
        for (i, b) in prd.iter().enumerate() {
            m.mem.write_u8(0x1000 + i as u64, *b).unwrap();
        }
        let device = [0xDE, 0xAD, 0xBE, 0xEF];
        let xfer = m.bmide_prd_read(&device).expect("prd read");
        assert_eq!(xfer.bytes_copied, 4);
        assert!(xfer.entry.eot);
        for (i, expected) in device.iter().enumerate() {
            assert_eq!(m.mem.read_u8(BUF + i as u64).unwrap(), *expected);
        }
    }

    /// Spec: Intel 82371AB ACPI PM — MachineBus decodes PMBASE when Command.IO set.
    #[test]
    fn machine_bus_piix_acpi_pm_port_decode() {
        use devices::{
            PCI_COMMAND_IO, PCI_COMMAND_OFFSET, PCI_PIIX_ACPI_PM1A_CNT, PCI_PIIX_ACPI_PM1A_EVT,
            PCI_PIIX_ACPI_PMBASE_OFFSET, PCI_PIIX_ACPI_PM_TMR,
        };
        let mut m = Machine::new(64 * 1024);
        {
            let mut bus = m.bus_mut();
            bus.port_out_u32(
                PCI_CONFIG_ADDRESS,
                PciConfig::make_address(0, 1, 3, PCI_PIIX_ACPI_PMBASE_OFFSET, true),
            )
            .unwrap();
            bus.port_out_u32(PCI_CONFIG_DATA, 0x0000_B000).unwrap();
            bus.port_out_u32(
                PCI_CONFIG_ADDRESS,
                PciConfig::make_address(0, 1, 3, PCI_COMMAND_OFFSET, true),
            )
            .unwrap();
            bus.port_out_u16(PCI_CONFIG_DATA, PCI_COMMAND_IO).unwrap();

            bus.port_out_u16(0xB000 + u16::from(PCI_PIIX_ACPI_PM1A_EVT), 0x0101)
                .unwrap();
            bus.port_out_u16(0xB000 + u16::from(PCI_PIIX_ACPI_PM1A_CNT), 0x0001)
                .unwrap();
            bus.port_out_u32(0xB000 + u16::from(PCI_PIIX_ACPI_PM_TMR), 0x0012_3456)
                .unwrap();
            assert_eq!(
                bus.port_in_u16(0xB000 + u16::from(PCI_PIIX_ACPI_PM1A_EVT))
                    .unwrap(),
                0x0101
            );
            assert_eq!(
                bus.port_in_u16(0xB000 + u16::from(PCI_PIIX_ACPI_PM1A_CNT))
                    .unwrap(),
                0x0001
            );
            assert_eq!(
                bus.port_in_u32(0xB000 + u16::from(PCI_PIIX_ACPI_PM_TMR))
                    .unwrap(),
                0x0012_3456
            );

            // Clear IO → decode off (generic port sink, not ACPI PM readback).
            bus.port_out_u16(PCI_CONFIG_DATA, 0).unwrap();
            bus.port_out_u16(0xB000 + u16::from(PCI_PIIX_ACPI_PM1A_CNT), 0xFFFF)
                .unwrap();
        }
        // Re-open with IO: prior disabled write must not have overwritten 0x0001.
        {
            let mut bus = m.bus_mut();
            bus.port_out_u32(
                PCI_CONFIG_ADDRESS,
                PciConfig::make_address(0, 1, 3, PCI_COMMAND_OFFSET, true),
            )
            .unwrap();
            bus.port_out_u16(PCI_CONFIG_DATA, PCI_COMMAND_IO).unwrap();
            assert_eq!(
                bus.port_in_u16(0xB000 + u16::from(PCI_PIIX_ACPI_PM1A_CNT))
                    .unwrap(),
                0x0001
            );
        }
        m.reset();
        assert_eq!(m.pci.acpi_pm_io, [0; 64]);
        assert_eq!(m.pci.acpi_pm_io_base(), None);
    }

    /// Spec: Intel 82371SB UHCI — MachineBus decodes BAR0 when Command.IO set.
    #[test]
    fn machine_bus_piix_usb_uhci_bar0_port_decode() {
        use devices::{PCI_COMMAND_IO, PCI_COMMAND_OFFSET, PCI_PIIX_USB_BAR0_OFFSET};
        let mut m = Machine::new(64 * 1024);
        {
            let mut bus = m.bus_mut();
            bus.port_out_u32(
                PCI_CONFIG_ADDRESS,
                PciConfig::make_address(0, 1, 2, PCI_PIIX_USB_BAR0_OFFSET, true),
            )
            .unwrap();
            bus.port_out_u32(PCI_CONFIG_DATA, 0x0000_D000).unwrap();
            bus.port_out_u32(
                PCI_CONFIG_ADDRESS,
                PciConfig::make_address(0, 1, 2, PCI_COMMAND_OFFSET, true),
            )
            .unwrap();
            bus.port_out_u16(PCI_CONFIG_DATA, PCI_COMMAND_IO).unwrap();

            bus.port_out_u16(0xD000, 0x0001).unwrap(); // USBCMD
            bus.port_out_u16(0xD002, 0x0020).unwrap(); // USBSTS
            bus.port_out_u32(0xD008, 0x1000_2000).unwrap(); // FLBASEADD
            assert_eq!(bus.port_in_u16(0xD000).unwrap(), 0x0001);
            assert_eq!(bus.port_in_u16(0xD002).unwrap(), 0x0020);
            assert_eq!(bus.port_in_u32(0xD008).unwrap(), 0x1000_2000);

            // Clear IO → decode off (generic port sink, not UHCI readback).
            bus.port_out_u16(PCI_CONFIG_DATA, 0).unwrap();
            bus.port_out_u16(0xD000, 0xFFFF).unwrap();
        }
        // Re-open with IO: prior disabled write must not have overwritten USBCMD.
        {
            let mut bus = m.bus_mut();
            bus.port_out_u32(
                PCI_CONFIG_ADDRESS,
                PciConfig::make_address(0, 1, 2, PCI_COMMAND_OFFSET, true),
            )
            .unwrap();
            bus.port_out_u16(PCI_CONFIG_DATA, PCI_COMMAND_IO).unwrap();
            assert_eq!(bus.port_in_u16(0xD000).unwrap(), 0x0001);
        }
        m.reset();
        assert_eq!(m.pci.uhci_io, [0; 32]);
        assert_eq!(m.pci.uhci_io_base(), None);
    }

    /// Spec: ATA/ATAPI + OSDev ATA PIO — MachineBus primary IDE IDENTIFY + READ/WRITE SECTORS.
    #[test]
    fn machine_bus_ide_identify_and_read_sectors() {
        use devices::{
            ATA_CMD_IDENTIFY, ATA_CMD_READ_SECTORS, ATA_CMD_WRITE_SECTORS, ATA_DRIVE_LBA,
            ATA_SR_DRQ, IDE_PRIMARY_DATA, IDE_PRIMARY_DRIVE, IDE_PRIMARY_LBA_HI,
            IDE_PRIMARY_LBA_LO, IDE_PRIMARY_LBA_MID, IDE_PRIMARY_SECCOUNT, IDE_PRIMARY_STATUS,
        };
        let mut img = vec![0u8; 512 * 2];
        img[0] = 0xDE;
        img[1] = 0xAD;
        let mut m = Machine::new(64 * 1024);
        m.ide.attach_image(img);
        {
            let mut bus = m.bus_mut();
            bus.port_out_u8(IDE_PRIMARY_DRIVE, 0xA0).unwrap();
            bus.port_out_u8(IDE_PRIMARY_STATUS, ATA_CMD_IDENTIFY)
                .unwrap();
            assert_ne!(bus.port_in_u8(IDE_PRIMARY_STATUS).unwrap() & ATA_SR_DRQ, 0);
            let w0 = bus.port_in_u16(IDE_PRIMARY_DATA).unwrap();
            assert_eq!(w0, 0x0040);
            for _ in 1..256 {
                let _ = bus.port_in_u16(IDE_PRIMARY_DATA).unwrap();
            }
            assert_eq!(bus.port_in_u8(IDE_PRIMARY_STATUS).unwrap() & ATA_SR_DRQ, 0);

            bus.port_out_u8(IDE_PRIMARY_DRIVE, 0xA0 | ATA_DRIVE_LBA)
                .unwrap();
            bus.port_out_u8(IDE_PRIMARY_SECCOUNT, 1).unwrap();
            bus.port_out_u8(IDE_PRIMARY_LBA_LO, 0).unwrap();
            bus.port_out_u8(IDE_PRIMARY_LBA_MID, 0).unwrap();
            bus.port_out_u8(IDE_PRIMARY_LBA_HI, 0).unwrap();
            bus.port_out_u8(IDE_PRIMARY_STATUS, ATA_CMD_READ_SECTORS)
                .unwrap();
            assert_ne!(bus.port_in_u8(IDE_PRIMARY_STATUS).unwrap() & ATA_SR_DRQ, 0);
            assert_eq!(bus.port_in_u16(IDE_PRIMARY_DATA).unwrap(), 0xADDE);
            for _ in 1..256 {
                let _ = bus.port_in_u16(IDE_PRIMARY_DATA).unwrap();
            }

            // WRITE SECTORS LBA1 then READ back.
            bus.port_out_u8(IDE_PRIMARY_DRIVE, 0xA0 | ATA_DRIVE_LBA)
                .unwrap();
            bus.port_out_u8(IDE_PRIMARY_SECCOUNT, 1).unwrap();
            bus.port_out_u8(IDE_PRIMARY_LBA_LO, 1).unwrap();
            bus.port_out_u8(IDE_PRIMARY_LBA_MID, 0).unwrap();
            bus.port_out_u8(IDE_PRIMARY_LBA_HI, 0).unwrap();
            bus.port_out_u8(IDE_PRIMARY_STATUS, ATA_CMD_WRITE_SECTORS)
                .unwrap();
            assert_ne!(bus.port_in_u8(IDE_PRIMARY_STATUS).unwrap() & ATA_SR_DRQ, 0);
            bus.port_out_u16(IDE_PRIMARY_DATA, 0x55AA).unwrap();
            for _ in 1..256 {
                bus.port_out_u16(IDE_PRIMARY_DATA, 0).unwrap();
            }
            assert_eq!(bus.port_in_u8(IDE_PRIMARY_STATUS).unwrap() & ATA_SR_DRQ, 0);
            bus.port_out_u8(IDE_PRIMARY_DRIVE, 0xA0 | ATA_DRIVE_LBA)
                .unwrap();
            bus.port_out_u8(IDE_PRIMARY_SECCOUNT, 1).unwrap();
            bus.port_out_u8(IDE_PRIMARY_LBA_LO, 1).unwrap();
            bus.port_out_u8(IDE_PRIMARY_LBA_MID, 0).unwrap();
            bus.port_out_u8(IDE_PRIMARY_LBA_HI, 0).unwrap();
            bus.port_out_u8(IDE_PRIMARY_STATUS, ATA_CMD_READ_SECTORS)
                .unwrap();
            assert_eq!(bus.port_in_u16(IDE_PRIMARY_DATA).unwrap(), 0x55AA);
        }
        assert_eq!(m.ide.image[512], 0xAA);
        assert_eq!(m.ide.image[513], 0x55);
    }

    #[test]
    fn machine_reset_preserves_ide_image_and_clears_drq() {
        use devices::{ATA_CMD_IDENTIFY, ATA_SR_DRQ, IDE_PRIMARY_DRIVE, IDE_PRIMARY_STATUS};
        let mut m = Machine::new(64 * 1024);
        m.ide.attach_image(vec![0u8; 512]);
        m.ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        m.ide
            .port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY));
        assert_ne!(m.ide.port_read(IDE_PRIMARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);
        m.reset();
        assert!(m.ide.present);
        assert_eq!(m.ide.image.len(), 512);
        assert_eq!(m.ide.port_read(IDE_PRIMARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);
    }

    /// Spec: ATA + OSDev ATA PIO + IBM PC AT — IDENTIFY → IRQ14 → vector 0x76.
    #[test]
    fn ide_identify_asserts_irq14_via_poll_external_irq() {
        use devices::{
            ATA_CMD_IDENTIFY, ATA_SR_DRQ, IDE_PRIMARY_CTRL, IDE_PRIMARY_DRIVE, IDE_PRIMARY_STATUS,
        };
        let mut m = Machine::new(64 * 1024);
        init_at_pic_unmask_irq14(&mut m);
        m.ide.attach_image(vec![0u8; 512]);
        // Clear nIEN so INTRQ is driven.
        m.ide.port_write(IDE_PRIMARY_CTRL, 1, 0);
        m.ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        m.ide
            .port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY));
        assert!(m.ide.irq_line());
        assert_ne!(m.ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
        {
            let mut bus = m.bus_mut();
            // Spec: AT slave ICW2 base 0x70 → IRQ14 vector 0x76.
            assert_eq!(bus.poll_external_irq(), Some(0x76));
            assert_eq!(bus.poll_external_irq(), None);
        }
        // Status read clears IDE INTRQ; EOI already done by poll path.
        let _ = m.ide.port_read(IDE_PRIMARY_STATUS, 1);
        assert!(!m.ide.irq_line());
        m.pic.port_write(PIC_MASTER_CMD, 1, 0x20);
        m.pic.port_write(PIC_SLAVE_CMD, 1, 0x20);
    }

    /// Spec: ATA nIEN=1 — IDENTIFY sets DRQ but does not deliver IRQ14.
    #[test]
    fn ide_nien_masks_irq14_on_machine_bus() {
        use devices::{
            ATA_CMD_IDENTIFY, ATA_DC_NIEN, ATA_SR_DRQ, IDE_PRIMARY_CTRL, IDE_PRIMARY_DRIVE,
            IDE_PRIMARY_STATUS,
        };
        let mut m = Machine::new(64 * 1024);
        init_at_pic_unmask_irq14(&mut m);
        m.ide.attach_image(vec![0u8; 512]);
        m.ide
            .port_write(IDE_PRIMARY_CTRL, 1, u32::from(ATA_DC_NIEN));
        m.ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        m.ide
            .port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY));
        assert_ne!(m.ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
        assert!(!m.ide.irq_line());
        {
            let mut bus = m.bus_mut();
            assert_eq!(bus.poll_external_irq(), None);
        }
    }

    /// Spec: ATA + OSDev ATA PIO + IBM PC AT — secondary IDENTIFY → IRQ15 → vector 0x77.
    #[test]
    fn ide_secondary_identify_asserts_irq15_via_poll_external_irq() {
        use devices::{
            ATA_CMD_IDENTIFY, ATA_SR_DRQ, IDE_SECONDARY_CTRL, IDE_SECONDARY_DRIVE,
            IDE_SECONDARY_STATUS,
        };
        let mut m = Machine::new(64 * 1024);
        init_at_pic_unmask_irq15(&mut m);
        m.ide_secondary.attach_image(vec![0u8; 512]);
        m.ide_secondary.port_write(IDE_SECONDARY_CTRL, 1, 0);
        m.ide_secondary.port_write(IDE_SECONDARY_DRIVE, 1, 0xA0);
        m.ide_secondary
            .port_write(IDE_SECONDARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY));
        assert!(m.ide_secondary.irq_line());
        assert_ne!(
            m.ide_secondary.port_read(IDE_SECONDARY_CTRL, 1) as u8 & ATA_SR_DRQ,
            0
        );
        {
            let mut bus = m.bus_mut();
            // Spec: AT slave ICW2 base 0x70 → IRQ15 vector 0x77.
            assert_eq!(bus.poll_external_irq(), Some(0x77));
            assert_eq!(bus.poll_external_irq(), None);
        }
        let _ = m.ide_secondary.port_read(IDE_SECONDARY_STATUS, 1);
        assert!(!m.ide_secondary.irq_line());
    }

    /// Spec: OSDev ATA PIO — secondary ports decode on MachineBus; absent → status 0.
    #[test]
    fn machine_bus_ide_secondary_absent_status_zero() {
        use devices::IDE_SECONDARY_STATUS;
        let mut m = Machine::new(64 * 1024);
        {
            let mut bus = m.bus_mut();
            assert_eq!(bus.port_in_u8(IDE_SECONDARY_STATUS).unwrap(), 0);
            bus.port_out_u8(0x176, 0xA0).unwrap();
            bus.port_out_u8(IDE_SECONDARY_STATUS, 0xEC).unwrap();
            assert_eq!(bus.port_in_u8(IDE_SECONDARY_STATUS).unwrap(), 0);
        }
    }

    /// Spec: OSDev FDC / Intel 82077AA — DOR release + Sense Interrupt via MachineBus.
    #[test]
    fn machine_bus_fdc_dor_msr_fifo() {
        let mut m = Machine::new(64 * 1024);
        assert!(!Fdc82077::owns_port(0x3F6)); // IDE alt/control, not FDC
        {
            let mut bus = m.bus_mut();
            assert_eq!(bus.port_in_u8(FDC_MSR).unwrap(), 0);
            bus.port_out_u8(FDC_DOR, FDC_DOR_RESET_N).unwrap();
            assert_eq!(bus.port_in_u8(FDC_MSR).unwrap(), FDC_MSR_RQM);
            // Sense Interrupt Status (0x08) → ST0 then PCN; MSR RQM|DIO in result.
            bus.port_out_u8(FDC_FIFO, FDC_CMD_SENSE_INT).unwrap();
            assert_eq!(bus.port_in_u8(FDC_MSR).unwrap(), FDC_MSR_RQM | FDC_MSR_DIO);
            assert_eq!(bus.port_in_u8(FDC_FIFO).unwrap(), 0xC0); // ST0 IC=11 | US=0
            assert_eq!(bus.port_in_u8(FDC_FIFO).unwrap(), 0x00); // PCN stub
            assert_eq!(bus.port_in_u8(FDC_MSR).unwrap(), FDC_MSR_RQM);
            // 0x3F6 remains IDE (write device control); must not disturb FDC DOR.
            bus.port_out_u8(0x3F6, 0x02).unwrap();
        }
        assert_eq!(m.fdc.dor, FDC_DOR_RESET_N);
        m.reset();
        assert_eq!(m.fdc.dor, 0);
    }

    /// Spec: Intel 82077AA Specify — two params via MachineBus; no result phase.
    #[test]
    fn machine_bus_fdc_specify() {
        let mut m = Machine::new(64 * 1024);
        {
            let mut bus = m.bus_mut();
            bus.port_out_u8(FDC_DOR, FDC_DOR_RESET_N).unwrap();
            bus.port_out_u8(FDC_FIFO, FDC_CMD_SPECIFY).unwrap();
            assert_eq!(bus.port_in_u8(FDC_MSR).unwrap(), FDC_MSR_RQM);
            bus.port_out_u8(FDC_FIFO, 0xCF).unwrap();
            bus.port_out_u8(FDC_FIFO, 0x02).unwrap();
            assert_eq!(bus.port_in_u8(FDC_MSR).unwrap(), FDC_MSR_RQM);
        }
        assert_eq!(m.fdc.specify_srt_hut, 0xCF);
        assert_eq!(m.fdc.specify_hlt_nd, 0x02);
    }

    /// Spec: Intel 82077AA / OSDev Configure — three params via MachineBus; no result/IRQ.
    #[test]
    fn machine_bus_fdc_configure() {
        let mut m = Machine::new(64 * 1024);
        {
            let mut bus = m.bus_mut();
            bus.port_out_u8(FDC_DOR, FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ)
                .unwrap();
            bus.port_out_u8(FDC_FIFO, FDC_CMD_CONFIGURE).unwrap();
            assert_eq!(bus.port_in_u8(FDC_MSR).unwrap(), FDC_MSR_RQM);
            bus.port_out_u8(FDC_FIFO, 0x00).unwrap();
            bus.port_out_u8(FDC_FIFO, 0x57).unwrap();
            bus.port_out_u8(FDC_FIFO, 0x00).unwrap();
            assert_eq!(bus.port_in_u8(FDC_MSR).unwrap(), FDC_MSR_RQM);
            assert_eq!(
                bus.poll_external_irq(),
                None,
                "Configure must not assert IRQ6"
            );
        }
        assert_eq!(m.fdc.configure_byte0, 0x00);
        assert_eq!(m.fdc.configure_eis_fifo_poll_thr, 0x57);
        assert_eq!(m.fdc.configure_pretrk, 0x00);
    }

    /// Spec: Intel 82077AA Recalibrate — unit param → PCN=0, Seek End ST0, IRQ6 via bus.
    #[test]
    fn machine_bus_fdc_recalibrate() {
        let mut m = Machine::new(64 * 1024);
        init_at_pic_unmask_irq6(&mut m);
        {
            let mut bus = m.bus_mut();
            bus.port_out_u8(FDC_DOR, FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ)
                .unwrap();
            bus.port_out_u8(FDC_FIFO, FDC_CMD_RECALIBRATE).unwrap();
            assert_eq!(bus.port_in_u8(FDC_MSR).unwrap(), FDC_MSR_RQM);
            bus.port_out_u8(FDC_FIFO, 0x01).unwrap();
            assert_eq!(bus.port_in_u8(FDC_MSR).unwrap(), FDC_MSR_RQM);
            // Spec: AT master ICW2 base 0x08 → IRQ6 vector 0x0E.
            assert_eq!(bus.poll_external_irq(), Some(0x0E));
            bus.port_out_u8(FDC_FIFO, FDC_CMD_SENSE_INT).unwrap();
            assert_eq!(bus.port_in_u8(FDC_FIFO).unwrap(), FDC_ST0_SEEK_END | 0x01);
            assert_eq!(bus.port_in_u8(FDC_FIFO).unwrap(), 0x00);
            assert_eq!(bus.poll_external_irq(), None);
        }
        assert_eq!(m.fdc.pcn[1], 0);
        m.pic.port_write(PIC_MASTER_CMD, 1, 0x20);
    }

    /// Spec: Intel 82077AA Seek — HD|US + NCN → PCN=NCN, Seek End ST0, IRQ6 via bus.
    #[test]
    fn machine_bus_fdc_seek() {
        let mut m = Machine::new(64 * 1024);
        init_at_pic_unmask_irq6(&mut m);
        {
            let mut bus = m.bus_mut();
            bus.port_out_u8(FDC_DOR, FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ)
                .unwrap();
            bus.port_out_u8(FDC_FIFO, FDC_CMD_SEEK).unwrap();
            assert_eq!(bus.port_in_u8(FDC_MSR).unwrap(), FDC_MSR_RQM);
            bus.port_out_u8(FDC_FIFO, 0x01).unwrap(); // HD=0 | US=1
            bus.port_out_u8(FDC_FIFO, 0x14).unwrap(); // NCN
            assert_eq!(bus.port_in_u8(FDC_MSR).unwrap(), FDC_MSR_RQM);
            // Spec: AT master ICW2 base 0x08 → IRQ6 vector 0x0E.
            assert_eq!(bus.poll_external_irq(), Some(0x0E));
            bus.port_out_u8(FDC_FIFO, FDC_CMD_SENSE_INT).unwrap();
            assert_eq!(bus.port_in_u8(FDC_FIFO).unwrap(), FDC_ST0_SEEK_END | 0x01);
            assert_eq!(bus.port_in_u8(FDC_FIFO).unwrap(), 0x14);
            assert_eq!(bus.poll_external_irq(), None);
        }
        assert_eq!(m.fdc.pcn[1], 0x14);
        m.pic.port_write(PIC_MASTER_CMD, 1, 0x20);
    }

    /// Spec: Intel 82077AA §5.2.5 Sense Drive Status — HD|US param → ST3 result
    /// via bus; no execution phase, no IRQ.
    #[test]
    fn machine_bus_fdc_sense_drive_status() {
        let mut m = Machine::new(64 * 1024);
        {
            let mut bus = m.bus_mut();
            bus.port_out_u8(FDC_DOR, FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ)
                .unwrap();
            bus.port_out_u8(FDC_FIFO, FDC_CMD_SENSE_DRIVE_STATUS)
                .unwrap();
            assert_eq!(bus.port_in_u8(FDC_MSR).unwrap(), FDC_MSR_RQM);
            bus.port_out_u8(FDC_FIFO, 0x06).unwrap(); // HD=1 | US=2
            assert_eq!(bus.port_in_u8(FDC_MSR).unwrap(), FDC_MSR_RQM | FDC_MSR_DIO);
            let st3 = bus.port_in_u8(FDC_FIFO).unwrap();
            assert_eq!(
                st3,
                FDC_ST3_TRACK0 | FDC_ST3_RESERVED_BIT3 | FDC_ST3_RESERVED_BIT5 | 0x06,
                "T0 (pcn==0) | reserved bits | HD|US"
            );
            assert_eq!(bus.port_in_u8(FDC_MSR).unwrap(), FDC_MSR_RQM);
            assert_eq!(
                bus.poll_external_irq(),
                None,
                "Sense Drive Status has no IRQ"
            );
        }
    }

    /// Spec: Intel 82077AA + OSDev FDC + IBM PC AT — assert_irq6 → IRQ6 → vector 0x0E.
    #[test]
    fn fdc_assert_irq6_via_poll_external_irq() {
        let mut m = Machine::new(64 * 1024);
        init_at_pic_unmask_irq6(&mut m);
        m.fdc
            .port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ));
        m.fdc.assert_irq6();
        assert!(m.fdc.irq_line());
        {
            let mut bus = m.bus_mut();
            // Spec: AT master ICW2 base 0x08 → IRQ6 vector 0x0E.
            assert_eq!(bus.poll_external_irq(), Some(0x0E));
            assert_eq!(bus.poll_external_irq(), None);
        }
        m.fdc.clear_irq6();
        assert!(!m.fdc.irq_line());
        m.pic.port_write(PIC_MASTER_CMD, 1, 0x20);
    }

    /// Spec: Intel 82077AA DOR bit3 — without DMA/IRQ enable, assert does not deliver IRQ6.
    #[test]
    fn fdc_dor_dma_irq_masks_irq6_on_machine_bus() {
        let mut m = Machine::new(64 * 1024);
        init_at_pic_unmask_irq6(&mut m);
        m.fdc.port_write(FDC_DOR, 1, u32::from(FDC_DOR_RESET_N)); // no DMA/IRQ bit
        m.fdc.assert_irq6();
        assert!(!m.fdc.irq_line());
        {
            let mut bus = m.bus_mut();
            assert_eq!(bus.poll_external_irq(), None);
        }
    }

    /// DOR: nRESET | DMA/IRQ | motor0 (classic SeaBIOS-style enable).
    const FDC_DOR_MOTOR0_DMA: u8 = FDC_DOR_RESET_N | FDC_DOR_DMA_IRQ | 0x10;

    /// Spec: IBM PC 1.44MB — [`Machine::attach_floppy_image`] rejects wrong size
    /// and [`Machine::with_floppy`] attaches exact-size media.
    #[test]
    fn machine_attach_floppy_image_and_with_floppy_helpers() {
        let mut m = Machine::new(64 * 1024);
        assert!(!m.fdc.has_media());
        assert!(m.attach_floppy_image(vec![0u8; 512]).is_err());
        assert!(!m.fdc.has_media());
        m.attach_floppy_image(vec![0xAAu8; FDC_1440_IMAGE_SIZE])
            .expect("exact 1.44MB");
        assert!(m.fdc.has_media());

        assert!(Machine::with_floppy(64 * 1024, vec![0u8; 512]).is_err());
        let m2 = Machine::with_floppy(64 * 1024, vec![0x55u8; FDC_1440_IMAGE_SIZE])
            .expect("with_floppy exact 1.44MB");
        assert!(m2.fdc.has_media());
    }

    /// Spec: Intel 82077AA DMA mode + 8237A Write + OSDev ISA DMA floppy ch2 —
    /// MachineBus FDC FIFO READ DATA completion with media auto-wires
    /// `dma_transfer(2)` Write (device→memory): 512 sector bytes land in PhysMem
    /// at the programmed DMA address and TC latches.
    ///
    /// Media is attached via [`Machine::with_floppy`] (wraps
    /// [`Machine::attach_floppy_image`] → [`Fdc82077::attach_image`]).
    #[test]
    fn machine_bus_fdc_read_data_dma_ch2_writes_sector_into_physmem() {
        let mut img = vec![0u8; FDC_1440_IMAGE_SIZE];
        for (i, b) in img[..FDC_SECTOR_SIZE].iter_mut().enumerate() {
            *b = (i & 0xFF) as u8;
        }
        let mut m = Machine::with_floppy(256 * 1024, img).expect("1.44MB image");
        // Program ISA DMA ch2: page/addr, count=511 (512 bytes), Write|Single|Inc.
        program_dma_ch2_write(&mut m, 0x01, 0x1000, 511); // @ phys 0x1_1000
                                                          // Poison destination so we detect a real DMA write.
        for i in 0..FDC_SECTOR_SIZE {
            m.mem.write_u8(0x1_1000 + i as u64, 0xEE).unwrap();
        }

        {
            let mut bus = m.bus_mut();
            bus.port_out_u8(FDC_DOR, FDC_DOR_MOTOR0_DMA).unwrap();
            // READ DATA MFM: C/H/R/N = 0/0/1/2, EOT=R (single-sector).
            bus.port_out_u8(FDC_FIFO, FDC_CMD_MFM | FDC_CMD_READ_DATA)
                .unwrap();
            for p in [0x00u8, 0x00, 0x00, 0x01, 0x02, 0x01, 0x1B, 0xFF] {
                bus.port_out_u8(FDC_FIFO, p).unwrap();
            }
        }

        for i in 0..FDC_SECTOR_SIZE {
            assert_eq!(
                m.mem.read_u8(0x1_1000 + i as u64).unwrap(),
                (i & 0xFF) as u8,
                "PhysMem[{:#x}] after FDC DMA ch2",
                0x1_1000 + i
            );
        }
        assert_eq!(m.dma.master.channels[2].count, 0xFFFF);
        assert_eq!(
            m.dma.port_read(0x08, 1) as u8 & 0x0F,
            0x04,
            "TC latched for ch2"
        );
        assert!(
            m.fdc.last_sector().is_some(),
            "inspection latch remains after DMA"
        );
        assert!(
            m.fdc.take_pending_dma_sector().is_none(),
            "pending arm consumed by auto-wire"
        );
    }

    /// Spec: Intel 82077AA §5.1.1 / Table 5-1 READ DATA multi-sector (MT=0
    /// same head, EOT=R+1) + Intel 8237A Write + OSDev ISA DMA floppy ch2 —
    /// MachineBus auto-wire copies the concatenated 2×512 latch into PhysMem
    /// when ch2 Word Count is programmed for the full transfer.
    ///
    /// Media via [`Machine::with_floppy`] / [`Machine::attach_floppy_image`].
    #[test]
    fn machine_bus_fdc_read_data_multi_sector_dma_ch2_writes_into_physmem() {
        let mut img = vec![0u8; FDC_1440_IMAGE_SIZE];
        // Sector R=1 @ 0: 0x00..; sector R=2 @ 512: 0x80..
        for (i, b) in img[..FDC_SECTOR_SIZE].iter_mut().enumerate() {
            *b = (i & 0xFF) as u8;
        }
        for (i, b) in img[FDC_SECTOR_SIZE..2 * FDC_SECTOR_SIZE]
            .iter_mut()
            .enumerate()
        {
            *b = 0x80 | ((i & 0x7F) as u8);
        }
        let mut m = Machine::with_floppy(256 * 1024, img).expect("1.44MB image");
        let xfer_len = 2 * FDC_SECTOR_SIZE;
        // count = N−1 → 1023 for 1024 bytes @ phys 0x1_1000
        program_dma_ch2_write(&mut m, 0x01, 0x1000, (xfer_len - 1) as u16);
        for i in 0..xfer_len {
            m.mem.write_u8(0x1_1000 + i as u64, 0xEE).unwrap();
        }

        {
            let mut bus = m.bus_mut();
            bus.port_out_u8(FDC_DOR, FDC_DOR_MOTOR0_DMA).unwrap();
            // READ DATA MFM MT=0: C/H/R/N/EOT = 0/0/1/2/2 — two sectors.
            bus.port_out_u8(FDC_FIFO, FDC_CMD_MFM | FDC_CMD_READ_DATA)
                .unwrap();
            for p in [0x00u8, 0x00, 0x00, 0x01, 0x02, 0x02, 0x1B, 0xFF] {
                bus.port_out_u8(FDC_FIFO, p).unwrap();
            }
        }

        assert_eq!(
            m.fdc.last_sector_byte_count(),
            xfer_len,
            "device latch concatenates R..=EOT"
        );
        for i in 0..FDC_SECTOR_SIZE {
            assert_eq!(
                m.mem.read_u8(0x1_1000 + i as u64).unwrap(),
                (i & 0xFF) as u8,
                "PhysMem sector1[{i}]"
            );
        }
        for i in 0..FDC_SECTOR_SIZE {
            assert_eq!(
                m.mem
                    .read_u8(0x1_1000 + FDC_SECTOR_SIZE as u64 + i as u64)
                    .unwrap(),
                0x80 | ((i & 0x7F) as u8),
                "PhysMem sector2[{i}]"
            );
        }
        assert_eq!(m.dma.master.channels[2].count, 0xFFFF);
        assert_eq!(
            m.dma.port_read(0x08, 1) as u8 & 0x0F,
            0x04,
            "TC latched for ch2"
        );
        assert!(
            m.fdc.take_pending_dma_sector().is_none(),
            "pending arm consumed by auto-wire"
        );
    }

    /// Spec: Intel 82077AA TC (§4.2.5) + §6.2 ST1 EN + Intel 8237A TC —
    /// MachineBus auto-wire: ch2 Word Count covers only the first of two READ
    /// DATA sectors → PhysMem gets 512 bytes, TC latches, FDC result is
    /// abnormal + ST1 EN with ENDaddress R=1 (documented early-stop model).
    #[test]
    fn machine_bus_fdc_read_data_dma_tc_early_stop_st1_en() {
        let mut img = vec![0u8; FDC_1440_IMAGE_SIZE];
        for (i, b) in img[..FDC_SECTOR_SIZE].iter_mut().enumerate() {
            *b = (i & 0xFF) as u8;
        }
        for (i, b) in img[FDC_SECTOR_SIZE..2 * FDC_SECTOR_SIZE]
            .iter_mut()
            .enumerate()
        {
            *b = 0x80 | ((i & 0x7F) as u8);
        }
        let mut m = Machine::with_floppy(256 * 1024, img).expect("1.44MB image");
        // DMA count = 511 → 512 bytes only (FDC pending = 1024).
        program_dma_ch2_write(&mut m, 0x01, 0x1000, 511);
        for i in 0..(2 * FDC_SECTOR_SIZE) {
            m.mem.write_u8(0x1_1000 + i as u64, 0xEE).unwrap();
        }

        {
            let mut bus = m.bus_mut();
            bus.port_out_u8(FDC_DOR, FDC_DOR_MOTOR0_DMA).unwrap();
            bus.port_out_u8(FDC_FIFO, FDC_CMD_MFM | FDC_CMD_READ_DATA)
                .unwrap();
            for p in [0x00u8, 0x00, 0x00, 0x01, 0x02, 0x02, 0x1B, 0xFF] {
                bus.port_out_u8(FDC_FIFO, p).unwrap();
            }
        }

        for i in 0..FDC_SECTOR_SIZE {
            assert_eq!(
                m.mem.read_u8(0x1_1000 + i as u64).unwrap(),
                (i & 0xFF) as u8,
                "first sector DMA'd"
            );
        }
        // Bytes beyond TC must remain poison.
        for i in FDC_SECTOR_SIZE..(2 * FDC_SECTOR_SIZE) {
            assert_eq!(
                m.mem.read_u8(0x1_1000 + i as u64).unwrap(),
                0xEE,
                "past-TC PhysMem untouched"
            );
        }
        assert_eq!(m.dma.master.status & 0x0F, 0x04, "TC latched");
        assert_eq!(m.fdc.last_sector_byte_count(), FDC_SECTOR_SIZE);

        {
            let mut bus = m.bus_mut();
            let st0 = bus.port_in_u8(FDC_FIFO).unwrap();
            assert_eq!(st0 & FDC_ST0_IC_ABNORMAL, FDC_ST0_IC_ABNORMAL);
            assert_eq!(bus.port_in_u8(FDC_FIFO).unwrap(), FDC_ST1_EN);
            assert_eq!(bus.port_in_u8(FDC_FIFO).unwrap(), 0x00); // ST2
            assert_eq!(bus.port_in_u8(FDC_FIFO).unwrap(), 0x00); // C
            assert_eq!(bus.port_in_u8(FDC_FIFO).unwrap(), 0x00); // H
            assert_eq!(bus.port_in_u8(FDC_FIFO).unwrap(), 0x01); // R
            assert_eq!(bus.port_in_u8(FDC_FIFO).unwrap(), 0x02); // N
        }
    }

    /// Spec: Intel 82077AA TC + §6.2 EN + 8237A — WRITE DATA auto-wire with
    /// DMA Word Count for one sector while EOT requests two: only sector 1 is
    /// written; result ST1 EN / ENDaddress R=1.
    #[test]
    fn machine_bus_fdc_write_data_dma_tc_early_stop_st1_en() {
        let mut m = Machine::new(256 * 1024);
        m.fdc
            .attach_image(vec![0xAAu8; FDC_1440_IMAGE_SIZE])
            .expect("1.44MB image");
        program_dma_ch2_read(&mut m, 0x01, 0x1000, 511); // 512 bytes only
        for i in 0..FDC_SECTOR_SIZE {
            m.mem
                .write_u8(0x1_1000 + i as u64, (i & 0xFF) as u8)
                .unwrap();
        }

        {
            let mut bus = m.bus_mut();
            bus.port_out_u8(FDC_DOR, FDC_DOR_MOTOR0_DMA).unwrap();
            bus.port_out_u8(FDC_FIFO, FDC_CMD_MFM | FDC_CMD_WRITE_DATA)
                .unwrap();
            for p in [0x00u8, 0x00, 0x00, 0x01, 0x02, 0x02, 0x1B, 0xFF] {
                bus.port_out_u8(FDC_FIFO, p).unwrap();
            }
        }

        let s1 = m.fdc.read_sector(0, 0, 1).expect("s1");
        for (i, &b) in s1.iter().enumerate() {
            assert_eq!(b, (i & 0xFF) as u8, "sector1 from short DMA");
        }
        assert!(
            m.fdc
                .read_sector(0, 0, 2)
                .expect("s2")
                .iter()
                .all(|&b| b == 0xAA),
            "sector2 untouched on early TC"
        );
        assert_eq!(m.dma.master.status & 0x0F, 0x04, "TC latched");

        {
            let mut bus = m.bus_mut();
            let st0 = bus.port_in_u8(FDC_FIFO).unwrap();
            assert_eq!(st0 & FDC_ST0_IC_ABNORMAL, FDC_ST0_IC_ABNORMAL);
            assert_eq!(bus.port_in_u8(FDC_FIFO).unwrap(), FDC_ST1_EN);
            let _ = bus.port_in_u8(FDC_FIFO).unwrap(); // ST2
            assert_eq!(bus.port_in_u8(FDC_FIFO).unwrap(), 0x00);
            assert_eq!(bus.port_in_u8(FDC_FIFO).unwrap(), 0x00);
            assert_eq!(bus.port_in_u8(FDC_FIFO).unwrap(), 0x01);
            assert_eq!(bus.port_in_u8(FDC_FIFO).unwrap(), 0x02);
        }
    }

    /// Spec: 82077AA — no media → ND result, no `last_sector`, MachineBus must
    /// not call DMA / must leave PhysMem at the DMA address unchanged.
    #[test]
    fn machine_bus_fdc_read_data_no_media_skips_dma_ch2() {
        let mut m = Machine::new(256 * 1024);
        assert!(!m.fdc.has_media());
        program_dma_ch2_write(&mut m, 0x01, 0x2000, 511); // @ phys 0x1_2000
        for i in 0..FDC_SECTOR_SIZE {
            m.mem.write_u8(0x1_2000 + i as u64, 0xA5).unwrap();
        }

        {
            let mut bus = m.bus_mut();
            bus.port_out_u8(FDC_DOR, FDC_DOR_MOTOR0_DMA).unwrap();
            bus.port_out_u8(FDC_FIFO, FDC_CMD_MFM | FDC_CMD_READ_DATA)
                .unwrap();
            for p in [0x00u8, 0x00, 0x00, 0x01, 0x02, 0x12, 0x1B, 0xFF] {
                bus.port_out_u8(FDC_FIFO, p).unwrap();
            }
        }

        for i in 0..FDC_SECTOR_SIZE {
            assert_eq!(
                m.mem.read_u8(0x1_2000 + i as u64).unwrap(),
                0xA5,
                "no-media must not DMA into PhysMem"
            );
        }
        assert!(m.fdc.last_sector().is_none());
        // Channel still programmed / unmasked — TC not latched (no transfer).
        assert_eq!(m.dma.master.status & 0x0F, 0);
        assert_eq!(m.dma.master.channels[2].count, 511);
    }

    /// Spec: Intel 82077AA DMA mode + 8237A Read + OSDev ISA DMA floppy ch2 —
    /// MachineBus FDC FIFO WRITE DATA completion with media auto-wires
    /// `dma_transfer(2)` Read (memory→device): 512 PhysMem bytes land in the
    /// floppy image via `write_sector` / `last_write` and TC latches.
    /// EOT==R → single-sector (multi-sector uses `pending_dma_write_byte_count`).
    #[test]
    fn machine_bus_fdc_write_data_dma_ch2_reads_sector_from_physmem() {
        let mut m = Machine::new(256 * 1024);
        m.fdc
            .attach_image(vec![0xAAu8; FDC_1440_IMAGE_SIZE])
            .expect("1.44MB image");
        // Program ISA DMA ch2: page/addr, count=511 (512 bytes), Read|Single|Inc.
        program_dma_ch2_read(&mut m, 0x01, 0x1000, 511); // @ phys 0x1_1000
        for i in 0..FDC_SECTOR_SIZE {
            m.mem
                .write_u8(0x1_1000 + i as u64, (i & 0xFF) as u8)
                .unwrap();
        }

        {
            let mut bus = m.bus_mut();
            bus.port_out_u8(FDC_DOR, FDC_DOR_MOTOR0_DMA).unwrap();
            // WRITE DATA MFM: C/H/R/N/EOT = 0/0/1/2/1 — single-sector, no pre-latch.
            bus.port_out_u8(FDC_FIFO, FDC_CMD_MFM | FDC_CMD_WRITE_DATA)
                .unwrap();
            for p in [0x00u8, 0x00, 0x00, 0x01, 0x02, 0x01, 0x1B, 0xFF] {
                bus.port_out_u8(FDC_FIFO, p).unwrap();
            }
        }

        let written = m
            .fdc
            .last_write()
            .expect("Machine DMA commit latches last_write")
            .to_vec();
        assert_eq!(written.len(), FDC_SECTOR_SIZE);
        for (i, b) in written.iter().enumerate() {
            assert_eq!(*b, (i & 0xFF) as u8, "last_write[{i}]");
        }
        let image_sector = m
            .fdc
            .read_sector(0, 0, 1)
            .expect("image must contain DMA-written sector");
        assert_eq!(image_sector.as_slice(), written.as_slice());
        assert_eq!(m.dma.master.channels[2].count, 0xFFFF);
        assert_eq!(
            m.dma.port_read(0x08, 1) as u8 & 0x0F,
            0x04,
            "TC latched for ch2"
        );
        assert!(
            !m.fdc.take_pending_dma_write(),
            "pending arm consumed by auto-wire"
        );
    }

    /// Spec: 82077AA — no media → NW result, no `last_write`, MachineBus must
    /// not DMA-read / must leave PhysMem at the DMA address unchanged and must
    /// not latch TC.
    #[test]
    fn machine_bus_fdc_write_data_no_media_skips_dma_ch2() {
        let mut m = Machine::new(256 * 1024);
        assert!(!m.fdc.has_media());
        program_dma_ch2_read(&mut m, 0x01, 0x2000, 511); // @ phys 0x1_2000
        for i in 0..FDC_SECTOR_SIZE {
            m.mem.write_u8(0x1_2000 + i as u64, 0x5A).unwrap();
        }

        {
            let mut bus = m.bus_mut();
            bus.port_out_u8(FDC_DOR, FDC_DOR_MOTOR0_DMA).unwrap();
            bus.port_out_u8(FDC_FIFO, FDC_CMD_MFM | FDC_CMD_WRITE_DATA)
                .unwrap();
            for p in [0x00u8, 0x00, 0x00, 0x01, 0x02, 0x12, 0x1B, 0xFF] {
                bus.port_out_u8(FDC_FIFO, p).unwrap();
            }
        }

        for i in 0..FDC_SECTOR_SIZE {
            assert_eq!(
                m.mem.read_u8(0x1_2000 + i as u64).unwrap(),
                0x5A,
                "no-media must not consume DMA source buffer"
            );
        }
        assert!(m.fdc.last_write().is_none());
        assert!(!m.fdc.take_pending_dma_write());
        assert_eq!(m.dma.master.status & 0x0F, 0);
        assert_eq!(m.dma.master.channels[2].count, 511);
    }

    /// Spec: QEMU fw_cfg — MachineBus wires `0x510`/`0x511`; signature `QEMU`.
    #[test]
    fn machine_bus_fw_cfg_signature() {
        let mut m = Machine::new(64 * 1024);
        let mut bus = m.bus_mut();
        bus.port_out_u16(FW_CFG_SELECTOR, FW_CFG_SIGNATURE).unwrap();
        let mut sig = [0u8; 4];
        for b in &mut sig {
            *b = bus.port_in_u8(FW_CFG_DATA).unwrap();
        }
        assert_eq!(&sig, FW_CFG_SIGNATURE_BYTES);
    }

    /// Spec: QEMU fw_cfg — ID 0x0001 is LE32 (base revision only here);
    /// RAM_SIZE 0x0003 is the configured byte count as LE64.
    #[test]
    fn machine_bus_fw_cfg_id_ram_size_reset_and_unknown_selector() {
        const RAM_SIZE: usize = 16 * 1024 * 1024;
        const FW_CFG_ID_SELECTOR: u16 = 0x0001;
        const FW_CFG_RAM_SIZE_SELECTOR: u16 = 0x0003;
        let mut m = Machine::new(RAM_SIZE);

        {
            let mut bus = m.bus_mut();
            bus.port_out_u16(FW_CFG_SELECTOR, FW_CFG_ID_SELECTOR)
                .unwrap();
            let mut id = [0u8; 4];
            for byte in &mut id {
                *byte = bus.port_in_u8(FW_CFG_DATA).unwrap();
            }
            // Base revision + DMA interface (the machine services `0x514`).
            assert_eq!(id, [0x03, 0x00, 0x00, 0x00]);

            bus.port_out_u16(FW_CFG_SELECTOR, FW_CFG_RAM_SIZE_SELECTOR)
                .unwrap();
            let mut ram_size = [0u8; 8];
            for byte in &mut ram_size {
                *byte = bus.port_in_u8(FW_CFG_DATA).unwrap();
            }
            assert_eq!(ram_size, [0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00]);

            bus.port_out_u16(FW_CFG_SELECTOR, 0x00FF).unwrap();
            assert_eq!(bus.port_in_u8(FW_CFG_DATA).unwrap(), 0);

            bus.port_out_u16(FW_CFG_SELECTOR, FW_CFG_RAM_SIZE_SELECTOR)
                .unwrap();
            let _ = bus.port_in_u8(FW_CFG_DATA).unwrap();
        }
        assert_eq!(m.fw_cfg.offset(), 1);

        m.reset();

        assert_eq!(m.fw_cfg.selector(), 0);
        assert_eq!(m.fw_cfg.offset(), 0);
        let mut bus = m.bus_mut();
        bus.port_out_u16(FW_CFG_SELECTOR, FW_CFG_ID_SELECTOR)
            .unwrap();
        let mut id = [0u8; 4];
        for byte in &mut id {
            *byte = bus.port_in_u8(FW_CFG_DATA).unwrap();
        }
        assert_eq!(id, [0x03, 0x00, 0x00, 0x00]);

        bus.port_out_u16(FW_CFG_SELECTOR, FW_CFG_RAM_SIZE_SELECTOR)
            .unwrap();
        let mut ram_size = [0u8; 8];
        for byte in &mut ram_size {
            *byte = bus.port_in_u8(FW_CFG_DATA).unwrap();
        }
        assert_eq!(ram_size, (RAM_SIZE as u64).to_le_bytes());
    }
}
