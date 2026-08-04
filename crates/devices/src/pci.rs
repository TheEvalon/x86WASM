//! PCI configuration mechanism #1 stub — ports `0xCF8` / `0xCFC`–`0xCFF`.
//!
//! Classic PC Type 1 configuration access: latch a bus/device/function/register
//! address in `CONFIG_ADDRESS`, then read/write `CONFIG_DATA`.
//!
//! # Spec refs
//!
//! - PCI Local Bus Specification — Configuration Mechanism #1 (`CONFIG_ADDRESS`
//!   at `0xCF8`, `CONFIG_DATA` at `0xCFC`; enable bit 31; Type 1 address fields);
//!   Type 0 config header vendor/device/class/header-type; multi-function bit.
//! - OSDev Wiki PCI — Configuration Space Access Mechanism #1; absent device
//!   reads return `0xFFFFFFFF`.
//! - Intel 440FX (i440FX) host bridge identity (`vendor 0x8086`, `device 0x1237`).
//! - Intel 82371SB (PIIX3) PCI function IDs used as classic pc-i440fx-compatible
//!   stubs: ISA bridge `8086:7000`, IDE `8086:7010`, USB UHCI `8086:7020` at
//!   `00:01.0` / `00:01.1` / `00:01.2` (behavior from public device IDs / PCI
//!   class codes — not copied source).
//! - Intel 82371AB (PIIX4) ACPI function public ID `8086:7113` at `00:01.3`
//!   (classic QEMU/SeaBIOS-compatible stub identity; class bridge/other `0x0680`).
//! - `docs/machine-model-pc-v1.md`, `plan.md` §15.2 / §21 PCI.
//!
//! # Scope (this slice)
//!
//! - Type 1 address latch (enable / bus / device / function / register).
//! - Host bridge at `00:00.0` with Intel-style vendor/device/class/header type 0.
//! - PIIX-style stubs: `00:01.0` ISA bridge (multi-function), `00:01.1` IDE,
//!   `00:01.2` USB UHCI, `00:01.3` ACPI identity only.
//! - Absent devices: `0xFFFFFFFF` when enable is set.
//! - Enable bit clear: data-port reads return `0xFFFFFFFF` (open-bus style).
//! - Byte/word/dword access via `0xCFC` + offset.
//!
//! # Unsupported (explicit)
//!
//! - BAR MMIO/IO decode, bus mastering, INTx routing tables
//! - USB host controller (UHCI frame list / ports / IRQ)
//! - ACPI PM I/O block / SMI / GPE / ACPI tables (config identity only)
//! - Capability lists, MSI, PCIe, hotplug
//! - IDE BARs tied to `IdePrimary` ports (legacy fixed ports remain)

use crate::PortDevice;

/// CONFIG_ADDRESS (Type 1). Spec: PCI Local Bus — Mechanism #1.
pub const PCI_CONFIG_ADDRESS: u16 = 0xCF8;
/// CONFIG_DATA base (bytes `0xCFC`–`0xCFF`).
pub const PCI_CONFIG_DATA: u16 = 0xCFC;

