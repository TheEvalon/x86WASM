//! PCI configuration mechanism #1 stub — ports `0xCF8` / `0xCFC`–`0xCFF`,
//! plus PIIX ISA Edge/Level Control (ELCR) at `0x4D0`/`0x4D1`.
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
//! - Intel 82371 / PIIX ISA bridge ELCR — I/O ports `0x4D0` (master PIC IRQs
//!   0–7) and `0x4D1` (slave PIC IRQs 8–15); SeaBIOS/firmware programs these for
//!   PCI level-triggered IRQ routing. OSDev Wiki 8259 PIC — ELCR.
//! - `docs/machine-model-pc-v1.md`, `plan.md` §15.2 / §21 PCI.
//!
//! # Scope (this slice)
//!
//! - Type 1 address latch (enable / bus / device / function / register).
//! - Host bridge at `00:00.0` with Intel-style vendor/device/class/header type 0.
//! - Host bridge Command (`0x04`) store/readback: sticky IO/MEM/BusMaster only
//!   (`PCI_HOST_BRIDGE_COMMAND_MASK` = `0x0007`); other Command bits hardwired 0.
//! - Host bridge Status (`0x06`) readback stub: CapList=0, FastB2B=1, DevSel=medium
//!   (`PCI_HOST_BRIDGE_STATUS_STUB` = `0x0280`); RW1C error bits (MDPE/STA/RTA/RMA/SSE/DPE).
//! - PIIX ISA bridge (`00:01.0`) Command (`0x04`) store/readback: sticky IO/MEM/BusMaster
//!   (`PCI_PIIX_ISA_COMMAND_MASK` = `0x0007`, same as host bridge); other bits hardwired 0.
//! - PIIX ISA Status (`0x06`) readback stub: same CapList/FastB2B/DevSel as host bridge
//!   (`PCI_PIIX_ISA_STATUS_STUB` = `0x0280`); RW1C error bits via `PCI_STATUS_RW1C_MASK`.
//! - PIIX IDE (`00:01.1`) Command (`0x04`) store/readback: sticky IO/BusMaster only
//!   (`PCI_PIIX_IDE_COMMAND_MASK` = `0x0005`); MEM and other bits hardwired 0.
//! - PIIX IDE Status (`0x06`) readback stub: same CapList/FastB2B/DevSel as host bridge
//!   (`PCI_PIIX_IDE_STATUS_STUB` = `0x0280`); RW1C error bits via `PCI_STATUS_RW1C_MASK`.
//! - PIIX USB UHCI (`00:01.2`) Command (`0x04`) store/readback: sticky IO/MEM/BusMaster
//!   (`PCI_PIIX_USB_COMMAND_MASK` = `0x0007`, same as host bridge); other bits hardwired 0.
//! - PIIX USB Status (`0x06`) readback stub: same CapList/FastB2B/DevSel as host bridge
//!   (`PCI_PIIX_USB_STATUS_STUB` = `0x0280`); RW1C error bits via `PCI_STATUS_RW1C_MASK`.
//! - PIIX ACPI (`00:01.3`) Command (`0x04`) store/readback: sticky IO/MEM/BusMaster
//!   (`PCI_PIIX_ACPI_COMMAND_MASK` = `0x0007`, same as host bridge); other bits hardwired 0.
//! - PIIX ACPI Status (`0x06`) readback stub: same CapList/FastB2B/DevSel as host bridge
//!   (`PCI_PIIX_ACPI_STATUS_STUB` = `0x0280`); RW1C error bits via `PCI_STATUS_RW1C_MASK`.
//! - PIIX-style stubs: `00:01.0` ISA bridge (multi-function), `00:01.1` IDE,
//!   `00:01.2` USB UHCI, `00:01.3` ACPI (Command + Status + PMBASE identity).
//! - Absent devices: `0xFFFFFFFF` when enable is set.
//! - Enable bit clear: data-port reads return `0xFFFFFFFF` (open-bus style).
//! - Byte/word/dword access via `0xCFC` + offset.
//! - PIIX ELCR `0x4D0`/`0x4D1` byte store/readback (reset `0x00`/`0x00`).
//!
//! # Unsupported (explicit)
//!
//! - BAR MMIO/IO decode, bus mastering engine, INTx routing tables
//! - Host-bridge / PIIX ISA / PIIX IDE / PIIX USB / PIIX ACPI Command side effects (IO/MEM decode, bus-master DMA)
//! - Status error *signaling* (host / ISA / IDE / USB / ACPI never latch RW1C bits from real aborts yet)
//! - Capability list walk (CapList hardwired 0 on host / ISA / IDE / USB / ACPI)
//! - USB host controller (UHCI frame list / ports / IRQ)
//! - ACPI PM I/O block / SMI / GPE / ACPI tables (Command + Status + PMBASE config only)
//! - Capability lists, MSI, PCIe, hotplug
//! - IDE BARs tied to `IdePrimary` ports (legacy fixed ports remain)
//! - ELCR bits driving `DualPic` LTIM / per-IRQ edge vs level (store only)

use crate::PortDevice;

/// CONFIG_ADDRESS (Type 1). Spec: PCI Local Bus — Mechanism #1.
pub const PCI_CONFIG_ADDRESS: u16 = 0xCF8;
/// CONFIG_DATA base (bytes `0xCFC`–`0xCFF`).
pub const PCI_CONFIG_DATA: u16 = 0xCFC;

/// PCI Command register config offset.
/// Spec: PCI Local Bus — Type 0 header Command at `0x04`.
pub const PCI_COMMAND_OFFSET: u8 = 0x04;
/// Command bit 0: I/O Space Enable.
pub const PCI_COMMAND_IO: u16 = 1 << 0;
/// Command bit 1: Memory Space Enable.
pub const PCI_COMMAND_MEM: u16 = 1 << 1;
/// Command bit 2: Bus Master Enable.
pub const PCI_COMMAND_BUS_MASTER: u16 = 1 << 2;
/// Host bridge (`00:00.0`) Command sticky-bit mask for this stub.
///
/// Sticky: IO | MEM | BusMaster (`0x0007`). All other Command bits are
/// hardwired 0 on store (no SERR/PERR/INTx-disable/etc. side effects yet).
pub const PCI_HOST_BRIDGE_COMMAND_MASK: u16 =
    PCI_COMMAND_IO | PCI_COMMAND_MEM | PCI_COMMAND_BUS_MASTER;
/// PIIX ISA bridge (`00:01.0`) Command sticky-bit mask for this stub.
///
/// Sticky: IO | MEM | BusMaster (`0x0007`) — same mask as the host bridge
/// (`PCI_HOST_BRIDGE_COMMAND_MASK`). All other Command bits are hardwired 0 on
/// store (no IO/MEM/BM decode side effects yet).
/// Spec: PCI Local Bus Command at `0x04`; Intel 82371SB ISA bridge function.
pub const PCI_PIIX_ISA_COMMAND_MASK: u16 =
    PCI_COMMAND_IO | PCI_COMMAND_MEM | PCI_COMMAND_BUS_MASTER;
