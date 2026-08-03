//! PCI configuration mechanism #1 stub — ports `0xCF8` / `0xCFC`–`0xCFF`.
//!
//! Classic PC Type 1 configuration access: latch a bus/device/function/register
//! address in `CONFIG_ADDRESS`, then read/write `CONFIG_DATA`.
//!
//! # Spec refs
//!
//! - PCI Local Bus Specification — Configuration Mechanism #1 (`CONFIG_ADDRESS`
//!   at `0xCF8`, `CONFIG_DATA` at `0xCFC`; enable bit 31; Type 1 address fields).
//! - OSDev Wiki PCI — Configuration Space Access Mechanism #1; absent device
//!   reads return `0xFFFFFFFF`.
//! - Intel 440FX (i440FX) host bridge identity used as the bus0/dev0/func0 stub
//!   (`vendor 0x8086`, `device 0x1237`); class code host bridge (`0x06`/`0x00`).
//! - `docs/machine-model-pc-v1.md`, `plan.md` §21 PCI.
//!
//! # Scope (this slice)
//!
//! - Type 1 address latch (enable / bus / device / function / register).
//! - Host bridge at `00:00.0` with Intel-style vendor/device/class/header type 0.
//! - Absent devices: `0xFFFFFFFF` when enable is set.
//! - Enable bit clear: data-port reads return `0xFFFFFFFF` (open-bus style).
//! - Byte/word/dword access via `0xCFC` + offset.
//!
//! # Unsupported (explicit)
//!
//! - Full PIIX3 / multi-function device tree
//! - BAR MMIO decode, bus mastering, INTx routing
//! - Capability lists, MSI, PCIe, hotplug

use crate::PortDevice;

/// CONFIG_ADDRESS (Type 1). Spec: PCI Local Bus — Mechanism #1.
pub const PCI_CONFIG_ADDRESS: u16 = 0xCF8;
/// CONFIG_DATA base (bytes `0xCFC`–`0xCFF`).
pub const PCI_CONFIG_DATA: u16 = 0xCFC;

/// Intel vendor ID.
pub const PCI_VENDOR_INTEL: u16 = 0x8086;
/// i440FX-class host bridge device ID (stub identity).
pub const PCI_DEVICE_I440FX: u16 = 0x1237;
/// PCI class: Bridge device.
pub const PCI_CLASS_BRIDGE: u8 = 0x06;
/// PCI subclass: Host/PCI bridge.
pub const PCI_SUBCLASS_HOST_BRIDGE: u8 = 0x00;

/// Enable bit in CONFIG_ADDRESS (bit 31).
const ADDR_ENABLE: u32 = 1 << 31;

/// PCI configuration mechanism #1 controller stub.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PciConfig {
    /// Latched CONFIG_ADDRESS value (bits 1:0 forced clear on write).
    pub address: u32,
    /// 256-byte config space for the host bridge at `00:00.0`.
    host_bridge: [u8; 256],
}

impl Default for PciConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl PciConfig {
    pub fn new() -> Self {
        let mut host_bridge = [0u8; 256];
        // Spec: PCI config header type 0 — vendor/device little-endian at 0x00.
        host_bridge[0] = (PCI_VENDOR_INTEL & 0xFF) as u8;
        host_bridge[1] = (PCI_VENDOR_INTEL >> 8) as u8;
        host_bridge[2] = (PCI_DEVICE_I440FX & 0xFF) as u8;
        host_bridge[3] = (PCI_DEVICE_I440FX >> 8) as u8;
        // Revision 0x02 (common i440FX-style); class code host bridge at 0x09–0x0B.
        host_bridge[8] = 0x02;
        host_bridge[9] = 0x00; // prog IF
        host_bridge[10] = PCI_SUBCLASS_HOST_BRIDGE;
        host_bridge[11] = PCI_CLASS_BRIDGE;
        // Header type 0 at offset 0x0E.
        host_bridge[0x0E] = 0x00;
        Self {
            address: 0,
            host_bridge,
        }
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

    fn is_host_bridge(&self) -> bool {
        self.bus() == 0 && self.device() == 0 && self.function() == 0
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
        if !self.is_host_bridge() {
            return 0xFFFFFFFF;
        }
        let base = self.reg_offset() as usize;
        let lane = (port - PCI_CONFIG_DATA) as usize;
        let off = base + lane;
        match size {
            1 => u32::from(self.host_bridge.get(off).copied().unwrap_or(0xFF)),
            2 => {
                let b0 = self.host_bridge.get(off).copied().unwrap_or(0xFF);
                let b1 = self.host_bridge.get(off + 1).copied().unwrap_or(0xFF);
                u32::from(u16::from_le_bytes([b0, b1]))
            }
            4 => {
                let mut bytes = [0xFFu8; 4];
                for (i, b) in bytes.iter_mut().enumerate() {
                    if let Some(v) = self.host_bridge.get(off + i) {
                        *b = *v;
                    }
                }
                u32::from_le_bytes(bytes)
            }
            _ => 0xFFFFFFFF,
        }
    }

    fn write_data(&mut self, size: u8, port: u16, value: u32) {
        if !self.enable() || !self.is_host_bridge() {
            return;
        }
        let base = self.reg_offset() as usize;
        let lane = (port - PCI_CONFIG_DATA) as usize;
        let off = base + lane;
        // Identity / class / header type are read-only in this stub.
        let readonly = |o: usize| matches!(o, 0x00..=0x03 | 0x08..=0x0B | 0x0E);
        match size {
            1 => {
                if off < 256 && !readonly(off) {
                    self.host_bridge[off] = value as u8;
                }
            }
            2 => {
                let bytes = (value as u16).to_le_bytes();
                for (i, b) in bytes.iter().enumerate() {
                    let o = off + i;
                    if o < 256 && !readonly(o) {
                        self.host_bridge[o] = *b;
                    }
                }
            }
            4 => {
                let bytes = value.to_le_bytes();
                for (i, b) in bytes.iter().enumerate() {
                    let o = off + i;
                    if o < 256 && !readonly(o) {
                        self.host_bridge[o] = *b;
                    }
                }
            }
            _ => {}
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
        // 00:1F.0 is a common PIIX ISA bridge slot; absent in this stub.
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 0x1F, 0, 0x00, true),
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
}