/// Intel vendor ID.
pub const PCI_VENDOR_INTEL: u16 = 0x8086;
/// i440FX-class host bridge device ID (stub identity).
pub const PCI_DEVICE_I440FX: u16 = 0x1237;
/// PIIX3 ISA bridge device ID (82371SB).
pub const PCI_DEVICE_PIIX3_ISA: u16 = 0x7000;
/// PIIX3 IDE controller device ID (82371SB).
pub const PCI_DEVICE_PIIX3_IDE: u16 = 0x7010;
/// PIIX3 USB UHCI controller device ID (82371SB).
pub const PCI_DEVICE_PIIX3_USB: u16 = 0x7020;
/// PIIX4 ACPI controller device ID (82371AB) — classic pc stub at `00:01.3`.
pub const PCI_DEVICE_PIIX_ACPI: u16 = 0x7113;
/// PCI class: Bridge device.
pub const PCI_CLASS_BRIDGE: u8 = 0x06;
/// PCI subclass: Host/PCI bridge.
pub const PCI_SUBCLASS_HOST_BRIDGE: u8 = 0x00;
/// PCI subclass: ISA bridge.
pub const PCI_SUBCLASS_ISA_BRIDGE: u8 = 0x01;
/// PCI subclass: Other bridge device (PIIX ACPI class `0x0680`).
pub const PCI_SUBCLASS_OTHER_BRIDGE: u8 = 0x80;
/// PCI class: Mass storage.
pub const PCI_CLASS_STORAGE: u8 = 0x01;
/// PCI subclass: IDE controller.
pub const PCI_SUBCLASS_IDE: u8 = 0x01;
/// PCI class: Serial bus controller.
pub const PCI_CLASS_SERIAL_BUS: u8 = 0x0C;
/// PCI subclass: USB controller.
pub const PCI_SUBCLASS_USB: u8 = 0x03;
/// UHCI programming interface (PCI class code prog IF).
pub const PCI_PROG_IF_UHCI: u8 = 0x00;
/// Header type multi-function bit.
pub const PCI_HEADER_MULTIFUNCTION: u8 = 0x80;
/// PIIX IDE Bus Master IDE Base Address Register (BMIBA) config offset.
/// Spec: Intel 82371SB — PCI config dword at `0x20` is an I/O BAR (bit0=1).
pub const PCI_PIIX_IDE_BMIBA_OFFSET: u8 = 0x20;
/// BMIBA I/O space indicator bit (PCI I/O BAR bit0).
pub const PCI_BAR_IO_SPACE: u32 = 0x01;
/// BMIBA size decode mask — 16-byte aligned I/O base (bits 15:4); low nibble
/// forced to `0001` (I/O space). Spec: PCI I/O BAR + PIIX BMIBA.
pub const PCI_PIIX_IDE_BMIBA_MASK: u32 = 0xFFF0;
/// PIIX USB UHCI BAR0 config offset (I/O space).
/// Spec: Intel 82371SB — UHCI I/O BAR at PCI config `0x20` (bit0=1).
pub const PCI_PIIX_USB_BAR0_OFFSET: u8 = 0x20;
/// UHCI BAR0 size decode mask — 32-byte aligned I/O base (bits 15:5).
/// Spec: PCI I/O BAR + UHCI I/O footprint (32 bytes).
pub const PCI_PIIX_USB_BAR0_MASK: u32 = 0xFFE0;
/// PIIX ISA PIRQ route control registers (PIRQRC[A:D]) config offsets `0x60`–`0x63`.
/// Spec: Intel 82371SB — each byte defaults to `0x80` (route disabled).
pub const PCI_PIIX_ISA_PIRQRC_OFFSET: u8 = 0x60;
/// Default PIRQRC byte value (IRQ routing disabled).
pub const PCI_PIIX_ISA_PIRQRC_DEFAULT: u8 = 0x80;

/// Enable bit in CONFIG_ADDRESS (bit 31).
const ADDR_ENABLE: u32 = 1 << 31;

/// Type-0 config header identity fields written at reset.
struct PciHeaderId {
    vendor: u16,
    device: u16,
    revision: u8,
    prog_if: u8,
    subclass: u8,
    class: u8,
    header_type: u8,
}

/// PCI configuration mechanism #1 controller stub.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PciConfig {
    /// Latched CONFIG_ADDRESS value (bits 1:0 forced clear on write).
    pub address: u32,
    /// 256-byte config space for the host bridge at `00:00.0`.
    host_bridge: [u8; 256],
    /// PIIX3 ISA bridge at `00:01.0` (multi-function).
    piix_isa: [u8; 256],
    /// PIIX3 IDE at `00:01.1`.
    piix_ide: [u8; 256],
    /// PIIX3 USB UHCI at `00:01.2` (identity stub only).
    piix_usb: [u8; 256],
    /// PIIX ACPI at `00:01.3` (identity stub only; `8086:7113`).
    piix_acpi: [u8; 256],
}