const _: () = assert!(PCI_PIIX_ISA_COMMAND_MASK == PCI_HOST_BRIDGE_COMMAND_MASK);
/// PIIX IDE (`00:01.1`) Command sticky-bit mask for this stub.
///
/// Sticky: IO | BusMaster (`0x0005`). MEM Space Enable and all other Command
/// bits are hardwired 0 on store (legacy IDE + BMIBA are I/O; no MEM decode /
/// BM engine side effects yet).
pub const PCI_PIIX_IDE_COMMAND_MASK: u16 = PCI_COMMAND_IO | PCI_COMMAND_BUS_MASTER;
/// PIIX USB UHCI (`00:01.2`) Command sticky-bit mask for this stub.
///
/// Sticky: IO | MEM | BusMaster (`0x0007`) — same mask as the host bridge
/// (`PCI_HOST_BRIDGE_COMMAND_MASK`). All other Command bits are hardwired 0 on
/// store (no IO/MEM/BM / UHCI engine side effects yet).
/// Spec: PCI Local Bus Command at `0x04`; Intel 82371SB USB UHCI function.
pub const PCI_PIIX_USB_COMMAND_MASK: u16 =
    PCI_COMMAND_IO | PCI_COMMAND_MEM | PCI_COMMAND_BUS_MASTER;
const _: () = assert!(PCI_PIIX_USB_COMMAND_MASK == PCI_HOST_BRIDGE_COMMAND_MASK);
/// PIIX ACPI (`00:01.3`) Command sticky-bit mask for this stub.
///
/// Sticky: IO | MEM | BusMaster (`0x0007`) — same mask as the host bridge
/// (`PCI_HOST_BRIDGE_COMMAND_MASK`). All other Command bits are hardwired 0 on
/// store (no IO/MEM/BM / ACPI PM I/O side effects yet).
/// Spec: PCI Local Bus Command at `0x04`; Intel 82371AB ACPI function.
pub const PCI_PIIX_ACPI_COMMAND_MASK: u16 =
    PCI_COMMAND_IO | PCI_COMMAND_MEM | PCI_COMMAND_BUS_MASTER;
const _: () = assert!(PCI_PIIX_ACPI_COMMAND_MASK == PCI_HOST_BRIDGE_COMMAND_MASK);

/// PCI Status register config offset.
/// Spec: PCI Local Bus — Type 0 header Status at `0x06`.
pub const PCI_STATUS_OFFSET: u8 = 0x06;
/// Status bit 4: Capabilities List (RO). Stub: 0 — no cap list yet.
pub const PCI_STATUS_CAP_LIST: u16 = 1 << 4;
/// Status bit 7: Fast Back-to-Back Capable (RO).
pub const PCI_STATUS_FAST_BACK: u16 = 1 << 7;
/// Status bits 10:9: DEVSEL Timing mask (RO).
pub const PCI_STATUS_DEVSEL_MASK: u16 = 0x0600;
/// DEVSEL Timing = medium (`01b`).
pub const PCI_STATUS_DEVSEL_MEDIUM: u16 = 0x0200;
/// Status bit 8: Master Data Parity Error (RW1C).
pub const PCI_STATUS_PARITY: u16 = 1 << 8;
/// Status bit 11: Signaled Target Abort (RW1C).
pub const PCI_STATUS_SIG_TARGET_ABORT: u16 = 1 << 11;
/// Status bit 12: Received Target Abort (RW1C).
pub const PCI_STATUS_REC_TARGET_ABORT: u16 = 1 << 12;
/// Status bit 13: Received Master Abort (RW1C).
pub const PCI_STATUS_REC_MASTER_ABORT: u16 = 1 << 13;
/// Status bit 14: Signaled System Error (RW1C).
pub const PCI_STATUS_SIG_SYSTEM_ERROR: u16 = 1 << 14;
/// Status bit 15: Detected Parity Error (RW1C).
pub const PCI_STATUS_DETECTED_PARITY: u16 = 1 << 15;
/// Host bridge Status RW1C error-bit mask (MDPE|STA|RTA|RMA|SSE|DPE).
pub const PCI_STATUS_RW1C_MASK: u16 = PCI_STATUS_PARITY
    | PCI_STATUS_SIG_TARGET_ABORT
    | PCI_STATUS_REC_TARGET_ABORT
    | PCI_STATUS_REC_MASTER_ABORT
    | PCI_STATUS_SIG_SYSTEM_ERROR
    | PCI_STATUS_DETECTED_PARITY;