impl Default for PciConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl PciConfig {
    pub fn new() -> Self {
        Self {
            address: 0,
            host_bridge: Self::init_host_bridge(),
            piix_isa: Self::init_piix_isa(),
            piix_ide: Self::init_piix_ide(),
            piix_usb: Self::init_piix_usb(),
            piix_acpi: Self::init_piix_acpi(),
        }
    }

    fn init_host_bridge() -> [u8; 256] {
        let mut cfg = [0u8; 256];
        // Spec: PCI config header type 0 — vendor/device little-endian at 0x00.
        Self::write_id(
            &mut cfg,
            PciHeaderId {
                vendor: PCI_VENDOR_INTEL,
                device: PCI_DEVICE_I440FX,
                revision: 0x02,
                prog_if: 0x00,
                subclass: PCI_SUBCLASS_HOST_BRIDGE,
                class: PCI_CLASS_BRIDGE,
                header_type: 0x00,
            },
        );
        cfg
    }

    fn init_piix_isa() -> [u8; 256] {
        let mut cfg = [0u8; 256];
        // Spec: PCI Local Bus — multi-function header bit7; ISA bridge class 0x0601.
        // Public PIIX3 (82371SB) ISA function ID 8086:7000.
        Self::write_id(
            &mut cfg,
            PciHeaderId {
                vendor: PCI_VENDOR_INTEL,
                device: PCI_DEVICE_PIIX3_ISA,
                revision: 0x00,
                prog_if: 0x00,
                subclass: PCI_SUBCLASS_ISA_BRIDGE,
                class: PCI_CLASS_BRIDGE,
                header_type: PCI_HEADER_MULTIFUNCTION,
            },
        );
        // Spec: Intel 82371SB — PIRQRC[A:D] at 0x60–0x63 default 0x80 (disabled).
        for i in 0..4 {
            cfg[PCI_PIIX_ISA_PIRQRC_OFFSET as usize + i] = PCI_PIIX_ISA_PIRQRC_DEFAULT;
        }
        cfg
    }

    fn init_piix_ide() -> [u8; 256] {
        let mut cfg = [0u8; 256];
        // Spec: PCI class mass-storage / IDE (0x0101); prog IF 0x80 = bus master
        // IDE capable bit advertised by classic PIIX; DMA engine still unsupported.
        // Public PIIX3 IDE function ID 8086:7010.
        Self::write_id(
            &mut cfg,
            PciHeaderId {
                vendor: PCI_VENDOR_INTEL,
                device: PCI_DEVICE_PIIX3_IDE,
                revision: 0x00,
                prog_if: 0x80,
                subclass: PCI_SUBCLASS_IDE,
                class: PCI_CLASS_STORAGE,
                header_type: 0x00,
            },
        );
        cfg
    }

    fn init_piix_usb() -> [u8; 256] {
        let mut cfg = [0u8; 256];
        // Spec: PCI class serial-bus / USB (0x0C03); prog IF 0x00 = UHCI.
        // Public PIIX3 USB function ID 8086:7020 — config identity only.
        Self::write_id(
            &mut cfg,
            PciHeaderId {
                vendor: PCI_VENDOR_INTEL,
                device: PCI_DEVICE_PIIX3_USB,
                revision: 0x00,
                prog_if: PCI_PROG_IF_UHCI,
                subclass: PCI_SUBCLASS_USB,
                class: PCI_CLASS_SERIAL_BUS,
                header_type: 0x00,
            },
        );
        cfg
    }