/// Host bridge (`00:00.0`) Status hardwired stub value.
///
/// CapList=0 (no capability pointer), FastB2B=1, DevSel=medium → `0x0280`.
/// Spec: PCI Local Bus — Status CapList / Fast Back-to-Back / DEVSEL Timing (RO).
pub const PCI_HOST_BRIDGE_STATUS_STUB: u16 = PCI_STATUS_FAST_BACK | PCI_STATUS_DEVSEL_MEDIUM;
/// PIIX ISA bridge (`00:01.0`) Status hardwired stub — same CapList/FastB2B/DevSel
/// pattern as the host bridge (`0x0280`). Spec: PCI Local Bus — Status register;
/// Intel 82371SB ISA bridge function.
pub const PCI_PIIX_ISA_STATUS_STUB: u16 = PCI_HOST_BRIDGE_STATUS_STUB;
const _: () = assert!(PCI_PIIX_ISA_STATUS_STUB == PCI_HOST_BRIDGE_STATUS_STUB);
/// PIIX IDE (`00:01.1`) Status hardwired stub — same CapList/FastB2B/DevSel pattern
/// as the host bridge (`0x0280`). Spec: PCI Local Bus — Status register.
pub const PCI_PIIX_IDE_STATUS_STUB: u16 = PCI_HOST_BRIDGE_STATUS_STUB;
/// PIIX USB UHCI (`00:01.2`) Status hardwired stub — same CapList/FastB2B/DevSel
/// pattern as the host bridge (`0x0280`). Spec: PCI Local Bus — Status register;
/// Intel 82371SB USB UHCI function.
pub const PCI_PIIX_USB_STATUS_STUB: u16 = PCI_HOST_BRIDGE_STATUS_STUB;
const _: () = assert!(PCI_PIIX_USB_STATUS_STUB == PCI_HOST_BRIDGE_STATUS_STUB);
/// PIIX ACPI (`00:01.3`) Status hardwired stub — same CapList/FastB2B/DevSel
/// pattern as the host bridge (`0x0280`). Spec: PCI Local Bus — Status register;
/// Intel 82371AB ACPI function.
pub const PCI_PIIX_ACPI_STATUS_STUB: u16 = PCI_HOST_BRIDGE_STATUS_STUB;
const _: () = assert!(PCI_PIIX_ACPI_STATUS_STUB == PCI_HOST_BRIDGE_STATUS_STUB);

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
/// PIIX IDE IDE Timing Register (IDETIM) config offset. Spec: Intel 82371SB — word at `0x40`.
pub const PCI_PIIX_IDE_IDETIM_OFFSET: u8 = 0x40;
/// PIIX ACPI PMBASE config offset (I/O BAR). Spec: Intel 82371AB — dword at `0x40`.
pub const PCI_PIIX_ACPI_PMBASE_OFFSET: u8 = 0x40;
/// IDETIM and ACPI PMBASE share config offset `0x40` on different functions.
const _: () = assert!(PCI_PIIX_IDE_IDETIM_OFFSET == PCI_PIIX_ACPI_PMBASE_OFFSET);
/// PIIX USB UHCI Legacy Support (LEGSUP) config offset. Spec: UHCI — dword at `0xC0`.
pub const PCI_PIIX_USB_LEGSUP_OFFSET: u8 = 0xC0;
const _: () = assert!(PCI_PIIX_USB_LEGSUP_OFFSET == 0xC0);
/// PMBASE I/O decode mask — 64-byte aligned (bits 15:6); bit0 = I/O space.
pub const PCI_PIIX_ACPI_PMBASE_MASK: u32 = 0xFFC0;
/// PIIX ISA Edge/Level Control Register — master PIC (IRQs 0–7).
/// Spec: Intel 82371 / OSDev 8259 PIC ELCR — I/O port `0x4D0`.
pub const PIIX_ELCR_MASTER: u16 = 0x4D0;
/// PIIX ISA Edge/Level Control Register — slave PIC (IRQs 8–15).
/// Spec: Intel 82371 / OSDev 8259 PIC ELCR — I/O port `0x4D1`.
pub const PIIX_ELCR_SLAVE: u16 = 0x4D1;

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
    /// PIIX ISA ELCR bytes at `0x4D0`/`0x4D1` (master/slave); reset `0x00`.
    /// Store/readback only — not wired to `DualPic` LTIM yet.
    pub elcr: [u8; 2],
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
            // Spec: PIIX ELCR power-on / reset defaults to edge-triggered (0).
            elcr: [0, 0],
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
        // Spec: PCI Status at 0x06 — CapList=0, FastB2B, DevSel=medium stub.
        let st = PCI_STATUS_OFFSET as usize;
        cfg[st..st + 2].copy_from_slice(&PCI_HOST_BRIDGE_STATUS_STUB.to_le_bytes());
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
        // Spec: PCI Local Bus — Status at 0x06 CapList=0, FastB2B, DevSel=medium stub.
        let st = PCI_STATUS_OFFSET as usize;
        cfg[st..st + 2].copy_from_slice(&PCI_PIIX_ISA_STATUS_STUB.to_le_bytes());
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
        // Spec: PCI Local Bus — Status at 0x06 CapList=0, FastB2B, DevSel=medium stub.
        let st = PCI_STATUS_OFFSET as usize;
        cfg[st..st + 2].copy_from_slice(&PCI_PIIX_IDE_STATUS_STUB.to_le_bytes());
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
        // Spec: PCI Local Bus — Status at 0x06 CapList=0, FastB2B, DevSel=medium stub.
        let st = PCI_STATUS_OFFSET as usize;
        cfg[st..st + 2].copy_from_slice(&PCI_PIIX_USB_STATUS_STUB.to_le_bytes());
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
        // Spec: PCI Local Bus — Status at 0x06 CapList=0, FastB2B, DevSel=medium stub.
        let st = PCI_STATUS_OFFSET as usize;
        cfg[st..st + 2].copy_from_slice(&PCI_PIIX_ACPI_STATUS_STUB.to_le_bytes());
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
        matches!(port, 0xCF8..=0xCFF | PIIX_ELCR_MASTER | PIIX_ELCR_SLAVE)
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
        let is_host_bridge = self.bus() == 0 && self.device() == 0 && self.function() == 0;
        let is_piix_isa = self.bus() == 0 && self.device() == 1 && self.function() == 0;
        let is_piix_ide = self.bus() == 0 && self.device() == 1 && self.function() == 1;
        let is_piix_usb = self.bus() == 0 && self.device() == 1 && self.function() == 2;
        let is_piix_acpi = self.bus() == 0 && self.device() == 1 && self.function() == 3;
        // Spec: PCI Status RW1C needs pre-write value (write-1-to-clear).
        let old_status =
            if is_host_bridge || is_piix_isa || is_piix_ide || is_piix_usb || is_piix_acpi {
                self.selected_cfg().map(|cfg| {
                    let st = PCI_STATUS_OFFSET as usize;
                    u16::from_le_bytes([cfg[st], cfg[st + 1]])
                })
            } else {
                None
            };
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
        // Spec: PCI Local Bus — Command at 0x04. Host bridge stub keeps only
        // IO/MEM/BusMaster sticky; other Command bits hardwired 0 (no decode yet).
        // Status at 0x06: hardwired CapList/FastB2B/DevSel + RW1C error bits.
        if is_host_bridge {
            let cmd_off = PCI_COMMAND_OFFSET as usize;
            let cmd = u16::from_le_bytes([cfg[cmd_off], cfg[cmd_off + 1]]);
            let masked = cmd & PCI_HOST_BRIDGE_COMMAND_MASK;
            cfg[cmd_off..cmd_off + 2].copy_from_slice(&masked.to_le_bytes());

            let st_off = PCI_STATUS_OFFSET as usize;
            let old = old_status.unwrap_or(PCI_HOST_BRIDGE_STATUS_STUB);
            let written = status_written_bits(base, lane, size, value);
            let rw1c = (old & PCI_STATUS_RW1C_MASK) & !(written & PCI_STATUS_RW1C_MASK);
            let status = PCI_HOST_BRIDGE_STATUS_STUB | rw1c;
            cfg[st_off..st_off + 2].copy_from_slice(&status.to_le_bytes());
        }
        // Spec: PCI Local Bus + Intel 82371SB — PIIX ISA bridge Command at 0x04
        // keeps IO/MEM/BusMaster sticky (same mask as host bridge); other bits
        // hardwired 0. Status at 0x06: same CapList/FastB2B/DevSel stub + RW1C.
        // Store/readback only — no decode side effects / error signaling yet.
        if is_piix_isa {
            let cmd_off = PCI_COMMAND_OFFSET as usize;
            let cmd = u16::from_le_bytes([cfg[cmd_off], cfg[cmd_off + 1]]);
            let masked = cmd & PCI_PIIX_ISA_COMMAND_MASK;
            cfg[cmd_off..cmd_off + 2].copy_from_slice(&masked.to_le_bytes());

            let st_off = PCI_STATUS_OFFSET as usize;
            let old = old_status.unwrap_or(PCI_PIIX_ISA_STATUS_STUB);
            let written = status_written_bits(base, lane, size, value);
            let rw1c = (old & PCI_STATUS_RW1C_MASK) & !(written & PCI_STATUS_RW1C_MASK);
            let status = PCI_PIIX_ISA_STATUS_STUB | rw1c;
            cfg[st_off..st_off + 2].copy_from_slice(&status.to_le_bytes());
        }
        // Spec: PCI Local Bus + Intel 82371SB — PIIX IDE Command at 0x04 keeps
        // only IO/BusMaster sticky; MEM and other bits hardwired 0 (no decode yet).
        // Status at 0x06: same CapList/FastB2B/DevSel stub + RW1C as host bridge.
        if is_piix_ide {
            let cmd_off = PCI_COMMAND_OFFSET as usize;
            let cmd = u16::from_le_bytes([cfg[cmd_off], cfg[cmd_off + 1]]);
            let masked = cmd & PCI_PIIX_IDE_COMMAND_MASK;
            cfg[cmd_off..cmd_off + 2].copy_from_slice(&masked.to_le_bytes());

            let st_off = PCI_STATUS_OFFSET as usize;
            let old = old_status.unwrap_or(PCI_PIIX_IDE_STATUS_STUB);
            let written = status_written_bits(base, lane, size, value);
            let rw1c = (old & PCI_STATUS_RW1C_MASK) & !(written & PCI_STATUS_RW1C_MASK);
            let status = PCI_PIIX_IDE_STATUS_STUB | rw1c;
            cfg[st_off..st_off + 2].copy_from_slice(&status.to_le_bytes());
        }
        // Spec: PCI Local Bus + Intel 82371SB — PIIX USB UHCI Command at 0x04
        // keeps IO/MEM/BusMaster sticky (same mask as host bridge); other bits
        // hardwired 0. Status at 0x06: same CapList/FastB2B/DevSel stub + RW1C.
        // Store/readback only — no UHCI IO/MEM/BM side effects / error signaling yet.
        if is_piix_usb {
            let cmd_off = PCI_COMMAND_OFFSET as usize;
            let cmd = u16::from_le_bytes([cfg[cmd_off], cfg[cmd_off + 1]]);
            let masked = cmd & PCI_PIIX_USB_COMMAND_MASK;
            cfg[cmd_off..cmd_off + 2].copy_from_slice(&masked.to_le_bytes());

            let st_off = PCI_STATUS_OFFSET as usize;
            let old = old_status.unwrap_or(PCI_PIIX_USB_STATUS_STUB);
            let written = status_written_bits(base, lane, size, value);
            let rw1c = (old & PCI_STATUS_RW1C_MASK) & !(written & PCI_STATUS_RW1C_MASK);
            let status = PCI_PIIX_USB_STATUS_STUB | rw1c;
            cfg[st_off..st_off + 2].copy_from_slice(&status.to_le_bytes());
        }
        // Spec: PCI Local Bus + Intel 82371AB — PIIX ACPI Command at 0x04
        // keeps IO/MEM/BusMaster sticky (same mask as host bridge); other bits
        // hardwired 0. Status at 0x06: same CapList/FastB2B/DevSel stub + RW1C.
        // Store/readback only — no ACPI PM I/O side effects / error signaling yet.
        if is_piix_acpi {
            let cmd_off = PCI_COMMAND_OFFSET as usize;
            let cmd = u16::from_le_bytes([cfg[cmd_off], cfg[cmd_off + 1]]);
            let masked = cmd & PCI_PIIX_ACPI_COMMAND_MASK;
            cfg[cmd_off..cmd_off + 2].copy_from_slice(&masked.to_le_bytes());

            let st_off = PCI_STATUS_OFFSET as usize;
            let old = old_status.unwrap_or(PCI_PIIX_ACPI_STATUS_STUB);
            let written = status_written_bits(base, lane, size, value);
            let rw1c = (old & PCI_STATUS_RW1C_MASK) & !(written & PCI_STATUS_RW1C_MASK);
            let status = PCI_PIIX_ACPI_STATUS_STUB | rw1c;
            cfg[st_off..st_off + 2].copy_from_slice(&status.to_le_bytes());
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
        // Spec: Intel 82371AB / PCI — PIIX ACPI PMBASE at config 0x40 is an I/O
        // BAR: bit0 hardwired 1; bits 15:6 programmable (64-byte align).
        // Store/readback only — no ACPI PM I/O decode yet.
        if is_piix_acpi && base == PCI_PIIX_ACPI_PMBASE_OFFSET as usize && lane == 0 && size == 4 {
            let masked = (value & PCI_PIIX_ACPI_PMBASE_MASK) | PCI_BAR_IO_SPACE;
            let bytes = masked.to_le_bytes();
            cfg[PCI_PIIX_ACPI_PMBASE_OFFSET as usize..PCI_PIIX_ACPI_PMBASE_OFFSET as usize + 4]
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

/// Bits written into Status (`0x06`/`0x07`) by a CONFIG_DATA store (0 = lane not touched).
fn status_written_bits(base: usize, lane: usize, size: u8, value: u32) -> u16 {
    let off = base + lane;
    let st = PCI_STATUS_OFFSET as usize;
    let bytes = value.to_le_bytes();
    let n = match size {
        1 => 1,
        2 => 2,
        4 => 4,
        _ => 0,
    };
    let mut written = 0u16;
    for (i, b) in bytes.iter().enumerate().take(n) {
        let o = off + i;
        if o == st {
            written |= u16::from(*b);
        } else if o == st + 1 {
            written |= u16::from(*b) << 8;
        }
    }
    written
}

impl PortDevice for PciConfig {
    fn port_read(&mut self, port: u16, size: u8) -> u32 {
        if (0xCF8..=0xCFB).contains(&port) {
            return self.read_address(size, port);
        }
        if (0xCFC..=0xCFF).contains(&port) {
            return self.read_data(size, port);
        }
        // Spec: Intel 82371 / OSDev ELCR — byte ports 0x4D0/0x4D1.
        if port == PIIX_ELCR_MASTER || port == PIIX_ELCR_SLAVE {
            let idx = (port - PIIX_ELCR_MASTER) as usize;
            return match size {
                1 => u32::from(self.elcr[idx]),
                2 if port == PIIX_ELCR_MASTER => u32::from(u16::from_le_bytes(self.elcr)),
                _ => u32::from(self.elcr[idx]),
            };
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
            return;
        }
        // Spec: Intel 82371 / OSDev ELCR — store/readback; DualPic LTIM not wired.
        if port == PIIX_ELCR_MASTER || port == PIIX_ELCR_SLAVE {
            let idx = (port - PIIX_ELCR_MASTER) as usize;
            match size {
                1 => self.elcr[idx] = value as u8,
                2 if port == PIIX_ELCR_MASTER => {
                    self.elcr = (value as u16).to_le_bytes();
                }
                _ => self.elcr[idx] = value as u8,
            }
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
    fn owns_piix_elcr_ports() {
        // Spec: Intel 82371 / OSDev ELCR — 0x4D0/0x4D1 on PIIX ISA path.
        assert!(PciConfig::owns_port(PIIX_ELCR_MASTER));
        assert!(PciConfig::owns_port(PIIX_ELCR_SLAVE));
        assert!(!PciConfig::owns_port(0x4CF));
        assert!(!PciConfig::owns_port(0x4D2));
    }

    /// Spec: Intel 82371 / OSDev 8259 PIC ELCR — SeaBIOS/PIIX programs
    /// `0x4D0`/`0x4D1` for edge/level; store/readback stub (LTIM not wired).
    #[test]
    fn piix_elcr_store_readback_and_reset() {
        let mut pci = PciConfig::new();
        assert_eq!(pci.port_read(PIIX_ELCR_MASTER, 1) as u8, 0x00);
        assert_eq!(pci.port_read(PIIX_ELCR_SLAVE, 1) as u8, 0x00);

        pci.port_write(PIIX_ELCR_MASTER, 1, 0x28);
        pci.port_write(PIIX_ELCR_SLAVE, 1, 0x0C);
        assert_eq!(pci.port_read(PIIX_ELCR_MASTER, 1) as u8, 0x28);
        assert_eq!(pci.port_read(PIIX_ELCR_SLAVE, 1) as u8, 0x0C);
        assert_eq!(pci.elcr, [0x28, 0x0C]);

        // Word access at 0x4D0 covers both ELCR bytes (LE).
        pci.port_write(PIIX_ELCR_MASTER, 2, 0xA5_5A);
        assert_eq!(pci.port_read(PIIX_ELCR_MASTER, 2) as u16, 0xA5_5A);
        assert_eq!(pci.elcr, [0x5A, 0xA5]);

        pci.reset();
        assert_eq!(pci.elcr, [0x00, 0x00]);
        assert_eq!(pci.port_read(PIIX_ELCR_MASTER, 1) as u8, 0x00);
        assert_eq!(pci.port_read(PIIX_ELCR_SLAVE, 1) as u8, 0x00);
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

    /// Spec: PCI Local Bus — Command at `0x04`. Host bridge `00:00.0` stub
    /// keeps IO/MEM/BusMaster sticky (`0x0007`); other Command bits hardwired 0.
    #[test]
    fn host_bridge_command_io_mem_busmaster_sticky() {
        let mut pci = PciConfig::new();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 0, 0, PCI_COMMAND_OFFSET, true),
        );
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            0,
            "Command defaults to 0 at reset"
        );

        // Guest writes all Command bits; only IO|MEM|BusMaster stick.
        pci.port_write(PCI_CONFIG_DATA, 2, 0xFFFF);
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            PCI_HOST_BRIDGE_COMMAND_MASK
        );
        assert_eq!(
            PCI_HOST_BRIDGE_COMMAND_MASK,
            PCI_COMMAND_IO | PCI_COMMAND_MEM | PCI_COMMAND_BUS_MASTER
        );

        // Subset stickiness: MEM|BusMaster without IO.
        pci.port_write(
            PCI_CONFIG_DATA,
            2,
            u32::from(PCI_COMMAND_MEM | PCI_COMMAND_BUS_MASTER | 0x0100), // SERR discarded
        );
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            PCI_COMMAND_MEM | PCI_COMMAND_BUS_MASTER
        );

        // Byte lane at 0xCFC (Command low): junk high bits masked.
        pci.port_write(PCI_CONFIG_DATA, 1, 0xFF);
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            PCI_HOST_BRIDGE_COMMAND_MASK
        );

        pci.reset();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 0, 0, PCI_COMMAND_OFFSET, true),
        );
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 2) as u16, 0);
    }

    /// Spec: PCI Local Bus + Intel 82371SB — Command at `0x04`. PIIX ISA
    /// `00:01.0` stub keeps IO/MEM/BusMaster sticky (`0x0007`), mirroring
    /// `PCI_HOST_BRIDGE_COMMAND_MASK`; other Command bits hardwired 0.
    #[test]
    fn piix_isa_command_io_mem_busmaster_sticky() {
        let mut pci = PciConfig::new();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 0, PCI_COMMAND_OFFSET, true),
        );
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            0,
            "Command defaults to 0 at reset"
        );

        // Guest writes all Command bits; only IO|MEM|BusMaster stick.
        pci.port_write(PCI_CONFIG_DATA, 2, 0xFFFF);
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            PCI_PIIX_ISA_COMMAND_MASK
        );
        assert_eq!(
            PCI_PIIX_ISA_COMMAND_MASK,
            PCI_COMMAND_IO | PCI_COMMAND_MEM | PCI_COMMAND_BUS_MASTER
        );
        assert_eq!(
            PCI_PIIX_ISA_COMMAND_MASK, PCI_HOST_BRIDGE_COMMAND_MASK,
            "ISA Command mask mirrors host bridge"
        );

        // Subset stickiness: MEM|BusMaster without IO; SERR discarded.
        pci.port_write(
            PCI_CONFIG_DATA,
            2,
            u32::from(PCI_COMMAND_MEM | PCI_COMMAND_BUS_MASTER | 0x0100),
        );
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            PCI_COMMAND_MEM | PCI_COMMAND_BUS_MASTER
        );

        // Byte lane at 0xCFC (Command low): junk high bits masked.
        pci.port_write(PCI_CONFIG_DATA, 1, 0xFF);
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            PCI_PIIX_ISA_COMMAND_MASK
        );

        // Wider write that previously stuck unmasked must now drop non-sticky bits.
        pci.port_write(PCI_CONFIG_DATA, 2, 0x0147);
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            PCI_COMMAND_IO | PCI_COMMAND_MEM | PCI_COMMAND_BUS_MASTER
        );

        pci.reset();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 0, PCI_COMMAND_OFFSET, true),
        );
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 2) as u16, 0);
    }

    /// PIIX ISA Command mask must not change host-bridge or IDE Command.
    #[test]
    fn piix_isa_command_mask_does_not_affect_other_functions() {
        let mut pci = PciConfig::new();

        // Host bridge still uses IO|MEM|BusMaster mask.
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 0, 0, PCI_COMMAND_OFFSET, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 2, 0xFFFF);
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            PCI_HOST_BRIDGE_COMMAND_MASK
        );

        // IDE still uses IO|BusMaster only (no MEM).
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 1, PCI_COMMAND_OFFSET, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 2, 0xFFFF);
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            PCI_PIIX_IDE_COMMAND_MASK
        );
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16 & PCI_COMMAND_MEM,
            0
        );
    }

    /// Spec: PCI Local Bus — Command at `0x04`. PIIX IDE `00:01.1` stub keeps
    /// IO/BusMaster sticky (`0x0005`); MEM and other Command bits hardwired 0.
    #[test]
    fn piix_ide_command_io_busmaster_sticky() {
        let mut pci = PciConfig::new();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 1, PCI_COMMAND_OFFSET, true),
        );
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            0,
            "Command defaults to 0 at reset"
        );

        // Guest writes all Command bits; only IO|BusMaster stick (no MEM).
        pci.port_write(PCI_CONFIG_DATA, 2, 0xFFFF);
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            PCI_PIIX_IDE_COMMAND_MASK
        );
        assert_eq!(
            PCI_PIIX_IDE_COMMAND_MASK,
            PCI_COMMAND_IO | PCI_COMMAND_BUS_MASTER
        );
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16 & PCI_COMMAND_MEM,
            0,
            "MEM Space Enable hardwired 0 on PIIX IDE"
        );

        // Subset stickiness: BusMaster without IO; SERR discarded.
        pci.port_write(
            PCI_CONFIG_DATA,
            2,
            u32::from(PCI_COMMAND_BUS_MASTER | 0x0100),
        );
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            PCI_COMMAND_BUS_MASTER
        );

        // Byte lane at 0xCFC (Command low): junk high bits masked.
        pci.port_write(PCI_CONFIG_DATA, 1, 0xFF);
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            PCI_PIIX_IDE_COMMAND_MASK
        );

        pci.reset();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 1, PCI_COMMAND_OFFSET, true),
        );
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 2) as u16, 0);
    }

    /// PIIX IDE Command mask must not change host-bridge or ISA Command.
    #[test]
    fn piix_ide_command_mask_does_not_affect_other_functions() {
        let mut pci = PciConfig::new();

        // Host bridge still uses IO|MEM|BusMaster mask.
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 0, 0, PCI_COMMAND_OFFSET, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 2, 0xFFFF);
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            PCI_HOST_BRIDGE_COMMAND_MASK
        );

        // ISA uses its own IO|MEM|BusMaster sticky mask (same bits as host).
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 0, PCI_COMMAND_OFFSET, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 2, 0x0147);
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            PCI_PIIX_ISA_COMMAND_MASK
        );
    }

    /// Spec: PCI Local Bus + Intel 82371SB — Command at `0x04`. PIIX USB UHCI
    /// `00:01.2` stub keeps IO/MEM/BusMaster sticky (`0x0007`), mirroring
    /// `PCI_HOST_BRIDGE_COMMAND_MASK`; other Command bits hardwired 0.
    #[test]
    fn piix_usb_command_io_mem_busmaster_sticky() {
        let mut pci = PciConfig::new();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 2, PCI_COMMAND_OFFSET, true),
        );
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            0,
            "Command defaults to 0 at reset"
        );

        // Guest writes all Command bits; only IO|MEM|BusMaster stick.
        pci.port_write(PCI_CONFIG_DATA, 2, 0xFFFF);
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            PCI_PIIX_USB_COMMAND_MASK
        );
        assert_eq!(
            PCI_PIIX_USB_COMMAND_MASK,
            PCI_COMMAND_IO | PCI_COMMAND_MEM | PCI_COMMAND_BUS_MASTER
        );
        assert_eq!(
            PCI_PIIX_USB_COMMAND_MASK, PCI_HOST_BRIDGE_COMMAND_MASK,
            "USB Command mask mirrors host bridge"
        );

        // Subset stickiness: MEM|BusMaster without IO; SERR discarded.
        pci.port_write(
            PCI_CONFIG_DATA,
            2,
            u32::from(PCI_COMMAND_MEM | PCI_COMMAND_BUS_MASTER | 0x0100),
        );
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            PCI_COMMAND_MEM | PCI_COMMAND_BUS_MASTER
        );

        // Byte lane at 0xCFC (Command low): junk high bits masked.
        pci.port_write(PCI_CONFIG_DATA, 1, 0xFF);
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            PCI_PIIX_USB_COMMAND_MASK
        );

        // Wider write that previously stuck unmasked must now drop non-sticky bits.
        pci.port_write(PCI_CONFIG_DATA, 2, 0x0147);
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            PCI_COMMAND_IO | PCI_COMMAND_MEM | PCI_COMMAND_BUS_MASTER
        );

        pci.reset();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 2, PCI_COMMAND_OFFSET, true),
        );
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 2) as u16, 0);
    }

    /// PIIX USB Command mask must not change host-bridge, ISA, or IDE Command.
    #[test]
    fn piix_usb_command_mask_does_not_affect_other_functions() {
        let mut pci = PciConfig::new();

        // Host bridge still uses IO|MEM|BusMaster mask.
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 0, 0, PCI_COMMAND_OFFSET, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 2, 0xFFFF);
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            PCI_HOST_BRIDGE_COMMAND_MASK
        );

        // ISA still uses IO|MEM|BusMaster.
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 0, PCI_COMMAND_OFFSET, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 2, 0xFFFF);
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            PCI_PIIX_ISA_COMMAND_MASK
        );

        // IDE still uses IO|BusMaster only (no MEM).
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 1, PCI_COMMAND_OFFSET, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 2, 0xFFFF);
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            PCI_PIIX_IDE_COMMAND_MASK
        );
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16 & PCI_COMMAND_MEM,
            0
        );
    }

    /// Spec: PCI Local Bus + Intel 82371AB — Command at `0x04`. PIIX ACPI
    /// `00:01.3` stub keeps IO/MEM/BusMaster sticky (`0x0007`), mirroring
    /// `PCI_HOST_BRIDGE_COMMAND_MASK`; other Command bits hardwired 0.
    #[test]
    fn piix_acpi_command_io_mem_busmaster_sticky() {
        let mut pci = PciConfig::new();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 3, PCI_COMMAND_OFFSET, true),
        );
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            0,
            "Command defaults to 0 at reset"
        );

        // Guest writes all Command bits; only IO|MEM|BusMaster stick.
        pci.port_write(PCI_CONFIG_DATA, 2, 0xFFFF);
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            PCI_PIIX_ACPI_COMMAND_MASK
        );
        assert_eq!(
            PCI_PIIX_ACPI_COMMAND_MASK,
            PCI_COMMAND_IO | PCI_COMMAND_MEM | PCI_COMMAND_BUS_MASTER
        );
        assert_eq!(
            PCI_PIIX_ACPI_COMMAND_MASK, PCI_HOST_BRIDGE_COMMAND_MASK,
            "ACPI Command mask mirrors host bridge"
        );

        // Subset stickiness: MEM|BusMaster without IO; SERR discarded.
        pci.port_write(
            PCI_CONFIG_DATA,
            2,
            u32::from(PCI_COMMAND_MEM | PCI_COMMAND_BUS_MASTER | 0x0100),
        );
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            PCI_COMMAND_MEM | PCI_COMMAND_BUS_MASTER
        );

        // Byte lane at 0xCFC (Command low): junk high bits masked.
        pci.port_write(PCI_CONFIG_DATA, 1, 0xFF);
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            PCI_PIIX_ACPI_COMMAND_MASK
        );

        // Wider write that previously stuck unmasked must now drop non-sticky bits.
        pci.port_write(PCI_CONFIG_DATA, 2, 0x0147);
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            PCI_COMMAND_IO | PCI_COMMAND_MEM | PCI_COMMAND_BUS_MASTER
        );

        pci.reset();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 3, PCI_COMMAND_OFFSET, true),
        );
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 2) as u16, 0);
    }

    /// PIIX ACPI Command mask must not change host-bridge, ISA, IDE, or USB Command.
    #[test]
    fn piix_acpi_command_mask_does_not_affect_other_functions() {
        let mut pci = PciConfig::new();

        // Host bridge still uses IO|MEM|BusMaster mask.
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 0, 0, PCI_COMMAND_OFFSET, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 2, 0xFFFF);
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            PCI_HOST_BRIDGE_COMMAND_MASK
        );

        // ISA still uses IO|MEM|BusMaster.
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 0, PCI_COMMAND_OFFSET, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 2, 0xFFFF);
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            PCI_PIIX_ISA_COMMAND_MASK
        );

        // IDE still uses IO|BusMaster only (no MEM).
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 1, PCI_COMMAND_OFFSET, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 2, 0xFFFF);
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            PCI_PIIX_IDE_COMMAND_MASK
        );
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16 & PCI_COMMAND_MEM,
            0
        );

        // USB still uses IO|MEM|BusMaster.
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 2, PCI_COMMAND_OFFSET, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 2, 0xFFFF);
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 2) as u16,
            PCI_PIIX_USB_COMMAND_MASK
        );
    }

    /// Spec: PCI Local Bus — Status at `0x06`. Host bridge stub CapList=0,
    /// FastB2B=1, DevSel=medium (`PCI_HOST_BRIDGE_STATUS_STUB` = `0x0280`).
    /// Access via dword base `0x04` + CONFIG_DATA lane `0xCFE`.
    #[test]
    fn host_bridge_status_caplist_fastb2b_devsel_stub() {
        let mut pci = PciConfig::new();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 0, 0, PCI_COMMAND_OFFSET, true),
        );
        let status = pci.port_read(0xCFE, 2) as u16;
        assert_eq!(status, PCI_HOST_BRIDGE_STATUS_STUB);
        assert_eq!(
            status & PCI_STATUS_CAP_LIST,
            0,
            "CapList hardwired 0 (no caps)"
        );
        assert_ne!(status & PCI_STATUS_FAST_BACK, 0, "FastB2B hardwired 1");
        assert_eq!(
            status & PCI_STATUS_DEVSEL_MASK,
            PCI_STATUS_DEVSEL_MEDIUM,
            "DevSel=medium"
        );
        assert_eq!(
            PCI_HOST_BRIDGE_STATUS_STUB,
            PCI_STATUS_FAST_BACK | PCI_STATUS_DEVSEL_MEDIUM
        );

        // Guest cannot set CapList or change DevSel/FastB2B via config write.
        pci.port_write(0xCFE, 2, 0xFFFF);
        assert_eq!(pci.port_read(0xCFE, 2) as u16, PCI_HOST_BRIDGE_STATUS_STUB);

        pci.reset();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 0, 0, PCI_COMMAND_OFFSET, true),
        );
        assert_eq!(pci.port_read(0xCFE, 2) as u16, PCI_HOST_BRIDGE_STATUS_STUB);
    }

    /// Spec: PCI Status RW1C — write-1 clears MDPE/STA/RTA/RMA/SSE/DPE; write-0 keeps.
    #[test]
    fn host_bridge_status_rw1c_error_bits() {
        let mut pci = PciConfig::new();
        let st = PCI_STATUS_OFFSET as usize;
        let injected =
            PCI_HOST_BRIDGE_STATUS_STUB | PCI_STATUS_REC_MASTER_ABORT | PCI_STATUS_DETECTED_PARITY;
        pci.host_bridge[st..st + 2].copy_from_slice(&injected.to_le_bytes());

        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 0, 0, PCI_COMMAND_OFFSET, true),
        );
        assert_eq!(pci.port_read(0xCFE, 2) as u16, injected);

        // Write-0 to RMA keeps it; write-1 to DPE clears only DPE.
        pci.port_write(0xCFE, 2, u32::from(PCI_STATUS_DETECTED_PARITY));
        assert_eq!(
            pci.port_read(0xCFE, 2) as u16,
            PCI_HOST_BRIDGE_STATUS_STUB | PCI_STATUS_REC_MASTER_ABORT
        );

        // Clear remaining RW1C with 0xFFFF; hardwired stub bits remain.
        pci.port_write(0xCFE, 2, 0xFFFF);
        assert_eq!(pci.port_read(0xCFE, 2) as u16, PCI_HOST_BRIDGE_STATUS_STUB);
    }

    /// Spec: PCI Local Bus — Status at `0x06`. PIIX ISA stub CapList=0,
    /// FastB2B=1, DevSel=medium (`PCI_PIIX_ISA_STATUS_STUB` = `0x0280`).
    /// Access via dword base `0x04` + CONFIG_DATA lane `0xCFE`.
    #[test]
    fn piix_isa_status_caplist_fastb2b_devsel_stub() {
        let mut pci = PciConfig::new();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 0, PCI_COMMAND_OFFSET, true),
        );
        let status = pci.port_read(0xCFE, 2) as u16;
        assert_eq!(status, PCI_PIIX_ISA_STATUS_STUB);
        assert_eq!(
            status & PCI_STATUS_CAP_LIST,
            0,
            "CapList hardwired 0 (no caps)"
        );
        assert_ne!(status & PCI_STATUS_FAST_BACK, 0, "FastB2B hardwired 1");
        assert_eq!(
            status & PCI_STATUS_DEVSEL_MASK,
            PCI_STATUS_DEVSEL_MEDIUM,
            "DevSel=medium"
        );
        assert_eq!(PCI_PIIX_ISA_STATUS_STUB, PCI_HOST_BRIDGE_STATUS_STUB);
        assert_eq!(PCI_PIIX_ISA_STATUS_STUB, PCI_PIIX_IDE_STATUS_STUB);

        // Guest cannot set CapList or change DevSel/FastB2B via config write.
        pci.port_write(0xCFE, 2, 0xFFFF);
        assert_eq!(pci.port_read(0xCFE, 2) as u16, PCI_PIIX_ISA_STATUS_STUB);

        pci.reset();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 0, PCI_COMMAND_OFFSET, true),
        );
        assert_eq!(pci.port_read(0xCFE, 2) as u16, PCI_PIIX_ISA_STATUS_STUB);
    }

    /// Spec: PCI Status RW1C — write-1 clears MDPE/STA/RTA/RMA/SSE/DPE on PIIX ISA.
    #[test]
    fn piix_isa_status_rw1c_error_bits() {
        let mut pci = PciConfig::new();
        let st = PCI_STATUS_OFFSET as usize;
        let injected =
            PCI_PIIX_ISA_STATUS_STUB | PCI_STATUS_REC_MASTER_ABORT | PCI_STATUS_DETECTED_PARITY;
        pci.piix_isa[st..st + 2].copy_from_slice(&injected.to_le_bytes());

        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 0, PCI_COMMAND_OFFSET, true),
        );
        assert_eq!(pci.port_read(0xCFE, 2) as u16, injected);

        // Write-0 to RMA keeps it; write-1 to DPE clears only DPE.
        pci.port_write(0xCFE, 2, u32::from(PCI_STATUS_DETECTED_PARITY));
        assert_eq!(
            pci.port_read(0xCFE, 2) as u16,
            PCI_PIIX_ISA_STATUS_STUB | PCI_STATUS_REC_MASTER_ABORT
        );

        // Clear remaining RW1C with 0xFFFF; hardwired stub bits remain.
        pci.port_write(0xCFE, 2, 0xFFFF);
        assert_eq!(pci.port_read(0xCFE, 2) as u16, PCI_PIIX_ISA_STATUS_STUB);
    }

    /// Spec: PCI Local Bus — Status at `0x06`. PIIX IDE stub CapList=0,
    /// FastB2B=1, DevSel=medium (`PCI_PIIX_IDE_STATUS_STUB` = `0x0280`).
    /// Access via dword base `0x04` + CONFIG_DATA lane `0xCFE`.
    #[test]
    fn piix_ide_status_caplist_fastb2b_devsel_stub() {
        let mut pci = PciConfig::new();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 1, PCI_COMMAND_OFFSET, true),
        );
        let status = pci.port_read(0xCFE, 2) as u16;
        assert_eq!(status, PCI_PIIX_IDE_STATUS_STUB);
        assert_eq!(
            status & PCI_STATUS_CAP_LIST,
            0,
            "CapList hardwired 0 (no caps)"
        );
        assert_ne!(status & PCI_STATUS_FAST_BACK, 0, "FastB2B hardwired 1");
        assert_eq!(
            status & PCI_STATUS_DEVSEL_MASK,
            PCI_STATUS_DEVSEL_MEDIUM,
            "DevSel=medium"
        );
        assert_eq!(PCI_PIIX_IDE_STATUS_STUB, PCI_HOST_BRIDGE_STATUS_STUB);

        // Guest cannot set CapList or change DevSel/FastB2B via config write.
        pci.port_write(0xCFE, 2, 0xFFFF);
        assert_eq!(pci.port_read(0xCFE, 2) as u16, PCI_PIIX_IDE_STATUS_STUB);

        pci.reset();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 1, PCI_COMMAND_OFFSET, true),
        );
        assert_eq!(pci.port_read(0xCFE, 2) as u16, PCI_PIIX_IDE_STATUS_STUB);
    }

    /// Spec: PCI Status RW1C — write-1 clears MDPE/STA/RTA/RMA/SSE/DPE on PIIX IDE.
    #[test]
    fn piix_ide_status_rw1c_error_bits() {
        let mut pci = PciConfig::new();
        let st = PCI_STATUS_OFFSET as usize;
        let injected =
            PCI_PIIX_IDE_STATUS_STUB | PCI_STATUS_REC_MASTER_ABORT | PCI_STATUS_DETECTED_PARITY;
        pci.piix_ide[st..st + 2].copy_from_slice(&injected.to_le_bytes());

        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 1, PCI_COMMAND_OFFSET, true),
        );
        assert_eq!(pci.port_read(0xCFE, 2) as u16, injected);

        // Write-0 to RMA keeps it; write-1 to DPE clears only DPE.
        pci.port_write(0xCFE, 2, u32::from(PCI_STATUS_DETECTED_PARITY));
        assert_eq!(
            pci.port_read(0xCFE, 2) as u16,
            PCI_PIIX_IDE_STATUS_STUB | PCI_STATUS_REC_MASTER_ABORT
        );

        // Clear remaining RW1C with 0xFFFF; hardwired stub bits remain.
        pci.port_write(0xCFE, 2, 0xFFFF);
        assert_eq!(pci.port_read(0xCFE, 2) as u16, PCI_PIIX_IDE_STATUS_STUB);
    }

    /// Spec: PCI Local Bus — Status at `0x06`. PIIX USB UHCI stub CapList=0,
    /// FastB2B=1, DevSel=medium (`PCI_PIIX_USB_STATUS_STUB` = `0x0280`).
    /// Access via dword base `0x04` + CONFIG_DATA lane `0xCFE`.
    #[test]
    fn piix_usb_status_caplist_fastb2b_devsel_stub() {
        let mut pci = PciConfig::new();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 2, PCI_COMMAND_OFFSET, true),
        );
        let status = pci.port_read(0xCFE, 2) as u16;
        assert_eq!(status, PCI_PIIX_USB_STATUS_STUB);
        assert_eq!(
            status & PCI_STATUS_CAP_LIST,
            0,
            "CapList hardwired 0 (no caps)"
        );
        assert_ne!(status & PCI_STATUS_FAST_BACK, 0, "FastB2B hardwired 1");
        assert_eq!(
            status & PCI_STATUS_DEVSEL_MASK,
            PCI_STATUS_DEVSEL_MEDIUM,
            "DevSel=medium"
        );
        assert_eq!(PCI_PIIX_USB_STATUS_STUB, PCI_HOST_BRIDGE_STATUS_STUB);
        assert_eq!(PCI_PIIX_USB_STATUS_STUB, PCI_PIIX_IDE_STATUS_STUB);
        assert_eq!(PCI_PIIX_USB_STATUS_STUB, PCI_PIIX_ISA_STATUS_STUB);

        // Guest cannot set CapList or change DevSel/FastB2B via config write.
        pci.port_write(0xCFE, 2, 0xFFFF);
        assert_eq!(pci.port_read(0xCFE, 2) as u16, PCI_PIIX_USB_STATUS_STUB);

        pci.reset();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 2, PCI_COMMAND_OFFSET, true),
        );
        assert_eq!(pci.port_read(0xCFE, 2) as u16, PCI_PIIX_USB_STATUS_STUB);
    }

    /// Spec: PCI Status RW1C — write-1 clears MDPE/STA/RTA/RMA/SSE/DPE on PIIX USB.
    #[test]
    fn piix_usb_status_rw1c_error_bits() {
        let mut pci = PciConfig::new();
        let st = PCI_STATUS_OFFSET as usize;
        let injected =
            PCI_PIIX_USB_STATUS_STUB | PCI_STATUS_REC_MASTER_ABORT | PCI_STATUS_DETECTED_PARITY;
        pci.piix_usb[st..st + 2].copy_from_slice(&injected.to_le_bytes());

        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 2, PCI_COMMAND_OFFSET, true),
        );
        assert_eq!(pci.port_read(0xCFE, 2) as u16, injected);

        // Write-0 to RMA keeps it; write-1 to DPE clears only DPE.
        pci.port_write(0xCFE, 2, u32::from(PCI_STATUS_DETECTED_PARITY));
        assert_eq!(
            pci.port_read(0xCFE, 2) as u16,
            PCI_PIIX_USB_STATUS_STUB | PCI_STATUS_REC_MASTER_ABORT
        );

        // Clear remaining RW1C with 0xFFFF; hardwired stub bits remain.
        pci.port_write(0xCFE, 2, 0xFFFF);
        assert_eq!(pci.port_read(0xCFE, 2) as u16, PCI_PIIX_USB_STATUS_STUB);
    }

    /// Spec: PCI Local Bus — Status at `0x06`. PIIX ACPI stub CapList=0,
    /// FastB2B=1, DevSel=medium (`PCI_PIIX_ACPI_STATUS_STUB` = `0x0280`).
    /// Access via dword base `0x04` + CONFIG_DATA lane `0xCFE`.
    #[test]
    fn piix_acpi_status_caplist_fastb2b_devsel_stub() {
        let mut pci = PciConfig::new();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 3, PCI_COMMAND_OFFSET, true),
        );
        let status = pci.port_read(0xCFE, 2) as u16;
        assert_eq!(status, PCI_PIIX_ACPI_STATUS_STUB);
        assert_eq!(
            status & PCI_STATUS_CAP_LIST,
            0,
            "CapList hardwired 0 (no caps)"
        );
        assert_ne!(status & PCI_STATUS_FAST_BACK, 0, "FastB2B hardwired 1");
        assert_eq!(
            status & PCI_STATUS_DEVSEL_MASK,
            PCI_STATUS_DEVSEL_MEDIUM,
            "DevSel=medium"
        );
        assert_eq!(PCI_PIIX_ACPI_STATUS_STUB, PCI_HOST_BRIDGE_STATUS_STUB);
        assert_eq!(PCI_PIIX_ACPI_STATUS_STUB, PCI_PIIX_IDE_STATUS_STUB);
        assert_eq!(PCI_PIIX_ACPI_STATUS_STUB, PCI_PIIX_ISA_STATUS_STUB);
        assert_eq!(PCI_PIIX_ACPI_STATUS_STUB, PCI_PIIX_USB_STATUS_STUB);

        // Guest cannot set CapList or change DevSel/FastB2B via config write.
        pci.port_write(0xCFE, 2, 0xFFFF);
        assert_eq!(pci.port_read(0xCFE, 2) as u16, PCI_PIIX_ACPI_STATUS_STUB);

        pci.reset();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 3, PCI_COMMAND_OFFSET, true),
        );
        assert_eq!(pci.port_read(0xCFE, 2) as u16, PCI_PIIX_ACPI_STATUS_STUB);
    }

    /// Spec: PCI Status RW1C — write-1 clears MDPE/STA/RTA/RMA/SSE/DPE on PIIX ACPI.
    #[test]
    fn piix_acpi_status_rw1c_error_bits() {
        let mut pci = PciConfig::new();
        let st = PCI_STATUS_OFFSET as usize;
        let injected =
            PCI_PIIX_ACPI_STATUS_STUB | PCI_STATUS_REC_MASTER_ABORT | PCI_STATUS_DETECTED_PARITY;
        pci.piix_acpi[st..st + 2].copy_from_slice(&injected.to_le_bytes());

        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 3, PCI_COMMAND_OFFSET, true),
        );
        assert_eq!(pci.port_read(0xCFE, 2) as u16, injected);

        // Write-0 to RMA keeps it; write-1 to DPE clears only DPE.
        pci.port_write(0xCFE, 2, u32::from(PCI_STATUS_DETECTED_PARITY));
        assert_eq!(
            pci.port_read(0xCFE, 2) as u16,
            PCI_PIIX_ACPI_STATUS_STUB | PCI_STATUS_REC_MASTER_ABORT
        );

        // Clear remaining RW1C with 0xFFFF; hardwired stub bits remain.
        pci.port_write(0xCFE, 2, 0xFFFF);
        assert_eq!(pci.port_read(0xCFE, 2) as u16, PCI_PIIX_ACPI_STATUS_STUB);
    }

    /// Spec: UHCI / PIIX USB — LEGSUP dword at config `0xC0` store/readback
    /// (legacy keyboard/mouse SMI routing not modeled).
    #[test]
    fn piix_usb_legsup_dword_store_readback() {
        let mut pci = PciConfig::new();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 2, PCI_PIIX_USB_LEGSUP_OFFSET, true),
        );
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 4), 0);
        pci.port_write(PCI_CONFIG_DATA, 4, 0x2000_0000);
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 4), 0x2000_0000);
        pci.port_write(PCI_CONFIG_DATA, 4, 0x0000_0000);
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 4), 0);
    }

    /// Spec: Intel 82371SB — IDE IDETIM at config `0x40` word store/readback
    /// (timing decode not modeled).
    #[test]
    fn piix_ide_idetim_word_store_readback() {
        let mut pci = PciConfig::new();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 1, PCI_PIIX_IDE_IDETIM_OFFSET, true),
        );
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 2) as u16, 0);
        pci.port_write(PCI_CONFIG_DATA, 2, 0xA307);
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 2) as u16, 0xA307);
        pci.port_write(PCI_CONFIG_DATA, 2, 0x0000);
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 2) as u16, 0);
    }

    /// Spec: Intel 82371AB — PMBASE at ACPI config `0x40` is an I/O BAR
    /// (bit0=1); bits 15:6 hold the 64-byte-aligned I/O base.
    #[test]
    fn piix_acpi_pmbase_io_bar_store_readback() {
        let mut pci = PciConfig::new();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 3, PCI_PIIX_ACPI_PMBASE_OFFSET, true),
        );
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 4), 0);
        pci.port_write(PCI_CONFIG_DATA, 4, 0x0000_B03E);
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 4),
            0x0000_B001,
            "PMBASE: bits15:6 kept, bit0=1, bits5:1=0"
        );
        pci.port_write(PCI_CONFIG_DATA, 4, 0x0000_4000);
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 4), 0x0000_4001);
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