    fn init_piix_acpi() -> [u8; 256] {
        let mut cfg = [0u8; 256];
        // Spec: PCI class bridge / other (0x0680). Public PIIX4 ACPI ID 8086:7113
        // used as classic pc-i440fx `00:01.3` stub — config identity only.
        Self::write_id(
            &mut cfg,
            PciHeaderId {
                vendor: PCI_VENDOR_INTEL,
                device: PCI_DEVICE_PIIX_ACPI,
                revision: 0x00,
                prog_if: 0x00,
                subclass: PCI_SUBCLASS_OTHER_BRIDGE,
                class: PCI_CLASS_BRIDGE,
                header_type: 0x00,
            },
        );
        cfg
    }

    fn write_id(cfg: &mut [u8; 256], id: PciHeaderId) {
        cfg[0] = (id.vendor & 0xFF) as u8;
        cfg[1] = (id.vendor >> 8) as u8;
        cfg[2] = (id.device & 0xFF) as u8;
        cfg[3] = (id.device >> 8) as u8;
        cfg[8] = id.revision;
        cfg[9] = id.prog_if;
        cfg[10] = id.subclass;
        cfg[11] = id.class;
        cfg[0x0E] = id.header_type;
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// True if this device owns the I/O port.
    pub fn owns_port(port: u16) -> bool {
        matches!(port, 0xCF8..=0xCFF)
    }

    fn enable(&self) -> bool {
        self.address & ADDR_ENABLE != 0
    }

    fn bus(&self) -> u8 {
        ((self.address >> 16) & 0xFF) as u8
    }

    fn device(&self) -> u8 {
        ((self.address >> 11) & 0x1F) as u8
    }

    fn function(&self) -> u8 {
        ((self.address >> 8) & 0x07) as u8
    }

    /// Dword-aligned register number from bits 7:2 (byte offset 0–252).
    fn reg_offset(&self) -> u8 {
        (self.address & 0xFC) as u8
    }

    fn selected_cfg(&self) -> Option<&[u8; 256]> {
        if self.bus() != 0 {
            return None;
        }
        match (self.device(), self.function()) {
            (0, 0) => Some(&self.host_bridge),
            (1, 0) => Some(&self.piix_isa),
            (1, 1) => Some(&self.piix_ide),
            (1, 2) => Some(&self.piix_usb),
            (1, 3) => Some(&self.piix_acpi),
            _ => None,
        }
    }

    fn selected_cfg_mut(&mut self) -> Option<&mut [u8; 256]> {
        if self.bus() != 0 {
            return None;
        }
        match (self.device(), self.function()) {
            (0, 0) => Some(&mut self.host_bridge),
            (1, 0) => Some(&mut self.piix_isa),
            (1, 1) => Some(&mut self.piix_ide),
            (1, 2) => Some(&mut self.piix_usb),
            (1, 3) => Some(&mut self.piix_acpi),
            _ => None,
        }
    }

    fn write_address(&mut self, size: u8, port: u16, value: u32) {
        let shift = ((port - PCI_CONFIG_ADDRESS) as u32) * 8;
        match size {
            4 if port == PCI_CONFIG_ADDRESS => {
                // Spec: PCI Mechanism #1 — bits 1:0 of CONFIG_ADDRESS are hardwired 0.
                self.address = value & !0x3;
            }
            2 if port <= 0xCFA => {
                let mask = 0xFFFFu32 << shift;
                self.address = (self.address & !mask) | ((value as u16 as u32) << shift);
                self.address &= !0x3;
            }
            1 if port <= 0xCFB => {
                let mask = 0xFFu32 << shift;
                self.address = (self.address & !mask) | ((value as u8 as u32) << shift);
                self.address &= !0x3;
            }
            _ => {}
        }
    }

    fn read_address(&self, size: u8, port: u16) -> u32 {
        let shift = ((port - PCI_CONFIG_ADDRESS) as u32) * 8;
        match size {
            4 if port == PCI_CONFIG_ADDRESS => self.address,
            2 if port <= 0xCFA => (self.address >> shift) & 0xFFFF,
            1 if port <= 0xCFB => (self.address >> shift) & 0xFF,
            _ => 0xFFFFFFFF,
        }
    }

    /// Read CONFIG_DATA with port providing the byte offset within the latched dword.
    fn read_data(&self, size: u8, port: u16) -> u32 {
        // Documented choice: enable clear → open-bus `0xFFFFFFFF` on data reads.
        if !self.enable() {
            return 0xFFFFFFFF;
        }
        let Some(cfg) = self.selected_cfg() else {
            return 0xFFFFFFFF;
        };
        let base = self.reg_offset() as usize;
        let lane = (port - PCI_CONFIG_DATA) as usize;
        let off = base + lane;
        match size {
            1 => u32::from(cfg.get(off).copied().unwrap_or(0xFF)),
            2 => {
                let b0 = cfg.get(off).copied().unwrap_or(0xFF);
                let b1 = cfg.get(off + 1).copied().unwrap_or(0xFF);
                u32::from(u16::from_le_bytes([b0, b1]))
            }
            4 => {
                let mut bytes = [0xFFu8; 4];
                for (i, b) in bytes.iter_mut().enumerate() {
                    if let Some(v) = cfg.get(off + i) {
                        *b = *v;
                    }
                }
                u32::from_le_bytes(bytes)
            }
            _ => 0xFFFFFFFF,
        }
    }

    fn write_data(&mut self, size: u8, port: u16, value: u32) {
        if !self.enable() {
            return;
        }
        let base = self.reg_offset() as usize;
        let lane = (port - PCI_CONFIG_DATA) as usize;
        let off = base + lane;
        let is_piix_ide = self.bus() == 0 && self.device() == 1 && self.function() == 1;
        let is_piix_usb = self.bus() == 0 && self.device() == 1 && self.function() == 2;
        // Identity / class / header type are read-only in this stub.
        let readonly = |o: usize| matches!(o, 0x00..=0x03 | 0x08..=0x0B | 0x0E);
        let Some(cfg) = self.selected_cfg_mut() else {
            return;
        };
        match size {
            1 => {
                if off < 256 && !readonly(off) {
                    cfg[off] = value as u8;
                }
            }
            2 => {
                let bytes = (value as u16).to_le_bytes();
                for (i, b) in bytes.iter().enumerate() {
                    let o = off + i;
                    if o < 256 && !readonly(o) {
                        cfg[o] = *b;
                    }
                }
            }
            4 => {
                let bytes = value.to_le_bytes();
                for (i, b) in bytes.iter().enumerate() {
                    let o = off + i;
                    if o < 256 && !readonly(o) {
                        cfg[o] = *b;
                    }
                }
            }
            _ => {}
        }
        // Spec: Intel 82371SB / PCI — PIIX IDE BMIBA at config 0x20 is an I/O BAR:
        // bit0 hardwired 1; address bits 15:4 programmable; bits 3:1 zero.
        // Store/readback only — no BMIDE port decode yet.
        if is_piix_ide && base == PCI_PIIX_IDE_BMIBA_OFFSET as usize && lane == 0 && size == 4 {
            let masked = (value & PCI_PIIX_IDE_BMIBA_MASK) | PCI_BAR_IO_SPACE;
            let bytes = masked.to_le_bytes();
            cfg[PCI_PIIX_IDE_BMIBA_OFFSET as usize..PCI_PIIX_IDE_BMIBA_OFFSET as usize + 4]
                .copy_from_slice(&bytes);
        }
        // Spec: Intel 82371SB / PCI — PIIX USB UHCI BAR0 at config 0x20 is an
        // I/O BAR: bit0 hardwired 1; bits 15:5 programmable (32-byte align);
        // bits 4:1 zero. Store/readback only — no UHCI port decode yet.
        if is_piix_usb && base == PCI_PIIX_USB_BAR0_OFFSET as usize && lane == 0 && size == 4 {
            let masked = (value & PCI_PIIX_USB_BAR0_MASK) | PCI_BAR_IO_SPACE;
            let bytes = masked.to_le_bytes();
            cfg[PCI_PIIX_USB_BAR0_OFFSET as usize..PCI_PIIX_USB_BAR0_OFFSET as usize + 4]
                .copy_from_slice(&bytes);
        }
    }

    /// Build a Type 1 CONFIG_ADDRESS value for tests / callers.
    pub fn make_address(bus: u8, device: u8, function: u8, reg: u8, enable: bool) -> u32 {
        let mut a = (u32::from(bus) << 16)
            | (u32::from(device & 0x1F) << 11)
            | (u32::from(function & 0x07) << 8)
            | (u32::from(reg) & 0xFC);
        if enable {
            a |= ADDR_ENABLE;
        }
        a
    }
}

impl PortDevice for PciConfig {
    fn port_read(&mut self, port: u16, size: u8) -> u32 {
        if (0xCF8..=0xCFB).contains(&port) {
            return self.read_address(size, port);
        }
        if (0xCFC..=0xCFF).contains(&port) {
            return self.read_data(size, port);
        }
        0xFFFFFFFF
    }

    fn port_write(&mut self, port: u16, size: u8, value: u32) {
        if (0xCF8..=0xCFB).contains(&port) {
            self.write_address(size, port, value);
            return;
        }
        if (0xCFC..=0xCFF).contains(&port) {
            self.write_data(size, port, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_bridge_vendor_device_dword() {
        // Spec: PCI Mechanism #1 + header — vendor at 0x00 LE, device at 0x02.
        // 0x12378086 = device 0x1237, vendor 0x8086.
        let mut pci = PciConfig::new();
        let addr = PciConfig::make_address(0, 0, 0, 0x00, true);
        pci.port_write(PCI_CONFIG_ADDRESS, 4, addr);
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 4), 0x1237_8086);
    }

    #[test]
    fn host_bridge_class_and_header_type() {
        // Spec: PCI config header — class code at 0x09–0x0B; header type at 0x0E.
        let mut pci = PciConfig::new();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 0, 0, 0x08, true),
        );
        let dword = pci.port_read(PCI_CONFIG_DATA, 4);
        // Bytes: rev, progIF, subclass, class → LE dword.
        assert_eq!((dword >> 24) as u8, PCI_CLASS_BRIDGE);
        assert_eq!((dword >> 16) as u8, PCI_SUBCLASS_HOST_BRIDGE);

        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 0, 0, 0x0C, true),
        );
        let hdr = pci.port_read(PCI_CONFIG_DATA, 4);
        assert_eq!(((hdr >> 16) & 0xFF) as u8, 0x00); // header type 0
    }

    #[test]
    fn absent_device_returns_ffffffff() {
        // Spec: OSDev PCI / PCI Local Bus — master abort → 0xFFFFFFFF.
        let mut pci = PciConfig::new();
        // 00:1F.0 remains absent (ICH-style slot unused in this i440FX stub).
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 0x1F, 0, 0x00, true),
        );
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 4), 0xFFFF_FFFF);
        // 00:01.4 remains absent (only funcs 0–3 stubbed on this PIIX tree).
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 4, 0x00, true),
        );
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 4), 0xFFFF_FFFF);
    }

    #[test]
    fn enable_clear_data_read_open_bus() {
        // Documented: enable bit 31 clear → CONFIG_DATA reads 0xFFFFFFFF (open bus).
        let mut pci = PciConfig::new();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 0, 0, 0x00, false),
        );
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 4), 0xFFFF_FFFF);
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 1) as u8, 0xFF);
    }

    #[test]
    fn byte_access_vendor_low_via_cfc() {
        // Spec: CONFIG_DATA byte lane at 0xCFC — vendor low byte = 0x86.
        let mut pci = PciConfig::new();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 0, 0, 0x00, true),
        );
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 1) as u8, 0x86);
        assert_eq!(pci.port_read(0xCFD, 1) as u8, 0x80);
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 2) as u16, 0x8086);
    }

    #[test]
    fn address_latch_readback_and_reset() {
        let mut pci = PciConfig::new();
        let addr = PciConfig::make_address(0, 0, 0, 0x04, true);
        pci.port_write(PCI_CONFIG_ADDRESS, 4, addr | 0x3); // bits 1:0 ignored
        assert_eq!(pci.port_read(PCI_CONFIG_ADDRESS, 4), addr);
        pci.reset();
        assert_eq!(pci.address, 0);
        assert_eq!(
            {
                pci.port_write(
                    PCI_CONFIG_ADDRESS,
                    4,
                    PciConfig::make_address(0, 0, 0, 0, true),
                );
                pci.port_read(PCI_CONFIG_DATA, 4)
            },
            0x1237_8086
        );
    }

    #[test]
    fn owns_cf8_through_cff() {
        assert!(PciConfig::owns_port(0xCF8));
        assert!(PciConfig::owns_port(0xCFC));
        assert!(PciConfig::owns_port(0xCFF));
        assert!(!PciConfig::owns_port(0xCF7));
        assert!(!PciConfig::owns_port(0xD00));
    }

    #[test]
    fn piix_isa_bridge_identity_at_00_01_0() {
        // Spec: PCI header + public PIIX3 ISA ID 8086:7000; class ISA bridge 0x0601;
        // multi-function header bit.
        let mut pci = PciConfig::new();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 0, 0x00, true),
        );
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 4), 0x7000_8086);

        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 0, 0x08, true),
        );
        let class_dword = pci.port_read(PCI_CONFIG_DATA, 4);
        assert_eq!((class_dword >> 24) as u8, PCI_CLASS_BRIDGE);
        assert_eq!((class_dword >> 16) as u8, PCI_SUBCLASS_ISA_BRIDGE);

        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 0, 0x0C, true),
        );
        let hdr = pci.port_read(PCI_CONFIG_DATA, 4);
        assert_eq!(
            ((hdr >> 16) & 0xFF) as u8,
            PCI_HEADER_MULTIFUNCTION,
            "ISA function must advertise multi-function"
        );
    }

    #[test]
    fn piix_ide_identity_at_00_01_1() {
        // Spec: PCI header + public PIIX3 IDE ID 8086:7010; class IDE 0x0101.
        let mut pci = PciConfig::new();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 1, 0x00, true),
        );
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 4), 0x7010_8086);

        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 1, 0x08, true),
        );
        let class_dword = pci.port_read(PCI_CONFIG_DATA, 4);
        assert_eq!((class_dword >> 24) as u8, PCI_CLASS_STORAGE);
        assert_eq!((class_dword >> 16) as u8, PCI_SUBCLASS_IDE);
        assert_eq!((class_dword >> 8) as u8, 0x80); // prog IF bus-master capable bit
    }

    /// Spec: Intel 82371SB — PIIX IDE BMIBA at PCI config `0x20` is an I/O BAR
    /// (bit0=1); bits 15:4 hold the 16-byte-aligned I/O base; store/readback only.
    #[test]
    fn piix_ide_bmiba_io_bar_store_readback() {
        let mut pci = PciConfig::new();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 1, PCI_PIIX_IDE_BMIBA_OFFSET, true),
        );
        // Default after init is 0.
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 4), 0);

        // Guest programs base 0xF000 with junk low bits; device forces I/O BAR form.
        pci.port_write(PCI_CONFIG_DATA, 4, 0x0000_F00E);
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 4),
            0x0000_F001,
            "BMIBA: bits15:4 kept, bit0=1, bits3:1=0"
        );

        pci.port_write(PCI_CONFIG_DATA, 4, 0x0000_C000);
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 4), 0x0000_C001);
    }

    #[test]
    fn piix_ide_bmiba_does_not_alter_other_functions() {
        // Writing BMIBA-shaped value at host bridge BAR0 offset must not force I/O form.
        let mut pci = PciConfig::new();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 0, 0, PCI_PIIX_IDE_BMIBA_OFFSET, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 4, 0x0000_F000);
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 4),
            0x0000_F000,
            "non-IDE function keeps raw writable dword"
        );
    }

    #[test]
    fn piix_command_byte_writable_identity_readonly() {
        // Spec: PCI config — Command at 0x04 writable; vendor/device RO.
        let mut pci = PciConfig::new();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 0, 0x04, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 2, 0x0007);
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 2) as u16, 0x0007);

        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 0, 0x00, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 4, 0xDEAD_BEEF);
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 4), 0x7000_8086);
    }

    /// Spec: Intel 82371SB — PIRQRC[A:D] at ISA config `0x60`–`0x63` default
    /// `0x80`; store/readback (routing not wired to PIC yet).
    #[test]
    fn piix_isa_pirqrc_default_and_store_readback() {
        let mut pci = PciConfig::new();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 0, PCI_PIIX_ISA_PIRQRC_OFFSET, true),
        );
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 4),
            0x8080_8080,
            "PIRQRC[A:D] default 0x80 each"
        );
        pci.port_write(PCI_CONFIG_DATA, 4, 0x0B0A_0903);
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 4), 0x0B0A_0903);
        // Byte lane store of PIRQRC[B] (offset 0x61).
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 0, 0x61, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 1, 0x05);
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 1) as u8, 0x05);
    }

    /// Spec: Intel 82371SB — PIIX USB UHCI BAR0 at PCI config `0x20` is an I/O
    /// BAR (bit0=1); bits 15:5 hold the 32-byte-aligned I/O base.
    #[test]
    fn piix_usb_bar0_io_bar_store_readback() {
        let mut pci = PciConfig::new();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 2, PCI_PIIX_USB_BAR0_OFFSET, true),
        );
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 4), 0);

        // Guest programs base 0xC000 with junk low bits; device forces I/O BAR form.
        pci.port_write(PCI_CONFIG_DATA, 4, 0x0000_C01E);
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 4),
            0x0000_C001,
            "UHCI BAR0: bits15:5 kept, bit0=1, bits4:1=0"
        );

        pci.port_write(PCI_CONFIG_DATA, 4, 0x0000_F020);
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 4), 0x0000_F021);
    }

    #[test]
    fn piix_usb_identity_at_00_01_2() {
        // Spec: PCI header + public PIIX3 USB ID 8086:7020; class USB UHCI 0x0C0300.
        let mut pci = PciConfig::new();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 2, 0x00, true),
        );
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 4), 0x7020_8086);

        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 2, 0x08, true),
        );
        let class_dword = pci.port_read(PCI_CONFIG_DATA, 4);
        assert_eq!((class_dword >> 24) as u8, PCI_CLASS_SERIAL_BUS);
        assert_eq!((class_dword >> 16) as u8, PCI_SUBCLASS_USB);
        assert_eq!((class_dword >> 8) as u8, PCI_PROG_IF_UHCI);
    }

    #[test]
    fn piix_acpi_identity_at_00_01_3() {
        // Spec: PCI header + public PIIX4 ACPI ID 8086:7113; class bridge/other 0x0680.
        let mut pci = PciConfig::new();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 3, 0x00, true),
        );
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 4), 0x7113_8086);

        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 3, 0x08, true),
        );
        let class_dword = pci.port_read(PCI_CONFIG_DATA, 4);
        assert_eq!((class_dword >> 24) as u8, PCI_CLASS_BRIDGE);
        assert_eq!((class_dword >> 16) as u8, PCI_SUBCLASS_OTHER_BRIDGE);
        assert_eq!((class_dword >> 8) as u8, 0x00);
    }
}
