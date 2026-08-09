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
//! - Intel 440FX PCIset 82441FX (PMC) datasheet §3.2.18 "PAM — Programmable
//!   Attribute Map Registers (PAM[6:0])", Table 2 "Attribute Bit Assignment"
//!   and Table 3 "PAM Registers and Associated Memory Segments" — the shadow
//!   RAM attribute register block at PMC config `0x59`–`0x5F`.
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
//! - Host bridge Cache Line Size (`0x0C`) byte store/readback (reset `0x00`; no
//!   cache/burst side effects yet).
//! - Host bridge Latency Timer (`0x0D`) byte store/readback (reset `0x00`; no
//!   arbitration side effects yet).
//! - i440FX PMC Programmable Attribute Map registers PAM0–PAM6 (`0x59`–`0x5F`):
//!   store/readback of the RE/WE fields, reset default `0x00`, reserved bits
//!   ([7, 6, 3, 2] and `PAM0[3:0]`) hardwired 0, plus the decoded host accessors
//!   [`PciConfig::pam_regions`] / [`PciConfig::pam_region_for_addr`] reporting
//!   read-from-DRAM and write-to-DRAM per Table 3 segment. The register file is
//!   standalone — nothing in this crate steers memory from it.
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
//!   Reserved IRQ0/1/2/8/13 bits are hardwired 0 (always edge) via
//!   [`PIIX_ELCR_MASTER_WRITABLE`] / [`PIIX_ELCR_SLAVE_WRITABLE`].
//!   `MachineBus` syncs writes into `DualPic::set_elcr_level_mask` (per-IR level
//!   vs edge; OR'd with ICW1.LTIM inside `Pic8259`).
//! - PIIX ISA PIRQRC `0x60`–`0x63`: store/readback (default `0x80` disabled).
//!   When bit7 is clear, bits 3:0 select the ISA IRQ for that PIRQ; software
//!   [`PciConfig::set_pirq_line`] / `Machine::assert_pirq` (PCI INTx stub)
//!   drives `DualPic` via [`PciConfig::sync_pirq_to_pic`]. Not a full PCI
//!   device interrupt storm (IDE/UHCI engines remain unwired).
//!
//! - PIIX IDE BMIDE I/O BAR decode: when Command.IO is set and BMIBA has I/O
//!   form (bit0), the 16-byte Bus Master IDE register block at
//!   `BMIBA & 0xFFF0` is a noop store/readback (command/status/PRD pointers;
//!   primary + secondary).
//! - Bounded BMIDE PRD stubs on the primary channel, both directions, when
//!   Command.BusMaster is set: [`PciConfig::start_bm_read`] /
//!   [`PciConfig::run_prd_read_stub`] walk an EOT-terminated PRD table at
//!   BMIDTP and split a supplied device buffer across its memory regions
//!   (RWCON cleared), and [`PciConfig::start_bm_write`] /
//!   [`PciConfig::run_prd_write_stub`] walk the same table in the opposite
//!   direction to fill a device buffer from those regions (RWCON set). Both
//!   honor zero count = 64 KiB, the 256-entry missing-EOT cap, 32-bit
//!   address-wrap rejection, and BMISTA Active/Error latching. No ATA DMA
//!   command engine and no secondary-channel engine.
//! - PIIX ACPI PM I/O decode: when Command.IO is set and PMBASE has I/O form
//!   (bit0), the 64-byte PM register block at `PMBASE & 0xFFC0` is a noop
//!   store/readback (`PM1a_EVT` / `PM1a_CNT` / `PM_TMR` + remainder). No SCI,
//!   SMI, or power-state machine.
//!
//! - PIIX USB UHCI BAR0 I/O decode: when Command.IO is set and UHCI BAR0 has
//!   I/O form (bit0), the 32-byte UHCI register block at `BAR0 & 0xFFE0` is a
//!   noop store/readback (USBCMD/USBSTS/USBINTR/FRNUM/FLBASEADD/SOFMOD/PORTSC).
//!   No host-controller schedule/DMA engine. LEGSUP remains PCI config `0xC0`.
//!
//! # Unsupported (explicit)
//!
//! - BAR MMIO decode (other than PIIX IDE BMIDE / ACPI PM I/O stubs above), full
//!   BMIDE / ATA READ|WRITE DMA engine, full PCI device INTx storm (IDE/UHCI);
//!   PIRQRC software `assert_pirq` stub only
//! - Secondary-channel PRDT walking (BMIDTP at `0x0C` is store/readback only),
//!   BMIDE interrupt (BMISTA bit2) generation, and PRD-driven IDE task-file
//!   sequencing: both directions are host-called walkers, not a DMA engine
//!   started by an ATA command or by a guest write to BMICOM.SSBM
//! - Host-bridge / PIIX ISA / PIIX USB Command decode side effects;
//!   PIIX IDE Command side effects beyond BMIDE I/O enable;
//!   PIIX ACPI Command side effects beyond PM I/O enable
//! - Status error *signaling* (host / ISA / IDE / USB / ACPI never latch RW1C bits from real aborts yet)
//! - Capability list walk (CapList hardwired 0 on host / ISA / IDE / USB / ACPI)
//! - USB host controller (UHCI frame list / ports / IRQ)
//! - ACPI SCI/SMI / GPE / real power transitions / ACPI tables
//! - Capability lists, MSI, PCIe, hotplug
//! - IDE BARs tied to `IdePrimary` ports (legacy fixed ports remain)
//! - PAM *effect*: programming PAM0–PAM6 changes only this register file and
//!   the decoded accessors. No physical memory is remapped here, so BIOS
//!   shadowing does not work until the machine layer consumes
//!   [`PciConfig::pam_regions`]. The neighbouring PMC registers that complete
//!   the legacy memory map — FDHC (`0x68`, the `080000-09FFFFh` DRAM hole) and
//!   SMRAM (`0x72`) — are plain read/write config bytes with no decode.

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
/// PCI Cache Line Size config offset (Type 0 header byte).
/// Spec: PCI Local Bus — Cache Line Size at `0x0C` (units of 32-bit DWORDs).
/// This stub stores/reads back the byte; cache-line / burst side effects are
/// out of scope.
pub const PCI_CACHE_LINE_SIZE_OFFSET: u8 = 0x0C;
/// Host bridge Cache Line Size reset default.
pub const PCI_HOST_BRIDGE_CACHE_LINE_SIZE_DEFAULT: u8 = 0x00;
const _: () = assert!(PCI_CACHE_LINE_SIZE_OFFSET == 0x0C);
/// PCI Latency Timer config offset (Type 0 header byte).
/// Spec: PCI Local Bus — Latency Timer at `0x0D` (bus master grant timer, in
/// PCI clocks / 8). This stub stores/reads back the byte; arbitration timing
/// side effects are out of scope.
pub const PCI_LATENCY_TIMER_OFFSET: u8 = 0x0D;
/// Host bridge Latency Timer reset default.
pub const PCI_HOST_BRIDGE_LATENCY_TIMER_DEFAULT: u8 = 0x00;
const _: () = assert!(PCI_LATENCY_TIMER_OFFSET == 0x0D);
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
/// Bus Master IDE I/O register block size at BMIBA.
/// Spec: Intel 82371SB — primary + secondary command/status/PRD (16 bytes).
pub const PCI_PIIX_IDE_BMIDE_IO_SIZE: u16 = 16;
/// Primary Bus Master IDE Command (BMICOM) offset within BMIBA.
pub const PCI_PIIX_IDE_BMICOM_PRIMARY: u8 = 0x00;
/// Primary Bus Master IDE Status (BMISTA) offset within BMIBA.
pub const PCI_PIIX_IDE_BMISTA_PRIMARY: u8 = 0x02;
/// Primary Bus Master IDE Descriptor Table Pointer (BMIDTP) offset within BMIBA.
pub const PCI_PIIX_IDE_BMIDTP_PRIMARY: u8 = 0x04;
/// Secondary Bus Master IDE Command offset within BMIBA.
pub const PCI_PIIX_IDE_BMICOM_SECONDARY: u8 = 0x08;
/// Secondary Bus Master IDE Status offset within BMIBA.
pub const PCI_PIIX_IDE_BMISTA_SECONDARY: u8 = 0x0A;
/// Secondary Bus Master IDE Descriptor Table Pointer offset within BMIBA.
pub const PCI_PIIX_IDE_BMIDTP_SECONDARY: u8 = 0x0C;
/// BMICOM Start/Stop Bus Master (SSBM) — bit 0.
/// Spec: Intel 82371SB §2.7.1 / Programming Interface for Bus Master IDE.
pub const PCI_PIIX_IDE_BMICOM_SSBM: u8 = 1 << 0;
/// BMICOM Read/Write Control (RWCON) — bit 3; `0` = Read (IDE→memory), `1` = Write.
/// Spec: Intel 82371SB §2.7.1.
pub const PCI_PIIX_IDE_BMICOM_RWCON: u8 = 1 << 3;
/// BMISTA Bus Master IDE Active (BMIDEA) — bit 0 (RO on silicon).
/// Spec: Intel 82371SB §2.7.2.
pub const PCI_PIIX_IDE_BMISTA_ACTIVE: u8 = 1 << 0;
/// BMISTA DMA Error — bit 1, latched when the bounded PRD walk fails.
/// Spec: Intel 82371SB §2.7.2.
pub const PCI_PIIX_IDE_BMISTA_ERROR: u8 = 1 << 1;
/// Physical Region Descriptor entry size (8 bytes).
/// Spec: Intel Programming Interface for Bus Master IDE Controller Rev 1.0 §1.2.
pub const PCI_PIIX_IDE_PRD_ENTRY_SIZE: usize = 8;
/// PRD End of Table (EOT / EOL) — bit 7 of the last byte (bit 31 of dword 1).
/// Spec: Intel Programming Interface for Bus Master IDE Controller Rev 1.0 §1.2;
/// Intel 82371SB — Physical Region Descriptor Format.
pub const PCI_PIIX_IDE_PRD_EOT: u8 = 1 << 7;
/// Zero byte-count field in a PRD means 64 KiB.
/// Spec: Intel Programming Interface for Bus Master IDE Controller Rev 1.0 §1.2.
pub const PCI_PIIX_IDE_PRD_BYTE_COUNT_64K: u32 = 0x1_0000;
/// Emulator safety cap for a malformed PRD table with no EOT marker.
///
/// This is a deterministic software bound, not a hardware PRDT limit.
const PCI_PIIX_IDE_PRD_MAX_ENTRIES: usize = 256;
const _: () = assert!(PCI_PIIX_IDE_BMIDE_IO_SIZE == 16);
const _: () = assert!(PCI_PIIX_IDE_PRD_ENTRY_SIZE == 8);
/// PIIX USB UHCI BAR0 config offset (I/O space).
/// Spec: Intel 82371SB — UHCI I/O BAR at PCI config `0x20` (bit0=1).
pub const PCI_PIIX_USB_BAR0_OFFSET: u8 = 0x20;
/// UHCI BAR0 size decode mask — 32-byte aligned I/O base (bits 15:5).
/// Spec: PCI I/O BAR + UHCI I/O footprint (32 bytes).
pub const PCI_PIIX_USB_BAR0_MASK: u32 = 0xFFE0;

/// UHCI host-controller I/O register block size at BAR0.
/// Spec: Universal Host Controller Interface — 32-byte I/O footprint.
pub const PCI_PIIX_USB_UHCI_IO_SIZE: u16 = 32;
/// UHCI USB Command (USBCMD) offset within BAR0.
pub const PCI_PIIX_USB_UHCI_USBCMD: u8 = 0x00;
/// UHCI USB Status (USBSTS) offset within BAR0.
pub const PCI_PIIX_USB_UHCI_USBSTS: u8 = 0x02;
/// UHCI USB Interrupt Enable (USBINTR) offset within BAR0.
pub const PCI_PIIX_USB_UHCI_USBINTR: u8 = 0x04;
/// UHCI Frame Number (FRNUM) offset within BAR0.
pub const PCI_PIIX_USB_UHCI_FRNUM: u8 = 0x06;
/// UHCI Frame List Base Address (FLBASEADD) offset within BAR0.
pub const PCI_PIIX_USB_UHCI_FLBASEADD: u8 = 0x08;
/// UHCI Start of Frame Modify (SOFMOD) offset within BAR0.
pub const PCI_PIIX_USB_UHCI_SOFMOD: u8 = 0x0C;
/// UHCI Port 1 Status/Control (PORTSC1) offset within BAR0.
pub const PCI_PIIX_USB_UHCI_PORTSC1: u8 = 0x10;
/// UHCI Port 2 Status/Control (PORTSC2) offset within BAR0.
pub const PCI_PIIX_USB_UHCI_PORTSC2: u8 = 0x12;
const _: () = assert!(PCI_PIIX_USB_UHCI_IO_SIZE == 32);
/// PIIX ISA PIRQ route control registers (PIRQRC[A:D]) config offsets `0x60`–`0x63`.
/// Spec: Intel 82371SB — each byte defaults to `0x80` (route disabled).
pub const PCI_PIIX_ISA_PIRQRC_OFFSET: u8 = 0x60;
/// Default PIRQRC byte value (IRQ routing disabled).
pub const PCI_PIIX_ISA_PIRQRC_DEFAULT: u8 = 0x80;
/// PIRQRC bit7: when set, interrupt routing for that PIRQ is disabled.
/// Spec: Intel 82371SB — PIRQRC Route Enable (active-low disable).
pub const PCI_PIIX_ISA_PIRQRC_DISABLE: u8 = 0x80;
/// PIRQRC bits 3:0 — ISA IRQ select field.
/// Spec: Intel 82371SB — IRQ Routing.
pub const PCI_PIIX_ISA_PIRQRC_IRQ_MASK: u8 = 0x0F;
const _: () = assert!(PCI_PIIX_ISA_PIRQRC_DEFAULT == PCI_PIIX_ISA_PIRQRC_DISABLE);
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
/// ACPI PM I/O register block size at PMBASE.
/// Spec: Intel 82371AB — PM I/O footprint is 64 bytes.
pub const PCI_PIIX_ACPI_PM_IO_SIZE: u16 = 64;
/// PM1a Event block offset within PMBASE (`PM1_STS` at +0, `PM1_EN` at +2).
/// Spec: Intel 82371AB / ACPI — `PM1a_EVT_BLK`.
pub const PCI_PIIX_ACPI_PM1A_EVT: u8 = 0x00;
/// PM1a Control register offset within PMBASE.
/// Spec: Intel 82371AB / ACPI — `PM1a_CNT_BLK`.
pub const PCI_PIIX_ACPI_PM1A_CNT: u8 = 0x04;
/// Power Management Timer offset within PMBASE.
/// Spec: Intel 82371AB / ACPI — `PM_TMR_BLK` (24-bit timer; dword access).
pub const PCI_PIIX_ACPI_PM_TMR: u8 = 0x08;
const _: () = assert!(PCI_PIIX_ACPI_PM_IO_SIZE == 64);
const _: () = assert!(PCI_PIIX_ACPI_PM1A_EVT == 0x00);
const _: () = assert!(PCI_PIIX_ACPI_PM1A_CNT == 0x04);
const _: () = assert!(PCI_PIIX_ACPI_PM_TMR == 0x08);
/// PIIX ISA Edge/Level Control Register — master PIC (IRQs 0–7).
/// Spec: Intel 82371 / OSDev 8259 PIC ELCR — I/O port `0x4D0`.
pub const PIIX_ELCR_MASTER: u16 = 0x4D0;
/// PIIX ISA Edge/Level Control Register — slave PIC (IRQs 8–15).
/// Spec: Intel 82371 / OSDev 8259 PIC ELCR — I/O port `0x4D1`.
pub const PIIX_ELCR_SLAVE: u16 = 0x4D1;
/// Writable bits in master ELCR (`0x4D0`): IRQ3–7.
///
/// Spec: Intel 82371 / IFB — IRQ0 (timer), IRQ1 (keyboard), and IRQ2 (cascade)
/// are reserved / hardwired edge-triggered; software cannot select level.
pub const PIIX_ELCR_MASTER_WRITABLE: u8 = 0xF8;
/// Writable bits in slave ELCR (`0x4D1`): all except IRQ8 and IRQ13.
///
/// Spec: Intel 82371 / IFB — IRQ8# (RTC) and IRQ13 (FPU error) are reserved /
/// hardwired edge-triggered; software cannot select level.
pub const PIIX_ELCR_SLAVE_WRITABLE: u8 = 0xDE;
const _: () = assert!(PIIX_ELCR_MASTER_WRITABLE == 0xF8);
const _: () = assert!(PIIX_ELCR_SLAVE_WRITABLE == 0xDE);
const _: () = assert!(PIIX_ELCR_MASTER_WRITABLE & 0x07 == 0);
const _: () = assert!(PIIX_ELCR_SLAVE_WRITABLE & 0x21 == 0);

/// i440FX PMC Programmable Attribute Map register PAM0.
///
/// Spec: Intel 440FX PCIset 82441FX (PMC) datasheet §3.2.18 "PAM — Programmable
/// Attribute Map Registers (PAM[6:0])" — "Address Offset: PAM0 (59h) … PAM6
/// (5Fh)", "Default Value: 00h", "Attribute: Read/Write".
pub const PCI_PMC_PAM0_OFFSET: u8 = 0x59;
/// Number of PAM registers (PAM0–PAM6 at PMC config `0x59`–`0x5F`).
pub const PCI_PMC_PAM_COUNT: usize = 7;
/// PAM power-on / reset value. Spec: 440FX §3.2.18 "Default Value: 00h".
///
/// Table 2 encoding `00` is "Disabled. DRAM is disabled and all accesses are
/// directed to PCI", which is how the platform comes out of reset executing
/// from the BIOS ROM rather than from shadow DRAM.
pub const PCI_PMC_PAM_DEFAULT: u8 = 0x00;

/// Read Enable within one 4-bit PAM attribute field.
///
/// Spec: 440FX §3.2.18 Table 2 "Attribute Bit Assignment" — RE is bits [4, 0].
/// "When RE=1, CPU read accesses to the corresponding memory segment are
/// claimed by the PMC and directed to main memory. Conversely, when RE=0, the
/// CPU read accesses are directed to PCI."
pub const PCI_PMC_PAM_RE: u8 = 1 << 0;
/// Write Enable within one 4-bit PAM attribute field.
///
/// Spec: 440FX §3.2.18 Table 2 — WE is bits [5, 1]. "When WE=1, CPU write
/// accesses to the corresponding memory segment are claimed by the PMC and
/// directed to main memory. Conversely, when WE=0, the CPU write accesses are
/// directed to PCI."
pub const PCI_PMC_PAM_WE: u8 = 1 << 1;
/// Defined bits of one 4-bit PAM attribute field (RE|WE); bits 3:2 Reserved.
pub const PCI_PMC_PAM_FIELD_MASK: u8 = PCI_PMC_PAM_RE | PCI_PMC_PAM_WE;

/// Writable bits of PAM1–PAM6: RE|WE in both nibbles.
///
/// Spec: 440FX §3.2.18 Table 2 — bits [7, 6, 3, 2] are Reserved. PCI Local Bus
/// Specification: reserved configuration fields are read-only and return zero,
/// so this model masks them off on write instead of storing them.
pub const PCI_PMC_PAM_WRITABLE_MASK: u8 = 0x33;
/// Writable bits of PAM0: the high nibble only.
///
/// Spec: 440FX §3.2.18 Table 3 — `PAM0[3:0]` is Reserved; `PAM0[7:4]` carries
/// the `0F0000-0FFFFFh` BIOS Area attributes.
pub const PCI_PMC_PAM0_WRITABLE_MASK: u8 = 0x30;

// Spec: 440FX §3.2.18 Table 2 — a 4-bit attribute field defines only RE and WE,
// so the writable mask of PAM1–PAM6 is that field in both nibbles, and Table 3
// leaves PAM0 with the high nibble alone. Reset leaves every field Disabled.
const _: () = assert!(PCI_PMC_PAM_FIELD_MASK == 0x03);
const _: () =
    assert!(PCI_PMC_PAM_WRITABLE_MASK == PCI_PMC_PAM_FIELD_MASK | (PCI_PMC_PAM_FIELD_MASK << 4));
const _: () = assert!(PCI_PMC_PAM0_WRITABLE_MASK == PCI_PMC_PAM_FIELD_MASK << 4);
const _: () = assert!(PCI_PMC_PAM_DEFAULT & PCI_PMC_PAM_WRITABLE_MASK == 0);

/// Number of legacy memory segments controlled by the PAM registers.
///
/// Spec: 440FX §3.2.18 — "The PMC allows programmable memory attributes on 13
/// memory segments of various sizes in the 640-Kbyte to 1-Mbyte address range."
pub const PCI_PMC_PAM_REGION_COUNT: usize = 13;

/// Table 3 mapping in ascending guest-physical address order:
/// `(pam register index, high nibble?, inclusive start, inclusive end)`.
///
/// Spec: 440FX §3.2.18 Table 3 "PAM Registers and Associated Memory Segments".
/// Entries 0–11 are the twelve 16 KiB ISA Add-on BIOS / BIOS Extension segments
/// from `0C0000h` to `0EFFFFh`; entry 12 is the 64 KiB BIOS Area.
const PAM_REGION_MAP: [(usize, bool, u32, u32); PCI_PMC_PAM_REGION_COUNT] = [
    (1, false, 0x000C_0000, 0x000C_3FFF),
    (1, true, 0x000C_4000, 0x000C_7FFF),
    (2, false, 0x000C_8000, 0x000C_BFFF),
    (2, true, 0x000C_C000, 0x000C_FFFF),
    (3, false, 0x000D_0000, 0x000D_3FFF),
    (3, true, 0x000D_4000, 0x000D_7FFF),
    (4, false, 0x000D_8000, 0x000D_BFFF),
    (4, true, 0x000D_C000, 0x000D_FFFF),
    (5, false, 0x000E_0000, 0x000E_3FFF),
    (5, true, 0x000E_4000, 0x000E_7FFF),
    (6, false, 0x000E_8000, 0x000E_BFFF),
    (6, true, 0x000E_C000, 0x000E_FFFF),
    (0, true, 0x000F_0000, 0x000F_FFFF),
];

/// Decoded PAM attributes for one legacy memory segment.
///
/// Spec: 440FX §3.2.18 Table 2 / Table 3. This is the host-side view the
/// machine layer consumes to steer a physical-address range at ROM or at DRAM;
/// the device crate itself owns no memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PamRegion {
    /// Inclusive guest-physical start of the segment.
    pub start: u32,
    /// Inclusive guest-physical end of the segment.
    pub end: u32,
    /// RE=1 — CPU reads are directed to main memory (DRAM) rather than to PCI.
    pub read_from_ram: bool,
    /// WE=1 — CPU writes are directed to main memory (DRAM) rather than to PCI.
    pub write_to_ram: bool,
}

impl PamRegion {
    /// Segment length in bytes (16 KiB for the expansion/extension segments,
    /// 64 KiB for the BIOS Area).
    pub const fn len(&self) -> u32 {
        self.end - self.start + 1
    }

    /// Always false — every Table 3 segment has a non-zero length. Present so
    /// the `len` accessor does not trip `clippy::len_without_is_empty`.
    pub const fn is_empty(&self) -> bool {
        false
    }

    /// Whether `phys` falls inside this segment.
    pub const fn contains(&self, phys: u32) -> bool {
        phys >= self.start && phys <= self.end
    }
}

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
    /// Reserved IRQ0/1/2/8/13 bits are always 0 (hardwired edge).
    /// `MachineBus` applies these to `DualPic::set_elcr_level_mask` on write.
    pub elcr: [u8; 2],
    /// PIIX IDE Bus Master IDE I/O register file (16 bytes at BMIBA).
    /// Spec: Intel 82371SB — BMICOM/BMISTA/BMIDTP primary + secondary.
    /// Port store/readback plus bounded PRDT [`Self::start_bm_read`] stub. Reset all zeros.
    pub bmide_io: [u8; PCI_PIIX_IDE_BMIDE_IO_SIZE as usize],
    /// PIIX ACPI PM I/O register file (64 bytes at PMBASE).
    /// Spec: Intel 82371AB — `PM1a_EVT` / `PM1a_CNT` / `PM_TMR` (+ remainder).
    /// Store/readback only; no SCI/SMI/power-state machine. Reset all zeros.
    pub acpi_pm_io: [u8; PCI_PIIX_ACPI_PM_IO_SIZE as usize],
    /// PIIX USB UHCI I/O register file (32 bytes at BAR0).
    /// Spec: UHCI 1.1 — USBCMD/USBSTS/USBINTR/FRNUM/FLBASEADD/SOFMOD/PORTSC.
    /// Store/readback only; no schedule/DMA/port engine. Reset all zeros.
    pub uhci_io: [u8; PCI_PIIX_USB_UHCI_IO_SIZE as usize],
    /// Software PIRQA–PIRQD line levels (PCI INTx stub for tests).
    /// Spec: Intel 82371SB — PIRQ# pins; devices assert via `set_pirq_line`.
    pub pirq_asserted: [bool; 4],
    /// ISA IRQ bitmask last driven onto DualPic by [`Self::sync_pirq_to_pic`].
    /// Used to deassert IRQs that lose their last PIRQ route without touching
    /// unrelated PIC lines.
    pub pirq_pic_driven: u16,
}

/// Mask ELCR bytes to PIIX writable bits (IRQ0/1/2/8/13 forced edge / clear).
///
/// Spec: Intel 82371 / IFB — those IRQs cannot be programmed level-sensitive.
#[inline]
pub fn sanitize_piix_elcr(master: u8, slave: u8) -> (u8, u8) {
    (
        master & PIIX_ELCR_MASTER_WRITABLE,
        slave & PIIX_ELCR_SLAVE_WRITABLE,
    )
}

/// Decode a PIRQRC byte to a routed ISA IRQ, or `None` when disabled/invalid.
///
/// Spec: Intel 82371SB — bit7 set disables routing; bits 3:0 select IRQ.
/// Valid routes: 3, 4, 5, 6, 7, 9, 10, 11, 12, 14, 15 (not 0/1/2/8/13).
#[inline]
pub fn pirqrc_routed_irq(byte: u8) -> Option<u8> {
    if byte & PCI_PIIX_ISA_PIRQRC_DISABLE != 0 {
        return None;
    }
    match byte & PCI_PIIX_ISA_PIRQRC_IRQ_MASK {
        irq @ (3 | 4 | 5 | 6 | 7 | 9 | 10 | 11 | 12 | 14 | 15) => Some(irq),
        _ => None,
    }
}

/// One Physical Region Descriptor (8 bytes).
///
/// Spec: Intel Programming Interface for Bus Master IDE Controller Rev 1.0 §1.2;
/// Intel 82371SB — Physical Region Descriptor Format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BmidePrdEntry {
    /// Memory region physical base (bit 0 forced clear — word-aligned).
    pub phys_addr: u32,
    /// Byte count for this region (`0` in the PRD field → [`PCI_PIIX_IDE_PRD_BYTE_COUNT_64K`]).
    pub byte_count: u32,
    /// End of Table (EOT / EOL) — last descriptor in the PRDT.
    pub eot: bool,
}

/// Result of a bounded BMIDE PRD-table transfer stub.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BmidePrdTransfer {
    /// Last descriptor walked. For the existing one-entry case this remains that sole PRD.
    pub entry: BmidePrdEntry,
    /// Number of descriptors walked through the EOT entry.
    pub entries_walked: usize,
    /// Total bytes copied (never greater than the caller's device-buffer length).
    pub bytes_copied: usize,
}

/// Errors from [`PciConfig::start_bm_read`] / [`PciConfig::run_prd_read_stub`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BmidePrdError {
    /// PIIX IDE Command.BusMaster not set.
    BusMasterDisabled,
    /// `device_buf` empty — nothing to transfer.
    EmptyBuffer,
    /// No EOT marker was found before the deterministic descriptor cap.
    MissingEot {
        entries_walked: usize,
        bytes_copied: usize,
    },
    /// Fetching this PRD would wrap the 32-bit guest physical address space.
    PrdTableAddressOverflow { entry_index: usize },
    /// Writing this portion of a PRD would wrap the 32-bit guest physical address space.
    GuestAddressOverflow {
        phys_addr: u32,
        bytes_requested: usize,
    },
}

/// Decode one 8-byte PRD entry.
///
/// Spec: Intel Programming Interface for Bus Master IDE Controller Rev 1.0 §1.2 —
/// dword 0 = Memory Region Physical Base Address \[31:1\]; dword 1 low word =
/// Byte Count \[15:1\] (`0` → 64 KiB); bit 7 of the last byte = EOT.
#[inline]
pub fn decode_bmide_prd(bytes: &[u8; PCI_PIIX_IDE_PRD_ENTRY_SIZE]) -> BmidePrdEntry {
    let phys_addr = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) & !1;
    let count_raw = u16::from_le_bytes([bytes[4], bytes[5]]);
    // Spec: Byte Count [15:1] — bit 0 reserved/0; zero field means 64 KiB.
    let byte_count = if count_raw == 0 {
        PCI_PIIX_IDE_PRD_BYTE_COUNT_64K
    } else {
        u32::from(count_raw & 0xFFFE)
    };
    let eot = bytes[7] & PCI_PIIX_IDE_PRD_EOT != 0;
    BmidePrdEntry {
        phys_addr,
        byte_count,
        eot,
    }
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
            // Spec: Intel 82371SB — BMIDE I/O registers power-on / reset to 0.
            bmide_io: [0; PCI_PIIX_IDE_BMIDE_IO_SIZE as usize],
            // Spec: Intel 82371AB — ACPI PM I/O registers power-on / reset to 0.
            acpi_pm_io: [0; PCI_PIIX_ACPI_PM_IO_SIZE as usize],
            // Spec: UHCI — host-controller I/O registers power-on / reset to 0.
            uhci_io: [0; PCI_PIIX_USB_UHCI_IO_SIZE as usize],
            // Spec: Intel 82371SB — PIRQ# lines idle at reset; routes disabled (0x80).
            pirq_asserted: [false; 4],
            pirq_pic_driven: 0,
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

    /// PIRQRC byte for PIRQ `pirq` (0=A … 3=D) from ISA config `0x60`–`0x63`.
    ///
    /// Spec: Intel 82371SB — PIRQRC[A:D].
    pub fn pirqrc_byte(&self, pirq: u8) -> u8 {
        if pirq >= 4 {
            return PCI_PIIX_ISA_PIRQRC_DEFAULT;
        }
        self.piix_isa[PCI_PIIX_ISA_PIRQRC_OFFSET as usize + usize::from(pirq)]
    }

    /// Assert/deassert software PIRQA–PIRQD (PCI INTx stub). Does not touch PIC
    /// until [`Self::sync_pirq_to_pic`].
    ///
    /// Spec: Intel 82371SB — PIRQ# pin level; callers route via PIRQRC.
    pub fn set_pirq_line(&mut self, pirq: u8, high: bool) {
        if pirq < 4 {
            self.pirq_asserted[usize::from(pirq)] = high;
        }
    }

    /// Current software PIRQ line level (`pirq` 0=A … 3=D).
    pub fn pirq_line(&self, pirq: u8) -> bool {
        pirq < 4 && self.pirq_asserted[usize::from(pirq)]
    }

    /// True when a CONFIG_DATA access at `port`/`size` overlaps PIRQRC `0x60`–`0x63`
    /// on the currently latched Type-1 address (PIIX ISA `00:01.0`).
    pub fn pirqrc_config_write_overlaps(&self, port: u16, size: u8) -> bool {
        if !matches!(port, PCI_CONFIG_DATA..=0xCFF) || !self.enable() {
            return false;
        }
        if self.bus() != 0 || self.device() != 1 || self.function() != 0 {
            return false;
        }
        let start = u16::from(self.reg_offset()) + (port - PCI_CONFIG_DATA);
        let end = start.saturating_add(u16::from(size.max(1)));
        start < 0x64 && end > 0x60
    }

    /// Drive DualPic ISA IRQ lines from latched PIRQ levels through PIRQRC routes.
    ///
    /// Spec: Intel 82371SB — when PIRQRC bit7 is clear, an asserted PIRQ# connects
    /// to the selected ISA IRQ. Multiple PIRQs OR onto the same IRQ. Disabled or
    /// invalid routes do not drive. Only IRQs in the previous or new driven mask
    /// are updated (avoids stomping unrelated device lines).
    pub fn sync_pirq_to_pic(&mut self, pic: &mut crate::DualPic) {
        let mut new_mask = 0u16;
        for pirq in 0..4u8 {
            if !self.pirq_asserted[usize::from(pirq)] {
                continue;
            }
            if let Some(irq) = pirqrc_routed_irq(self.pirqrc_byte(pirq)) {
                new_mask |= 1u16 << irq;
            }
        }
        let changed = self.pirq_pic_driven | new_mask;
        for irq in 0..16u8 {
            let bit = 1u16 << irq;
            if changed & bit != 0 {
                pic.set_irq_line(irq, new_mask & bit != 0);
            }
        }
        self.pirq_pic_driven = new_mask;
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

            // Spec: Intel 440FX §3.2.18 — PAM0–PAM6 at 0x59–0x5F are R/W, but
            // Table 2 bits [7, 6, 3, 2] and Table 3 `PAM0[3:0]` are Reserved.
            Self::apply_pam_reserved_mask(cfg);
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
        // Port decode of the 16-byte BMIDE block is gated by Command.IO.
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
        // Port decode of the PM block is gated by Command.IO (see acpi_pm_io_base).
        if is_piix_acpi && base == PCI_PIIX_ACPI_PMBASE_OFFSET as usize && lane == 0 && size == 4 {
            let masked = (value & PCI_PIIX_ACPI_PMBASE_MASK) | PCI_BAR_IO_SPACE;
            let bytes = masked.to_le_bytes();
            cfg[PCI_PIIX_ACPI_PMBASE_OFFSET as usize..PCI_PIIX_ACPI_PMBASE_OFFSET as usize + 4]
                .copy_from_slice(&bytes);
        }
    }

    /// Raw PAM register byte, `index` 0–6 selecting PAM0 (`0x59`)–PAM6 (`0x5F`).
    ///
    /// Spec: 440FX §3.2.18. Reserved bits always read zero, so the value only
    /// ever carries RE/WE fields.
    pub fn pam_register(&self, index: usize) -> Option<u8> {
        if index >= PCI_PMC_PAM_COUNT {
            return None;
        }
        Some(self.host_bridge[PCI_PMC_PAM0_OFFSET as usize + index])
    }

    /// Store one PAM register byte from the host, `index` 0–6 (PAM0–PAM6).
    ///
    /// Applies the same reserved-bit mask a guest configuration write goes
    /// through, so a host that arms shadowing directly and a guest that
    /// programs `0xCF8`/`0xCFC` leave the register file in the same state.
    /// Returns `false` for an index outside PAM0–PAM6.
    ///
    /// Spec: 440FX §3.2.18 Table 2 / Table 3.
    pub fn set_pam_register(&mut self, index: usize, value: u8) -> bool {
        if index >= PCI_PMC_PAM_COUNT {
            return false;
        }
        let mask = if index == 0 {
            PCI_PMC_PAM0_WRITABLE_MASK
        } else {
            PCI_PMC_PAM_WRITABLE_MASK
        };
        self.host_bridge[PCI_PMC_PAM0_OFFSET as usize + index] = value & mask;
        true
    }

    /// All seven PAM register bytes, PAM0 first.
    pub fn pam_registers(&self) -> [u8; PCI_PMC_PAM_COUNT] {
        let base = PCI_PMC_PAM0_OFFSET as usize;
        let mut out = [0u8; PCI_PMC_PAM_COUNT];
        out.copy_from_slice(&self.host_bridge[base..base + PCI_PMC_PAM_COUNT]);
        out
    }

    /// Decoded attributes for all thirteen PAM segments, in ascending
    /// guest-physical address order (`0C0000h`… first, the BIOS Area last).
    ///
    /// Spec: 440FX §3.2.18 Table 2 / Table 3. This is the accessor the machine
    /// layer drives its per-region ROM/RAM attribute model from; the mapping is
    /// recomputed from the register file on every call, so a caller can refresh
    /// after any configuration write without a change notification.
    pub fn pam_regions(&self) -> [PamRegion; PCI_PMC_PAM_REGION_COUNT] {
        let mut out = [PamRegion {
            start: 0,
            end: 0,
            read_from_ram: false,
            write_to_ram: false,
        }; PCI_PMC_PAM_REGION_COUNT];
        for (slot, &(pam, high, start, end)) in out.iter_mut().zip(PAM_REGION_MAP.iter()) {
            let byte = self.host_bridge[PCI_PMC_PAM0_OFFSET as usize + pam];
            let field = if high { byte >> 4 } else { byte } & 0x0F;
            *slot = PamRegion {
                start,
                end,
                read_from_ram: field & PCI_PMC_PAM_RE != 0,
                write_to_ram: field & PCI_PMC_PAM_WE != 0,
            };
        }
        out
    }

    /// Decoded attributes for the PAM segment containing `phys`, if any.
    ///
    /// Spec: 440FX §3.2.18 — only the thirteen Table 3 segments are attribute
    /// controlled. The video buffer area `A0000-BFFFFh` "is not controlled by
    /// attribute bits" and the DOS area below it is handled by the FDHC
    /// register, so both return `None` here.
    pub fn pam_region_for_addr(&self, phys: u32) -> Option<PamRegion> {
        self.pam_regions().into_iter().find(|r| r.contains(phys))
    }

    /// True when a CONFIG_DATA access at `port`/`size` overlaps PAM0–PAM6
    /// (`0x59`–`0x5F`) on the currently latched Type-1 address, i.e. the PMC
    /// host bridge at `00:00.0`.
    ///
    /// Spec: 440FX §3.2.18 (PAM address offsets) and PCI Local Bus
    /// Specification Mechanism #1 (CONFIG_ADDRESS latches the register offset;
    /// `CFC`–`CFF` select the byte lane). Programming PAM has no memory effect
    /// inside this crate, so a machine that owns physical memory uses this to
    /// know when to re-read [`Self::pam_registers`].
    pub fn pam_config_write_overlaps(&self, port: u16, size: u8) -> bool {
        if !matches!(port, PCI_CONFIG_DATA..=0xCFF) || !self.enable() {
            return false;
        }
        if self.bus() != 0 || self.device() != 0 || self.function() != 0 {
            return false;
        }
        let start = u16::from(self.reg_offset()) + (port - PCI_CONFIG_DATA);
        let end = start.saturating_add(u16::from(size.max(1)));
        let pam_first = u16::from(PCI_PMC_PAM0_OFFSET);
        let pam_end = pam_first + PCI_PMC_PAM_COUNT as u16;
        start < pam_end && end > pam_first
    }

    /// Mask the reserved bits out of the host-bridge PAM block.
    ///
    /// Spec: 440FX §3.2.18 Table 2 (bits [7, 6, 3, 2] Reserved) and Table 3
    /// (`PAM0[3:0]` Reserved), with the PCI Local Bus Specification rule that
    /// reserved configuration fields read as zero.
    fn apply_pam_reserved_mask(cfg: &mut [u8; 256]) {
        let base = PCI_PMC_PAM0_OFFSET as usize;
        for i in 0..PCI_PMC_PAM_COUNT {
            let mask = if i == 0 {
                PCI_PMC_PAM0_WRITABLE_MASK
            } else {
                PCI_PMC_PAM_WRITABLE_MASK
            };
            cfg[base + i] &= mask;
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

    fn piix_ide_command(&self) -> u16 {
        let off = PCI_COMMAND_OFFSET as usize;
        u16::from_le_bytes([self.piix_ide[off], self.piix_ide[off + 1]])
    }

    fn piix_ide_bmiba(&self) -> u32 {
        let off = PCI_PIIX_IDE_BMIBA_OFFSET as usize;
        u32::from_le_bytes([
            self.piix_ide[off],
            self.piix_ide[off + 1],
            self.piix_ide[off + 2],
            self.piix_ide[off + 3],
        ])
    }

    /// Programmed BMIDE I/O base when Command.IO is set and BMIBA has I/O form.
    ///
    /// Spec: PCI Local Bus — I/O Space Enable gates BAR decode; Intel 82371SB —
    /// BMIBA bits 15:4 are the 16-byte-aligned I/O base (bit0 = I/O space).
    pub fn bmide_io_base(&self) -> Option<u16> {
        if self.piix_ide_command() & PCI_COMMAND_IO == 0 {
            return None;
        }
        let bar = self.piix_ide_bmiba();
        if bar & PCI_BAR_IO_SPACE == 0 {
            return None;
        }
        Some((bar & PCI_PIIX_IDE_BMIBA_MASK) as u16)
    }

    /// True when `port` falls in the decoded BMIDE I/O range.
    pub fn bmide_owns_port(&self, port: u16) -> bool {
        let Some(base) = self.bmide_io_base() else {
            return false;
        };
        port.wrapping_sub(base) < PCI_PIIX_IDE_BMIDE_IO_SIZE
    }

    fn bmide_port_read(&self, port: u16, size: u8) -> u32 {
        let Some(base) = self.bmide_io_base() else {
            return 0xFFFFFFFF;
        };
        let off = (port - base) as usize;
        match size {
            1 => u32::from(self.bmide_io.get(off).copied().unwrap_or(0xFF)),
            2 => {
                let b0 = self.bmide_io.get(off).copied().unwrap_or(0xFF);
                let b1 = self.bmide_io.get(off + 1).copied().unwrap_or(0xFF);
                u32::from(u16::from_le_bytes([b0, b1]))
            }
            4 => {
                let mut bytes = [0xFFu8; 4];
                for (i, b) in bytes.iter_mut().enumerate() {
                    if let Some(v) = self.bmide_io.get(off + i) {
                        *b = *v;
                    }
                }
                u32::from_le_bytes(bytes)
            }
            _ => 0xFFFFFFFF,
        }
    }

    fn bmide_port_write(&mut self, port: u16, size: u8, value: u32) {
        let Some(base) = self.bmide_io_base() else {
            return;
        };
        let off = (port - base) as usize;
        match size {
            1 => {
                if let Some(slot) = self.bmide_io.get_mut(off) {
                    *slot = value as u8;
                }
            }
            2 => {
                let bytes = (value as u16).to_le_bytes();
                for (i, b) in bytes.iter().enumerate() {
                    if let Some(slot) = self.bmide_io.get_mut(off + i) {
                        *slot = *b;
                    }
                }
            }
            4 => {
                let bytes = value.to_le_bytes();
                for (i, b) in bytes.iter().enumerate() {
                    if let Some(slot) = self.bmide_io.get_mut(off + i) {
                        *slot = *b;
                    }
                }
            }
            _ => {}
        }
    }

    /// Primary BMIDTP — Descriptor Table Pointer (dword-aligned).
    ///
    /// Spec: Intel 82371SB §2.7.3 — bits \[31:2\] = A\[31:2\]; bits \[1:0\] reserved.
    pub fn bmide_prd_table_ptr_primary(&self) -> u32 {
        let o = PCI_PIIX_IDE_BMIDTP_PRIMARY as usize;
        u32::from_le_bytes([
            self.bmide_io[o],
            self.bmide_io[o + 1],
            self.bmide_io[o + 2],
            self.bmide_io[o + 3],
        ]) & !0b11
    }

    /// Walk the primary-channel PRDT and copy `device_buf` → guest memory (BMIDE Read).
    ///
    /// Spec: Intel Programming Interface for Bus Master IDE Controller Rev 1.0
    /// §1.1–1.2 + Intel 82371SB §2.7 BMIDTP/PRD — fetch consecutive 8-byte
    /// descriptors at BMIDTP until EOT, decode each physical region and size,
    /// and split the caller buffer across those regions. A short final caller
    /// buffer writes only the available bytes; remaining PRDs are still walked
    /// to require EOT. The software safety cap reports [`BmidePrdError::MissingEot`].
    /// Does **not** issue ATA READ DMA or model PCI aborts.
    ///
    /// Requires PIIX IDE Command.BusMaster. Sets BMICOM.SSBM + clears
    /// BMICOM.RWCON (Read), sets BMISTA.Active while walking, then clears
    /// Active + SSBM on completion. A malformed/bounds-failing walk also
    /// latches BMISTA.Error before stopping.
    pub fn start_bm_read<R, W>(
        &mut self,
        device_buf: &[u8],
        mut mem_read: R,
        mut mem_write: W,
    ) -> Result<BmidePrdTransfer, BmidePrdError>
    where
        R: FnMut(u32) -> u8,
        W: FnMut(u32, u8),
    {
        if self.piix_ide_command() & PCI_COMMAND_BUS_MASTER == 0 {
            return Err(BmidePrdError::BusMasterDisabled);
        }
        if device_buf.is_empty() {
            return Err(BmidePrdError::EmptyBuffer);
        }

        // Spec: BMICOM — SSBM=1 starts; RWCON=0 selects Read (IDE→memory).
        self.begin_bm_primary(false);

        let prdt = self.bmide_prd_table_ptr_primary();
        let mut bytes_copied = 0usize;
        for entry_index in 0..PCI_PIIX_IDE_PRD_MAX_ENTRIES {
            let prd_addr = match Self::bm_prd_entry_addr(prdt, entry_index) {
                Some(addr) => addr,
                None => {
                    self.finish_bm_primary(true);
                    return Err(BmidePrdError::PrdTableAddressOverflow { entry_index });
                }
            };

            let mut prd_bytes = [0u8; PCI_PIIX_IDE_PRD_ENTRY_SIZE];
            for (i, b) in prd_bytes.iter_mut().enumerate() {
                *b = mem_read(prd_addr + i as u32);
            }
            let entry = decode_bmide_prd(&prd_bytes);

            let remaining = &device_buf[bytes_copied..];
            let n = (entry.byte_count as usize).min(remaining.len());
            if n != 0 && entry.phys_addr.checked_add((n - 1) as u32).is_none() {
                self.finish_bm_primary(true);
                return Err(BmidePrdError::GuestAddressOverflow {
                    phys_addr: entry.phys_addr,
                    bytes_requested: n,
                });
            }
            for (i, &byte) in remaining.iter().take(n).enumerate() {
                mem_write(entry.phys_addr + i as u32, byte);
            }
            bytes_copied += n;

            if entry.eot {
                self.finish_bm_primary(false);
                return Ok(BmidePrdTransfer {
                    entry,
                    entries_walked: entry_index + 1,
                    bytes_copied,
                });
            }
        }

        self.finish_bm_primary(true);
        Err(BmidePrdError::MissingEot {
            entries_walked: PCI_PIIX_IDE_PRD_MAX_ENTRIES,
            bytes_copied,
        })
    }

    /// Walk the primary-channel PRDT and fill `device_buf` from guest memory
    /// (BMIDE Write).
    ///
    /// This is the write-direction counterpart of [`Self::start_bm_read`] and
    /// keeps the same bounds: descriptors are fetched at BMIDTP until EOT, a
    /// zero byte-count field means 64 KiB, a table without EOT stops at the
    /// deterministic 256-entry cap with [`BmidePrdError::MissingEot`], and a
    /// region that would wrap the 32-bit guest physical address space is
    /// rejected before any byte is copied. A device buffer shorter than the
    /// described regions is filled and the remaining PRDs are still walked to
    /// require EOT.
    ///
    /// Spec: Intel Programming Interface for Bus Master IDE Controller Rev 1.0
    /// §§1.1–1.2 + Intel 82371SB §2.7 — BMICOM RWCON selects the transfer
    /// direction; this call sets SSBM and RWCON (Write), sets BMISTA.Active
    /// while walking, then clears Active + SSBM, latching BMISTA.Error on a
    /// malformed or out-of-bounds table.
    ///
    /// Requires PIIX IDE Command.BusMaster. Does **not** issue ATA WRITE DMA:
    /// there is still no ATA command engine, no secondary-channel engine, and
    /// no PCI abort modeling.
    pub fn start_bm_write<R>(
        &mut self,
        device_buf: &mut [u8],
        mut mem_read: R,
    ) -> Result<BmidePrdTransfer, BmidePrdError>
    where
        R: FnMut(u32) -> u8,
    {
        if self.piix_ide_command() & PCI_COMMAND_BUS_MASTER == 0 {
            return Err(BmidePrdError::BusMasterDisabled);
        }
        if device_buf.is_empty() {
            return Err(BmidePrdError::EmptyBuffer);
        }

        // Spec: BMICOM — SSBM=1 starts; RWCON=1 selects Write (memory→IDE).
        self.begin_bm_primary(true);

        let prdt = self.bmide_prd_table_ptr_primary();
        let mut bytes_copied = 0usize;
        for entry_index in 0..PCI_PIIX_IDE_PRD_MAX_ENTRIES {
            let prd_addr = match Self::bm_prd_entry_addr(prdt, entry_index) {
                Some(addr) => addr,
                None => {
                    self.finish_bm_primary(true);
                    return Err(BmidePrdError::PrdTableAddressOverflow { entry_index });
                }
            };

            let mut prd_bytes = [0u8; PCI_PIIX_IDE_PRD_ENTRY_SIZE];
            for (i, b) in prd_bytes.iter_mut().enumerate() {
                *b = mem_read(prd_addr + i as u32);
            }
            let entry = decode_bmide_prd(&prd_bytes);

            let n = (entry.byte_count as usize).min(device_buf.len() - bytes_copied);
            if n != 0 && entry.phys_addr.checked_add((n - 1) as u32).is_none() {
                self.finish_bm_primary(true);
                return Err(BmidePrdError::GuestAddressOverflow {
                    phys_addr: entry.phys_addr,
                    bytes_requested: n,
                });
            }
            for (i, slot) in device_buf[bytes_copied..bytes_copied + n]
                .iter_mut()
                .enumerate()
            {
                *slot = mem_read(entry.phys_addr + i as u32);
            }
            bytes_copied += n;

            if entry.eot {
                self.finish_bm_primary(false);
                return Ok(BmidePrdTransfer {
                    entry,
                    entries_walked: entry_index + 1,
                    bytes_copied,
                });
            }
        }

        self.finish_bm_primary(true);
        Err(BmidePrdError::MissingEot {
            entries_walked: PCI_PIIX_IDE_PRD_MAX_ENTRIES,
            bytes_copied,
        })
    }

    /// Alias for [`Self::start_bm_write`] — bounded primary-channel PRDT Write stub.
    #[inline]
    pub fn run_prd_write_stub<R>(
        &mut self,
        device_buf: &mut [u8],
        mem_read: R,
    ) -> Result<BmidePrdTransfer, BmidePrdError>
    where
        R: FnMut(u32) -> u8,
    {
        self.start_bm_write(device_buf, mem_read)
    }

    /// Address of PRD `entry_index`, or `None` when the fetch would wrap.
    fn bm_prd_entry_addr(prdt: u32, entry_index: usize) -> Option<u32> {
        let byte_offset = (entry_index * PCI_PIIX_IDE_PRD_ENTRY_SIZE) as u32;
        let prd_addr = prdt.checked_add(byte_offset)?;
        prd_addr.checked_add(PCI_PIIX_IDE_PRD_ENTRY_SIZE as u32 - 1)?;
        Some(prd_addr)
    }

    /// Latch BMICOM SSBM + the requested RWCON direction and BMISTA Active.
    fn begin_bm_primary(&mut self, write_direction: bool) {
        let cmd_off = PCI_PIIX_IDE_BMICOM_PRIMARY as usize;
        let started = self.bmide_io[cmd_off] | PCI_PIIX_IDE_BMICOM_SSBM;
        self.bmide_io[cmd_off] = if write_direction {
            started | PCI_PIIX_IDE_BMICOM_RWCON
        } else {
            started & !PCI_PIIX_IDE_BMICOM_RWCON
        };
        self.bmide_io[PCI_PIIX_IDE_BMISTA_PRIMARY as usize] |= PCI_PIIX_IDE_BMISTA_ACTIVE;
    }

    fn finish_bm_primary(&mut self, error: bool) {
        let st_off = PCI_PIIX_IDE_BMISTA_PRIMARY as usize;
        self.bmide_io[st_off] &= !PCI_PIIX_IDE_BMISTA_ACTIVE;
        if error {
            self.bmide_io[st_off] |= PCI_PIIX_IDE_BMISTA_ERROR;
        }
        let cmd_off = PCI_PIIX_IDE_BMICOM_PRIMARY as usize;
        self.bmide_io[cmd_off] &= !PCI_PIIX_IDE_BMICOM_SSBM;
    }

    /// Alias for [`Self::start_bm_read`] — bounded primary-channel PRDT Read stub.
    #[inline]
    pub fn run_prd_read_stub<R, W>(
        &mut self,
        device_buf: &[u8],
        mem_read: R,
        mem_write: W,
    ) -> Result<BmidePrdTransfer, BmidePrdError>
    where
        R: FnMut(u32) -> u8,
        W: FnMut(u32, u8),
    {
        self.start_bm_read(device_buf, mem_read, mem_write)
    }

    fn piix_acpi_command(&self) -> u16 {
        let off = PCI_COMMAND_OFFSET as usize;
        u16::from_le_bytes([self.piix_acpi[off], self.piix_acpi[off + 1]])
    }

    fn piix_acpi_pmbase(&self) -> u32 {
        let off = PCI_PIIX_ACPI_PMBASE_OFFSET as usize;
        u32::from_le_bytes([
            self.piix_acpi[off],
            self.piix_acpi[off + 1],
            self.piix_acpi[off + 2],
            self.piix_acpi[off + 3],
        ])
    }

    /// Programmed ACPI PM I/O base when Command.IO is set and PMBASE has I/O form.
    ///
    /// Spec: PCI Local Bus — I/O Space Enable gates BAR decode; Intel 82371AB —
    /// PMBASE bits 15:6 are the 64-byte-aligned I/O base (bit0 = I/O space).
    pub fn acpi_pm_io_base(&self) -> Option<u16> {
        if self.piix_acpi_command() & PCI_COMMAND_IO == 0 {
            return None;
        }
        let bar = self.piix_acpi_pmbase();
        if bar & PCI_BAR_IO_SPACE == 0 {
            return None;
        }
        Some((bar & PCI_PIIX_ACPI_PMBASE_MASK) as u16)
    }

    /// True when `port` falls in the decoded ACPI PM I/O range.
    pub fn acpi_pm_owns_port(&self, port: u16) -> bool {
        let Some(base) = self.acpi_pm_io_base() else {
            return false;
        };
        port.wrapping_sub(base) < PCI_PIIX_ACPI_PM_IO_SIZE
    }

    fn acpi_pm_port_read(&self, port: u16, size: u8) -> u32 {
        let Some(base) = self.acpi_pm_io_base() else {
            return 0xFFFFFFFF;
        };
        let off = (port - base) as usize;
        match size {
            1 => u32::from(self.acpi_pm_io.get(off).copied().unwrap_or(0xFF)),
            2 => {
                let b0 = self.acpi_pm_io.get(off).copied().unwrap_or(0xFF);
                let b1 = self.acpi_pm_io.get(off + 1).copied().unwrap_or(0xFF);
                u32::from(u16::from_le_bytes([b0, b1]))
            }
            4 => {
                let mut bytes = [0xFFu8; 4];
                for (i, b) in bytes.iter_mut().enumerate() {
                    if let Some(v) = self.acpi_pm_io.get(off + i) {
                        *b = *v;
                    }
                }
                u32::from_le_bytes(bytes)
            }
            _ => 0xFFFFFFFF,
        }
    }

    fn acpi_pm_port_write(&mut self, port: u16, size: u8, value: u32) {
        let Some(base) = self.acpi_pm_io_base() else {
            return;
        };
        let off = (port - base) as usize;
        match size {
            1 => {
                if let Some(slot) = self.acpi_pm_io.get_mut(off) {
                    *slot = value as u8;
                }
            }
            2 => {
                let bytes = (value as u16).to_le_bytes();
                for (i, b) in bytes.iter().enumerate() {
                    if let Some(slot) = self.acpi_pm_io.get_mut(off + i) {
                        *slot = *b;
                    }
                }
            }
            4 => {
                let bytes = value.to_le_bytes();
                for (i, b) in bytes.iter().enumerate() {
                    if let Some(slot) = self.acpi_pm_io.get_mut(off + i) {
                        *slot = *b;
                    }
                }
            }
            _ => {}
        }
    }

    fn piix_usb_command(&self) -> u16 {
        let off = PCI_COMMAND_OFFSET as usize;
        u16::from_le_bytes([self.piix_usb[off], self.piix_usb[off + 1]])
    }

    fn piix_usb_bar0(&self) -> u32 {
        let off = PCI_PIIX_USB_BAR0_OFFSET as usize;
        u32::from_le_bytes([
            self.piix_usb[off],
            self.piix_usb[off + 1],
            self.piix_usb[off + 2],
            self.piix_usb[off + 3],
        ])
    }

    /// Programmed UHCI I/O base when Command.IO is set and BAR0 has I/O form.
    ///
    /// Spec: PCI Local Bus — I/O Space Enable gates BAR decode; Intel 82371SB /
    /// UHCI — BAR0 bits 15:5 are the 32-byte-aligned I/O base (bit0 = I/O space).
    pub fn uhci_io_base(&self) -> Option<u16> {
        if self.piix_usb_command() & PCI_COMMAND_IO == 0 {
            return None;
        }
        let bar = self.piix_usb_bar0();
        if bar & PCI_BAR_IO_SPACE == 0 {
            return None;
        }
        Some((bar & PCI_PIIX_USB_BAR0_MASK) as u16)
    }

    /// True when `port` falls in the decoded UHCI I/O range.
    pub fn uhci_owns_port(&self, port: u16) -> bool {
        let Some(base) = self.uhci_io_base() else {
            return false;
        };
        port.wrapping_sub(base) < PCI_PIIX_USB_UHCI_IO_SIZE
    }

    fn uhci_port_read(&self, port: u16, size: u8) -> u32 {
        let Some(base) = self.uhci_io_base() else {
            return 0xFFFFFFFF;
        };
        let off = (port - base) as usize;
        match size {
            1 => u32::from(self.uhci_io.get(off).copied().unwrap_or(0xFF)),
            2 => {
                let b0 = self.uhci_io.get(off).copied().unwrap_or(0xFF);
                let b1 = self.uhci_io.get(off + 1).copied().unwrap_or(0xFF);
                u32::from(u16::from_le_bytes([b0, b1]))
            }
            4 => {
                let mut bytes = [0xFFu8; 4];
                for (i, b) in bytes.iter_mut().enumerate() {
                    if let Some(v) = self.uhci_io.get(off + i) {
                        *b = *v;
                    }
                }
                u32::from_le_bytes(bytes)
            }
            _ => 0xFFFFFFFF,
        }
    }

    fn uhci_port_write(&mut self, port: u16, size: u8, value: u32) {
        let Some(base) = self.uhci_io_base() else {
            return;
        };
        let off = (port - base) as usize;
        match size {
            1 => {
                if let Some(slot) = self.uhci_io.get_mut(off) {
                    *slot = value as u8;
                }
            }
            2 => {
                let bytes = (value as u16).to_le_bytes();
                for (i, b) in bytes.iter().enumerate() {
                    if let Some(slot) = self.uhci_io.get_mut(off + i) {
                        *slot = *b;
                    }
                }
            }
            4 => {
                let bytes = value.to_le_bytes();
                for (i, b) in bytes.iter().enumerate() {
                    if let Some(slot) = self.uhci_io.get_mut(off + i) {
                        *slot = *b;
                    }
                }
            }
            _ => {}
        }
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
        // Spec: Intel 82371SB — BMIDE I/O at BMIBA when Command.IO + BAR programmed.
        if self.bmide_owns_port(port) {
            return self.bmide_port_read(port, size);
        }
        // Spec: Intel 82371AB — ACPI PM I/O at PMBASE when Command.IO + BAR programmed.
        if self.acpi_pm_owns_port(port) {
            return self.acpi_pm_port_read(port, size);
        }
        // Spec: Intel 82371SB / UHCI — I/O at BAR0 when Command.IO + BAR programmed.
        if self.uhci_owns_port(port) {
            return self.uhci_port_read(port, size);
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
        // Spec: Intel 82371 / OSDev ELCR — store/readback; reserved IRQ0/1/2/8/13
        // bits hardwired 0 (always edge). MachineBus syncs DualPic.
        if port == PIIX_ELCR_MASTER || port == PIIX_ELCR_SLAVE {
            let idx = (port - PIIX_ELCR_MASTER) as usize;
            match size {
                1 => {
                    let mask = if idx == 0 {
                        PIIX_ELCR_MASTER_WRITABLE
                    } else {
                        PIIX_ELCR_SLAVE_WRITABLE
                    };
                    self.elcr[idx] = (value as u8) & mask;
                }
                2 if port == PIIX_ELCR_MASTER => {
                    let (m, s) = sanitize_piix_elcr(value as u8, (value >> 8) as u8);
                    self.elcr = [m, s];
                }
                _ => {
                    let mask = if idx == 0 {
                        PIIX_ELCR_MASTER_WRITABLE
                    } else {
                        PIIX_ELCR_SLAVE_WRITABLE
                    };
                    self.elcr[idx] = (value as u8) & mask;
                }
            }
            return;
        }
        // Spec: Intel 82371SB — BMIDE noop register file (no DMA).
        if self.bmide_owns_port(port) {
            self.bmide_port_write(port, size, value);
            return;
        }
        // Spec: Intel 82371AB — ACPI PM noop register file (no SCI/SMI).
        if self.acpi_pm_owns_port(port) {
            self.acpi_pm_port_write(port, size, value);
        }
        // Spec: Intel 82371SB / UHCI — noop register file (no schedule/DMA).
        if self.uhci_owns_port(port) {
            self.uhci_port_write(port, size, value);
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
    /// `0x4D0`/`0x4D1` for edge/level; store/readback (DualPic sync on MachineBus).
    /// Reserved IRQ0/1/2/8/13 bits are hardwired 0 (always edge) on write.
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

        // Word access at 0x4D0 covers both ELCR bytes (LE); reserved bits masked.
        // 0x5A → 0x58 (clear IRQ1); 0xA5 → 0x84 (clear IRQ8/IRQ13).
        pci.port_write(PIIX_ELCR_MASTER, 2, 0xA5_5A);
        assert_eq!(
            pci.port_read(PIIX_ELCR_MASTER, 2) as u16,
            u16::from_le_bytes([
                0x5A & PIIX_ELCR_MASTER_WRITABLE,
                0xA5 & PIIX_ELCR_SLAVE_WRITABLE,
            ])
        );
        assert_eq!(
            pci.elcr,
            [
                0x5A & PIIX_ELCR_MASTER_WRITABLE,
                0xA5 & PIIX_ELCR_SLAVE_WRITABLE,
            ]
        );

        pci.reset();
        assert_eq!(pci.elcr, [0x00, 0x00]);
        assert_eq!(pci.port_read(PIIX_ELCR_MASTER, 1) as u8, 0x00);
        assert_eq!(pci.port_read(PIIX_ELCR_SLAVE, 1) as u8, 0x00);
    }

    /// Spec: Intel 82371 / IFB ELCR — IRQ0/1/2/8/13 cannot be programmed for
    /// level-sensitive mode; reserved bits are hardwired 0 (always edge).
    #[test]
    fn piix_elcr_reserved_irqs_hardwired_edge_on_write() {
        let mut pci = PciConfig::new();
        pci.port_write(PIIX_ELCR_MASTER, 1, 0xFF);
        pci.port_write(PIIX_ELCR_SLAVE, 1, 0xFF);
        assert_eq!(
            pci.port_read(PIIX_ELCR_MASTER, 1) as u8,
            PIIX_ELCR_MASTER_WRITABLE
        );
        assert_eq!(
            pci.port_read(PIIX_ELCR_SLAVE, 1) as u8,
            PIIX_ELCR_SLAVE_WRITABLE
        );
        assert_eq!(
            pci.elcr,
            [PIIX_ELCR_MASTER_WRITABLE, PIIX_ELCR_SLAVE_WRITABLE]
        );
        // Explicit reserved-bit checks.
        assert_eq!(pci.elcr[0] & 0x07, 0, "IRQ0/1/2 reserved");
        assert_eq!(pci.elcr[1] & 0x21, 0, "IRQ8/13 reserved");
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

    /// Spec: Intel 82371SB — BMIDE I/O regs reset to 0; no decode until BMIBA+IO.
    #[test]
    fn piix_ide_bmide_reset_default_no_decode() {
        let pci = PciConfig::new();
        assert_eq!(pci.bmide_io, [0; PCI_PIIX_IDE_BMIDE_IO_SIZE as usize]);
        assert_eq!(pci.bmide_io_base(), None);
        assert!(!pci.bmide_owns_port(0xF000));
        assert!(!pci.bmide_owns_port(0x0000));
    }

    /// Spec: Intel 82371SB BMIDE — command/status/PRD store/readback at BMIBA.
    #[test]
    fn piix_ide_bmide_store_readback_when_io_enabled() {
        let mut pci = PciConfig::new();
        // Program BMIBA = 0xF000 (I/O form → 0xF001).
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 1, PCI_PIIX_IDE_BMIBA_OFFSET, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 4, 0x0000_F000);
        // Enable I/O Space.
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 1, PCI_COMMAND_OFFSET, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 2, u32::from(PCI_COMMAND_IO));

        assert_eq!(pci.bmide_io_base(), Some(0xF000));
        assert!(pci.bmide_owns_port(0xF000));
        assert!(pci.bmide_owns_port(0xF00F));
        assert!(!pci.bmide_owns_port(0xF010));

        // Primary command / status / PRD pointer.
        pci.port_write(0xF000 + u16::from(PCI_PIIX_IDE_BMICOM_PRIMARY), 1, 0x09);
        pci.port_write(0xF000 + u16::from(PCI_PIIX_IDE_BMISTA_PRIMARY), 1, 0x60);
        pci.port_write(
            0xF000 + u16::from(PCI_PIIX_IDE_BMIDTP_PRIMARY),
            4,
            0x0011_2233,
        );
        // Secondary channel.
        pci.port_write(0xF000 + u16::from(PCI_PIIX_IDE_BMICOM_SECONDARY), 1, 0x01);
        pci.port_write(0xF000 + u16::from(PCI_PIIX_IDE_BMISTA_SECONDARY), 1, 0x20);
        pci.port_write(
            0xF000 + u16::from(PCI_PIIX_IDE_BMIDTP_SECONDARY),
            4,
            0xAABB_CCDD,
        );

        assert_eq!(
            pci.port_read(0xF000 + u16::from(PCI_PIIX_IDE_BMICOM_PRIMARY), 1) as u8,
            0x09
        );
        assert_eq!(
            pci.port_read(0xF000 + u16::from(PCI_PIIX_IDE_BMISTA_PRIMARY), 1) as u8,
            0x60
        );
        assert_eq!(
            pci.port_read(0xF000 + u16::from(PCI_PIIX_IDE_BMIDTP_PRIMARY), 4),
            0x0011_2233
        );
        assert_eq!(
            pci.port_read(0xF000 + u16::from(PCI_PIIX_IDE_BMICOM_SECONDARY), 1) as u8,
            0x01
        );
        assert_eq!(
            pci.port_read(0xF000 + u16::from(PCI_PIIX_IDE_BMISTA_SECONDARY), 1) as u8,
            0x20
        );
        assert_eq!(
            pci.port_read(0xF000 + u16::from(PCI_PIIX_IDE_BMIDTP_SECONDARY), 4),
            0xAABB_CCDD
        );

        pci.reset();
        assert_eq!(pci.bmide_io, [0; PCI_PIIX_IDE_BMIDE_IO_SIZE as usize]);
        assert_eq!(pci.bmide_io_base(), None);
    }

    /// Program PIIX IDE BMIBA + Command.IO|BusMaster for PRD stub tests.
    fn program_bmide_bar_and_bus_master(pci: &mut PciConfig, bmiba: u16) {
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 1, PCI_PIIX_IDE_BMIBA_OFFSET, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 4, u32::from(bmiba));
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 1, PCI_COMMAND_OFFSET, true),
        );
        pci.port_write(
            PCI_CONFIG_DATA,
            2,
            u32::from(PCI_COMMAND_IO | PCI_COMMAND_BUS_MASTER),
        );
    }

    fn store_test_bmide_prd(
        mem: &mut [u8],
        prdt: u32,
        index: usize,
        phys_addr: u32,
        byte_count: u16,
        eot: bool,
    ) {
        let start = prdt as usize + index * PCI_PIIX_IDE_PRD_ENTRY_SIZE;
        let mut prd = [0u8; PCI_PIIX_IDE_PRD_ENTRY_SIZE];
        prd[0..4].copy_from_slice(&phys_addr.to_le_bytes());
        prd[4..6].copy_from_slice(&byte_count.to_le_bytes());
        if eot {
            prd[7] = PCI_PIIX_IDE_PRD_EOT;
        }
        mem[start..start + PCI_PIIX_IDE_PRD_ENTRY_SIZE].copy_from_slice(&prd);
    }

    /// Spec: Intel Programming Interface for Bus Master IDE §1.2 — PRD decode.
    #[test]
    fn decode_bmide_prd_addr_size_eot_and_zero_means_64k() {
        // addr=0x00100000, count=0x0200 (512), EOT set
        let mut e = [0u8; 8];
        e[0..4].copy_from_slice(&0x0010_0000u32.to_le_bytes());
        e[4..6].copy_from_slice(&0x0200u16.to_le_bytes());
        e[7] = PCI_PIIX_IDE_PRD_EOT;
        let d = decode_bmide_prd(&e);
        assert_eq!(d.phys_addr, 0x0010_0000);
        assert_eq!(d.byte_count, 512);
        assert!(d.eot);

        // Zero count → 64 KiB; EOT clear; odd addr bit forced 0.
        let mut z = [0u8; 8];
        z[0..4].copy_from_slice(&0x0000_1001u32.to_le_bytes());
        let d0 = decode_bmide_prd(&z);
        assert_eq!(d0.phys_addr, 0x0000_1000);
        assert_eq!(d0.byte_count, PCI_PIIX_IDE_PRD_BYTE_COUNT_64K);
        assert!(!d0.eot);
    }

    /// Spec: Intel 82371SB BMIDE — one-PRD Read stub copies via mem callbacks.
    #[test]
    fn start_bm_read_walks_one_prd_and_copies_to_fake_memory() {
        let mut pci = PciConfig::new();
        program_bmide_bar_and_bus_master(&mut pci, 0xF000);

        const PRDT: u32 = 0x0000_2000;
        const BUF: u32 = 0x0000_3000;
        // Primary BMIDTP ← PRDT.
        pci.port_write(0xF000 + u16::from(PCI_PIIX_IDE_BMIDTP_PRIMARY), 4, PRDT);

        // Fake guest RAM: PRD at PRDT + destination buffer at BUF.
        use std::cell::RefCell;
        let mem = RefCell::new(vec![0u8; 0x4000]);
        let mut prd = [0u8; 8];
        prd[0..4].copy_from_slice(&BUF.to_le_bytes());
        prd[4..6].copy_from_slice(&8u16.to_le_bytes());
        prd[7] = PCI_PIIX_IDE_PRD_EOT;
        mem.borrow_mut()[PRDT as usize..PRDT as usize + 8].copy_from_slice(&prd);

        let device = [0xAAu8, 0xBB, 0xCC, 0xDD, 0x11, 0x22, 0x33, 0x44];
        let xfer = pci
            .start_bm_read(
                &device,
                |phys| mem.borrow().get(phys as usize).copied().unwrap_or(0xFF),
                |phys, b| {
                    if let Some(slot) = mem.borrow_mut().get_mut(phys as usize) {
                        *slot = b;
                    }
                },
            )
            .expect("bm read");

        assert_eq!(xfer.entry.phys_addr, BUF);
        assert_eq!(xfer.entry.byte_count, 8);
        assert!(xfer.entry.eot);
        assert_eq!(xfer.entries_walked, 1);
        assert_eq!(xfer.bytes_copied, 8);
        assert_eq!(&mem.borrow()[BUF as usize..BUF as usize + 8], &device);
        // EOT completion clears Active + SSBM.
        assert_eq!(
            pci.bmide_io[PCI_PIIX_IDE_BMISTA_PRIMARY as usize] & PCI_PIIX_IDE_BMISTA_ACTIVE,
            0
        );
        assert_eq!(
            pci.bmide_io[PCI_PIIX_IDE_BMICOM_PRIMARY as usize] & PCI_PIIX_IDE_BMICOM_SSBM,
            0
        );

        // Alias path.
        mem.borrow_mut()[BUF as usize..BUF as usize + 8].fill(0);
        let xfer2 = pci
            .run_prd_read_stub(
                &device,
                |phys| mem.borrow().get(phys as usize).copied().unwrap_or(0xFF),
                |phys, b| {
                    if let Some(slot) = mem.borrow_mut().get_mut(phys as usize) {
                        *slot = b;
                    }
                },
            )
            .expect("alias");
        assert_eq!(xfer2.bytes_copied, 8);
        assert_eq!(&mem.borrow()[BUF as usize..BUF as usize + 8], &device);
    }

    /// Spec: Intel 82371SB §2.7.1 BMICOM RWCON — the write direction moves
    /// guest memory into the device buffer and leaves RWCON latched; reset
    /// clears the whole BMIDE register file.
    #[test]
    fn start_bm_write_fills_device_buffer_and_reset_clears_bmide() {
        let mut pci = PciConfig::new();
        program_bmide_bar_and_bus_master(&mut pci, 0xF000);

        const PRDT: u32 = 0x0000_1000;
        const SRC: u32 = 0x0000_3000;
        pci.port_write(0xF000 + u16::from(PCI_PIIX_IDE_BMIDTP_PRIMARY), 4, PRDT);

        let mut mem = vec![0u8; 0x4000];
        store_test_bmide_prd(&mut mem, PRDT, 0, SRC, 4, true);
        mem[SRC as usize..SRC as usize + 4].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);

        let mut device = [0u8; 4];
        let xfer = pci
            .start_bm_write(&mut device, |phys| mem[phys as usize])
            .expect("bm write");
        assert_eq!(device, [0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(xfer.bytes_copied, 4);
        assert_eq!(
            pci.bmide_io[PCI_PIIX_IDE_BMICOM_PRIMARY as usize] & PCI_PIIX_IDE_BMICOM_RWCON,
            PCI_PIIX_IDE_BMICOM_RWCON
        );
        assert_eq!(
            pci.bmide_io[PCI_PIIX_IDE_BMISTA_PRIMARY as usize] & PCI_PIIX_IDE_BMISTA_ACTIVE,
            0
        );

        pci.reset();
        assert_eq!(pci.bmide_io, [0; PCI_PIIX_IDE_BMIDE_IO_SIZE as usize]);
        assert_eq!(pci.bmide_io_base(), None);
    }

    /// Spec: Bus Master IDE Interface Rev. 1.0 §1.2; Intel 82371SB §2.7.
    #[test]
    fn start_bm_read_walks_two_prds_and_shortens_final_region() {
        let mut pci = PciConfig::new();
        program_bmide_bar_and_bus_master(&mut pci, 0xF000);

        const PRDT: u32 = 0x0000_1000;
        const BUF_A: u32 = 0x0000_3000;
        const BUF_B: u32 = 0x0000_4000;
        pci.port_write(0xF000 + u16::from(PCI_PIIX_IDE_BMIDTP_PRIMARY), 4, PRDT);

        use std::cell::RefCell;
        let mem = RefCell::new(vec![0xEEu8; 0x5000]);
        store_test_bmide_prd(&mut mem.borrow_mut(), PRDT, 0, BUF_A, 4, false);
        store_test_bmide_prd(&mut mem.borrow_mut(), PRDT, 1, BUF_B, 8, true);

        let device = [0x10, 0x11, 0x12, 0x13, 0x20, 0x21];
        let xfer = pci
            .start_bm_read(
                &device,
                |phys| mem.borrow()[phys as usize],
                |phys, byte| mem.borrow_mut()[phys as usize] = byte,
            )
            .expect("two-entry PRDT");

        assert_eq!(xfer.bytes_copied, device.len());
        assert_eq!(xfer.entry.phys_addr, BUF_B);
        assert!(xfer.entry.eot);
        assert_eq!(xfer.entries_walked, 2);
        assert_eq!(
            &mem.borrow()[BUF_A as usize..BUF_A as usize + 4],
            &device[..4]
        );
        assert_eq!(
            &mem.borrow()[BUF_B as usize..BUF_B as usize + 2],
            &device[4..]
        );
        assert_eq!(
            &mem.borrow()[BUF_B as usize + 2..BUF_B as usize + 8],
            &[0xEE; 6]
        );
        assert_eq!(
            pci.bmide_io[PCI_PIIX_IDE_BMISTA_PRIMARY as usize] & PCI_PIIX_IDE_BMISTA_ACTIVE,
            0
        );
        assert_eq!(
            pci.bmide_io[PCI_PIIX_IDE_BMICOM_PRIMARY as usize] & PCI_PIIX_IDE_BMICOM_SSBM,
            0
        );
    }

    /// Spec: Bus Master IDE Interface Rev. 1.0 §1.2 — count zero means 64 KiB.
    #[test]
    fn start_bm_read_zero_count_consumes_64k_before_next_prd() {
        let mut pci = PciConfig::new();
        program_bmide_bar_and_bus_master(&mut pci, 0xF000);

        const PRDT: u32 = 0x0000_1000;
        const BUF_A: u32 = 0x0001_0000;
        const BUF_B: u32 = 0x0002_2000;
        pci.port_write(0xF000 + u16::from(PCI_PIIX_IDE_BMIDTP_PRIMARY), 4, PRDT);

        use std::cell::RefCell;
        let mem = RefCell::new(vec![0u8; 0x0002_3000]);
        store_test_bmide_prd(&mut mem.borrow_mut(), PRDT, 0, BUF_A, 0, false);
        store_test_bmide_prd(&mut mem.borrow_mut(), PRDT, 1, BUF_B, 2, true);
        let device: Vec<u8> = (0..PCI_PIIX_IDE_PRD_BYTE_COUNT_64K as usize + 2)
            .map(|i| i as u8)
            .collect();

        let xfer = pci
            .start_bm_read(
                &device,
                |phys| mem.borrow()[phys as usize],
                |phys, byte| mem.borrow_mut()[phys as usize] = byte,
            )
            .expect("zero-count PRD");

        let split = PCI_PIIX_IDE_PRD_BYTE_COUNT_64K as usize;
        assert_eq!(xfer.bytes_copied, device.len());
        assert_eq!(xfer.entries_walked, 2);
        assert_eq!(
            &mem.borrow()[BUF_A as usize..BUF_A as usize + split],
            &device[..split]
        );
        assert_eq!(
            &mem.borrow()[BUF_B as usize..BUF_B as usize + 2],
            &device[split..]
        );
        assert!(xfer.entry.eot);
    }

    /// Spec: Bus Master IDE Interface Rev. 1.0 §1.2 requires an EOT descriptor.
    #[test]
    fn start_bm_read_missing_eot_stops_at_safety_cap_and_sets_error() {
        const EXPECTED_PRD_CAP: usize = 256;

        let mut pci = PciConfig::new();
        program_bmide_bar_and_bus_master(&mut pci, 0xF000);
        const PRDT: u32 = 0x0000_1000;
        const BUF: u32 = 0x0000_4000;
        pci.port_write(0xF000 + u16::from(PCI_PIIX_IDE_BMIDTP_PRIMARY), 4, PRDT);

        use std::cell::{Cell, RefCell};
        let mem = RefCell::new(vec![0u8; 0x5000]);
        for index in 0..EXPECTED_PRD_CAP {
            store_test_bmide_prd(&mut mem.borrow_mut(), PRDT, index, BUF, 2, false);
        }
        let reads = Cell::new(0usize);
        let writes = Cell::new(0usize);

        let result = pci.start_bm_read(
            &[0xA5],
            |phys| {
                reads.set(reads.get() + 1);
                mem.borrow()[phys as usize]
            },
            |phys, byte| {
                writes.set(writes.get() + 1);
                mem.borrow_mut()[phys as usize] = byte;
            },
        );

        assert_eq!(
            result,
            Err(BmidePrdError::MissingEot {
                entries_walked: EXPECTED_PRD_CAP,
                bytes_copied: 1,
            })
        );
        assert_eq!(reads.get(), EXPECTED_PRD_CAP * PCI_PIIX_IDE_PRD_ENTRY_SIZE);
        assert_eq!(
            writes.get(),
            1,
            "device buffer must not be reread or overrun"
        );
        assert_eq!(
            pci.bmide_io[PCI_PIIX_IDE_BMISTA_PRIMARY as usize] & PCI_PIIX_IDE_BMISTA_ACTIVE,
            0
        );
        assert_ne!(
            pci.bmide_io[PCI_PIIX_IDE_BMISTA_PRIMARY as usize] & PCI_PIIX_IDE_BMISTA_ERROR,
            0,
            "BMISTA Error must latch"
        );
        assert_eq!(
            pci.bmide_io[PCI_PIIX_IDE_BMICOM_PRIMARY as usize] & PCI_PIIX_IDE_BMICOM_SSBM,
            0
        );
    }

    /// Intel 82371SB §2.7: reject DMA ranges that wrap the 32-bit guest address space.
    #[test]
    fn start_bm_read_rejects_wrapping_guest_ranges_without_callbacks() {
        use std::cell::{Cell, RefCell};

        let mut pci = PciConfig::new();
        program_bmide_bar_and_bus_master(&mut pci, 0xF000);
        const PRDT: u32 = 0x0000_1000;
        pci.port_write(0xF000 + u16::from(PCI_PIIX_IDE_BMIDTP_PRIMARY), 4, PRDT);

        let mem = RefCell::new(vec![0u8; 0x2000]);
        store_test_bmide_prd(&mut mem.borrow_mut(), PRDT, 0, 0xFFFF_FFFE, 4, true);
        let writes = RefCell::new(Vec::new());
        let result = pci.start_bm_read(
            &[1, 2, 3, 4],
            |phys| mem.borrow()[phys as usize],
            |phys, byte| writes.borrow_mut().push((phys, byte)),
        );
        assert_eq!(
            result,
            Err(BmidePrdError::GuestAddressOverflow {
                phys_addr: 0xFFFF_FFFE,
                bytes_requested: 4,
            })
        );
        assert!(writes.borrow().is_empty());
        assert_ne!(
            pci.bmide_io[PCI_PIIX_IDE_BMISTA_PRIMARY as usize] & PCI_PIIX_IDE_BMISTA_ERROR,
            0
        );

        let mut pci = PciConfig::new();
        program_bmide_bar_and_bus_master(&mut pci, 0xF000);
        pci.port_write(
            0xF000 + u16::from(PCI_PIIX_IDE_BMIDTP_PRIMARY),
            4,
            0xFFFF_FFFC,
        );
        let reads = Cell::new(0usize);
        let result = pci.start_bm_read(
            &[0x5A],
            |_| {
                reads.set(reads.get() + 1);
                0
            },
            |_, _| {},
        );
        assert_eq!(
            result,
            Err(BmidePrdError::PrdTableAddressOverflow { entry_index: 0 })
        );
        assert_eq!(reads.get(), 0);
    }

    /// Spec: PCI Command Bus Master Enable gates BMIDE DMA.
    #[test]
    fn start_bm_read_requires_bus_master_enable() {
        let mut pci = PciConfig::new();
        // IO only — no BusMaster.
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 1, PCI_PIIX_IDE_BMIBA_OFFSET, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 4, 0x0000_F000);
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 1, PCI_COMMAND_OFFSET, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 2, u32::from(PCI_COMMAND_IO));

        let err = pci
            .start_bm_read(&[0x01], |_| 0, |_, _| {})
            .expect_err("BM disabled");
        assert_eq!(err, BmidePrdError::BusMasterDisabled);
    }

    /// Spec: PCI Command I/O Space Enable — clear → BMIDE BAR not decoded.
    #[test]
    fn piix_ide_bmide_disabled_when_io_command_clear() {
        let mut pci = PciConfig::new();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 1, PCI_PIIX_IDE_BMIBA_OFFSET, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 4, 0x0000_C000);
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 1, PCI_COMMAND_OFFSET, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 2, u32::from(PCI_COMMAND_IO));
        assert_eq!(pci.bmide_io_base(), Some(0xC000));
        pci.port_write(0xC000, 1, 0x55);
        assert_eq!(pci.port_read(0xC000, 1) as u8, 0x55);

        // Clear IO; BusMaster alone must not enable decode.
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 1, PCI_COMMAND_OFFSET, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 2, u32::from(PCI_COMMAND_BUS_MASTER));
        assert_eq!(pci.bmide_io_base(), None);
        assert!(!pci.bmide_owns_port(0xC000));
        // Writes while disabled must not mutate the register file.
        pci.port_write(0xC000, 1, 0xAA);
        // Re-enable IO — prior store while disabled discarded; last good value remains.
        pci.port_write(PCI_CONFIG_DATA, 2, u32::from(PCI_COMMAND_IO));
        assert_eq!(pci.port_read(0xC000, 1) as u8, 0x55);
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

    /// Spec: PCI Local Bus — Cache Line Size at `0x0C`. Host bridge `00:00.0`
    /// stores/reads back the byte (reset `0x00`); no cache/burst side effects.
    /// Mechanism #1: CONFIG_ADDRESS dword-aligned at `0x0C`; byte via `0xCFC`.
    /// Spec: Intel 440FX §3.2.18 Table 3 — the thirteen PAM segments and the
    /// register/nibble that owns each one.
    #[test]
    fn pam_region_map_matches_datasheet_table_3() {
        assert_eq!(PCI_PMC_PAM0_OFFSET, 0x59);
        assert_eq!(PCI_PMC_PAM_COUNT, 7);
        assert_eq!(PCI_PMC_PAM_REGION_COUNT, 13);

        // Twelve 16 KiB segments, then the 64 KiB BIOS Area.
        let expected: [(usize, bool, u32, u32); 13] = [
            (1, false, 0x000C_0000, 0x000C_3FFF),
            (1, true, 0x000C_4000, 0x000C_7FFF),
            (2, false, 0x000C_8000, 0x000C_BFFF),
            (2, true, 0x000C_C000, 0x000C_FFFF),
            (3, false, 0x000D_0000, 0x000D_3FFF),
            (3, true, 0x000D_4000, 0x000D_7FFF),
            (4, false, 0x000D_8000, 0x000D_BFFF),
            (4, true, 0x000D_C000, 0x000D_FFFF),
            (5, false, 0x000E_0000, 0x000E_3FFF),
            (5, true, 0x000E_4000, 0x000E_7FFF),
            (6, false, 0x000E_8000, 0x000E_BFFF),
            (6, true, 0x000E_C000, 0x000E_FFFF),
            (0, true, 0x000F_0000, 0x000F_FFFF),
        ];
        assert_eq!(PAM_REGION_MAP, expected);

        let regions = PciConfig::new().pam_regions();
        for region in regions.iter().take(12) {
            assert_eq!(region.len(), 16 * 1024);
            assert!(!region.is_empty());
        }
        assert_eq!(regions[12].len(), 64 * 1024);
    }

    /// Spec: Intel 440FX §3.2.18 Table 2 — the four encodings of a 4-bit
    /// attribute field: Disabled, Read Only, Write Only, Read/Write.
    #[test]
    fn pam_attribute_field_encodings_decode_to_re_we() {
        assert_eq!(PCI_PMC_PAM_RE, 0x01);
        assert_eq!(PCI_PMC_PAM_WE, 0x02);
        assert_eq!(PCI_PMC_PAM_FIELD_MASK, 0x03);

        let mut pci = PciConfig::new();
        // PAM6 low nibble owns 0E8000-0EBFFFh (region index 10).
        for (field, re, we) in [
            (0x0u8, false, false), // Disabled
            (0x1, true, false),    // Read Only
            (0x2, false, true),    // Write Only
            (0x3, true, true),     // Read/Write
        ] {
            let addr = PciConfig::make_address(0, 0, 0, 0x5F, true);
            pci.port_write(PCI_CONFIG_ADDRESS, 4, addr);
            pci.port_write(PCI_CONFIG_DATA + 3, 1, u32::from(field));

            let region = pci.pam_regions()[10];
            assert_eq!(region.start, 0x000E_8000);
            assert_eq!(region.read_from_ram, re, "field {field:#x} RE");
            assert_eq!(region.write_to_ram, we, "field {field:#x} WE");
        }
    }

    /// Spec: Intel 440FX §3.2.18 — "Default Value: 00h" and "Attribute:
    /// Read/Write"; Table 2 reserves bits [7, 6, 3, 2] and Table 3 reserves
    /// `PAM0[3:0]`, which the PCI Local Bus Specification requires to read zero.
    #[test]
    fn pam_registers_default_store_readback_and_mask_reserved() {
        let mut pci = PciConfig::new();
        assert_eq!(
            pci.pam_registers(),
            [PCI_PMC_PAM_DEFAULT; PCI_PMC_PAM_COUNT]
        );
        assert_eq!(pci.pam_register(PCI_PMC_PAM_COUNT), None);

        for i in 0..PCI_PMC_PAM_COUNT {
            let offset = PCI_PMC_PAM0_OFFSET + i as u8;
            let addr = PciConfig::make_address(0, 0, 0, offset, true);
            pci.port_write(PCI_CONFIG_ADDRESS, 4, addr);
            pci.port_write(PCI_CONFIG_DATA + u16::from(offset & 3), 1, 0xFF);

            let expected = if i == 0 {
                PCI_PMC_PAM0_WRITABLE_MASK
            } else {
                PCI_PMC_PAM_WRITABLE_MASK
            };
            assert_eq!(pci.pam_register(i), Some(expected));
        }

        // `PAM0[3:0]` being reserved means nothing can turn the low nibble on,
        // so no region ever decodes from it.
        assert_eq!(pci.pam_register(0), Some(0x30));

        pci.reset();
        assert_eq!(
            pci.pam_registers(),
            [PCI_PMC_PAM_DEFAULT; PCI_PMC_PAM_COUNT]
        );
    }

    /// The datasheet's worked shadowing example: Write Only while copying the
    /// ROM into DRAM, then Read Only so writes go back out to the expansion bus.
    #[test]
    fn pam_bios_area_shadowing_sequence_decodes() {
        let mut pci = PciConfig::new();
        let addr = PciConfig::make_address(0, 0, 0, PCI_PMC_PAM0_OFFSET, true);

        pci.port_write(PCI_CONFIG_ADDRESS, 4, addr);
        pci.port_write(PCI_CONFIG_DATA + 1, 1, 0x20);
        let bios = pci.pam_region_for_addr(0x000F_FFF0).unwrap();
        assert_eq!((bios.read_from_ram, bios.write_to_ram), (false, true));

        pci.port_write(PCI_CONFIG_ADDRESS, 4, addr);
        pci.port_write(PCI_CONFIG_DATA + 1, 1, 0x10);
        let bios = pci.pam_region_for_addr(0x000F_FFF0).unwrap();
        assert_eq!((bios.read_from_ram, bios.write_to_ram), (true, false));
    }

    /// Spec: Intel 440FX §3.2.18 — the video buffer area `A0000-BFFFFh` "is not
    /// controlled by attribute bits", and the PAM range stops at `0FFFFFh`.
    #[test]
    fn pam_lookup_returns_none_outside_table_3() {
        let pci = PciConfig::new();
        for addr in [0x0000_0000u32, 0x0007_FFFF, 0x000A_0000, 0x000B_FFFF] {
            assert!(pci.pam_region_for_addr(addr).is_none());
        }
        assert!(pci.pam_region_for_addr(0x000C_0000).is_some());
        assert!(pci.pam_region_for_addr(0x000F_FFFF).is_some());
        assert!(pci.pam_region_for_addr(0x0010_0000).is_none());
    }

    #[test]
    fn host_bridge_cache_line_size_store_readback() {
        let mut pci = PciConfig::new();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 0, 0, PCI_CACHE_LINE_SIZE_OFFSET, true),
        );
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 1) as u8,
            PCI_HOST_BRIDGE_CACHE_LINE_SIZE_DEFAULT,
            "Cache Line Size defaults to 0 at reset"
        );

        pci.port_write(PCI_CONFIG_DATA, 1, 0x08);
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 1) as u8, 0x08);

        pci.port_write(PCI_CONFIG_DATA, 1, 0x10);
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 1) as u8, 0x10);

        // Word write at 0xCFC must not clobber Latency Timer when only CLS changes
        // via byte write; dword lane: set CLS=0x20 leaving LT at prior value.
        pci.port_write(0xCFD, 1, 0x40); // Latency Timer
        pci.port_write(PCI_CONFIG_DATA, 1, 0x20); // Cache Line Size only
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 1) as u8, 0x20);
        assert_eq!(pci.port_read(0xCFD, 1) as u8, 0x40);

        pci.reset();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 0, 0, PCI_CACHE_LINE_SIZE_OFFSET, true),
        );
        assert_eq!(
            pci.port_read(PCI_CONFIG_DATA, 1) as u8,
            PCI_HOST_BRIDGE_CACHE_LINE_SIZE_DEFAULT
        );
    }

    /// Spec: PCI Local Bus — Latency Timer at `0x0D`. Host bridge `00:00.0`
    /// stores/reads back the byte (reset `0x00`); no arbitration side effects.
    /// Mechanism #1: CONFIG_ADDRESS is dword-aligned (`0x0C`); byte at `0x0D`
    /// is accessed via CONFIG_DATA lane `0xCFD`.
    #[test]
    fn host_bridge_latency_timer_store_readback() {
        let mut pci = PciConfig::new();
        // Latch dword `0x0C`; Latency Timer is lane +1 (`0xCFD`).
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 0, 0, 0x0C, true),
        );
        assert_eq!(
            pci.port_read(0xCFD, 1) as u8,
            PCI_HOST_BRIDGE_LATENCY_TIMER_DEFAULT,
            "Latency Timer defaults to 0 at reset"
        );

        pci.port_write(0xCFD, 1, 0x40);
        assert_eq!(pci.port_read(0xCFD, 1) as u8, 0x40);

        pci.port_write(0xCFD, 1, 0xFF);
        assert_eq!(pci.port_read(0xCFD, 1) as u8, 0xFF);

        // Word write at 0xCFC: lo=Cache Line Size, hi=Latency Timer.
        pci.port_write(PCI_CONFIG_DATA, 2, 0x20_08); // CLS=0x08, LT=0x20
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 1) as u8, 0x08);
        assert_eq!(pci.port_read(0xCFD, 1) as u8, 0x20);

        pci.reset();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 0, 0, 0x0C, true),
        );
        assert_eq!(
            pci.port_read(0xCFD, 1) as u8,
            PCI_HOST_BRIDGE_LATENCY_TIMER_DEFAULT
        );
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

    /// Spec: Intel 82371AB — ACPI PM I/O regs reset to 0; no decode until PMBASE+IO.
    #[test]
    fn piix_acpi_pm_reset_default_no_decode() {
        let pci = PciConfig::new();
        assert_eq!(pci.acpi_pm_io, [0; PCI_PIIX_ACPI_PM_IO_SIZE as usize]);
        assert_eq!(pci.acpi_pm_io_base(), None);
        assert!(!pci.acpi_pm_owns_port(0xB000));
        assert!(!pci.acpi_pm_owns_port(0x0000));
    }

    /// Spec: Intel 82371AB PM — `PM1a_EVT` / `PM1a_CNT` / `PM_TMR` store/readback.
    #[test]
    fn piix_acpi_pm_store_readback_when_io_enabled() {
        let mut pci = PciConfig::new();
        // Program PMBASE = 0xB000 (I/O form → 0xB001).
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 3, PCI_PIIX_ACPI_PMBASE_OFFSET, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 4, 0x0000_B000);
        // Enable I/O Space.
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 3, PCI_COMMAND_OFFSET, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 2, u32::from(PCI_COMMAND_IO));

        assert_eq!(pci.acpi_pm_io_base(), Some(0xB000));
        assert!(pci.acpi_pm_owns_port(0xB000));
        assert!(pci.acpi_pm_owns_port(0xB03F));
        assert!(!pci.acpi_pm_owns_port(0xB040));

        // PM1a_EVT (STS+EN), PM1a_CNT, PM_TMR.
        pci.port_write(0xB000 + u16::from(PCI_PIIX_ACPI_PM1A_EVT), 2, 0x0101);
        pci.port_write(0xB000 + u16::from(PCI_PIIX_ACPI_PM1A_EVT) + 2, 2, 0x0202);
        pci.port_write(0xB000 + u16::from(PCI_PIIX_ACPI_PM1A_CNT), 2, 0x0001);
        pci.port_write(0xB000 + u16::from(PCI_PIIX_ACPI_PM_TMR), 4, 0x00AB_CDEF);

        assert_eq!(
            pci.port_read(0xB000 + u16::from(PCI_PIIX_ACPI_PM1A_EVT), 2) as u16,
            0x0101
        );
        assert_eq!(
            pci.port_read(0xB000 + u16::from(PCI_PIIX_ACPI_PM1A_EVT) + 2, 2) as u16,
            0x0202
        );
        assert_eq!(
            pci.port_read(0xB000 + u16::from(PCI_PIIX_ACPI_PM1A_CNT), 2) as u16,
            0x0001
        );
        assert_eq!(
            pci.port_read(0xB000 + u16::from(PCI_PIIX_ACPI_PM_TMR), 4),
            0x00AB_CDEF
        );

        pci.reset();
        assert_eq!(pci.acpi_pm_io, [0; PCI_PIIX_ACPI_PM_IO_SIZE as usize]);
        assert_eq!(pci.acpi_pm_io_base(), None);
    }

    /// Spec: PCI Command I/O Space Enable — clear → ACPI PM BAR not decoded.
    #[test]
    fn piix_acpi_pm_disabled_when_io_command_clear() {
        let mut pci = PciConfig::new();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 3, PCI_PIIX_ACPI_PMBASE_OFFSET, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 4, 0x0000_4000);
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 3, PCI_COMMAND_OFFSET, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 2, u32::from(PCI_COMMAND_IO));
        assert_eq!(pci.acpi_pm_io_base(), Some(0x4000));
        pci.port_write(0x4000 + u16::from(PCI_PIIX_ACPI_PM1A_CNT), 2, 0x0005);
        assert_eq!(
            pci.port_read(0x4000 + u16::from(PCI_PIIX_ACPI_PM1A_CNT), 2) as u16,
            0x0005
        );

        // Clear IO; BusMaster alone must not enable decode.
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 3, PCI_COMMAND_OFFSET, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 2, u32::from(PCI_COMMAND_BUS_MASTER));
        assert_eq!(pci.acpi_pm_io_base(), None);
        assert!(!pci.acpi_pm_owns_port(0x4000));
        // Writes while disabled must not mutate the register file.
        pci.port_write(0x4000 + u16::from(PCI_PIIX_ACPI_PM1A_CNT), 2, 0x00AA);
        // Re-enable IO — prior store while disabled discarded; last good value remains.
        pci.port_write(PCI_CONFIG_DATA, 2, u32::from(PCI_COMMAND_IO));
        assert_eq!(
            pci.port_read(0x4000 + u16::from(PCI_PIIX_ACPI_PM1A_CNT), 2) as u16,
            0x0005
        );
    }

    /// Spec: Intel 82371SB — PIRQRC[A:D] at ISA config `0x60`–`0x63` default
    /// `0x80`; store/readback.
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

    /// Spec: Intel 82371SB — PIRQRC bit7 set disables IRQ routing.
    #[test]
    fn pirqrc_routed_irq_disabled_when_bit7_set() {
        assert_eq!(pirqrc_routed_irq(0x80), None);
        assert_eq!(pirqrc_routed_irq(0x85), None);
        assert_eq!(pirqrc_routed_irq(PCI_PIIX_ISA_PIRQRC_DEFAULT), None);
    }

    /// Spec: Intel 82371SB — bit7 clear + bits3:0 select a valid ISA IRQ.
    #[test]
    fn pirqrc_routed_irq_selects_valid_isa_irq() {
        assert_eq!(pirqrc_routed_irq(0x03), Some(3));
        assert_eq!(pirqrc_routed_irq(0x05), Some(5));
        assert_eq!(pirqrc_routed_irq(0x0B), Some(11));
        assert_eq!(pirqrc_routed_irq(0x0E), Some(14));
        // Reserved / invalid selects: no route.
        assert_eq!(pirqrc_routed_irq(0x00), None);
        assert_eq!(pirqrc_routed_irq(0x02), None);
        assert_eq!(pirqrc_routed_irq(0x08), None);
        assert_eq!(pirqrc_routed_irq(0x0D), None);
    }

    /// Spec: Intel 82371SB — asserted PIRQ with disable bit set does not drive PIC.
    #[test]
    fn assert_pirq_disabled_does_not_assert_pic() {
        use crate::DualPic;

        let mut pci = PciConfig::new();
        let mut pic = DualPic::new();
        // Default PIRQRC = 0x80 (disabled).
        pci.set_pirq_line(0, true);
        pci.sync_pirq_to_pic(&mut pic);
        assert_eq!(pci.pirq_pic_driven, 0);
        assert_eq!(pic.poll_irq(), None);
    }

    /// Spec: Intel 82371SB — unmasked PIRQRC routes asserted PIRQ to ISA IRQ.
    #[test]
    fn assert_pirq_routes_to_selected_isa_irq() {
        use crate::{DualPic, PIC_MASTER_CMD, PIC_MASTER_DATA, PIC_SLAVE_CMD, PIC_SLAVE_DATA};

        let mut pci = PciConfig::new();
        let mut pic = DualPic::new();
        // Classic AT cascade + unmask IRQ5 (master IR5).
        pic.port_write(PIC_MASTER_CMD, 1, 0x11);
        pic.port_write(PIC_MASTER_DATA, 1, 0x08);
        pic.port_write(PIC_MASTER_DATA, 1, 0x04);
        pic.port_write(PIC_MASTER_DATA, 1, 0x01);
        pic.port_write(PIC_SLAVE_CMD, 1, 0x11);
        pic.port_write(PIC_SLAVE_DATA, 1, 0x70);
        pic.port_write(PIC_SLAVE_DATA, 1, 0x02);
        pic.port_write(PIC_SLAVE_DATA, 1, 0x01);
        pic.port_write(PIC_MASTER_DATA, 1, 0xDF); // unmask IR5

        // PIRQA → IRQ5 (bit7 clear).
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 0, PCI_PIIX_ISA_PIRQRC_OFFSET, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 1, 0x05);
        assert_eq!(pirqrc_routed_irq(pci.pirqrc_byte(0)), Some(5));

        pci.set_pirq_line(0, true);
        pci.sync_pirq_to_pic(&mut pic);
        assert_eq!(pic.poll_irq(), Some(0x0D)); // vector base 0x08 + IR5
    }

    /// Spec: Intel 82371SB — writing disable while PIRQ held drops the ISA line.
    #[test]
    fn pirqrc_disable_while_asserted_drops_pic_line() {
        use crate::{DualPic, PIC_MASTER_CMD, PIC_MASTER_DATA, PIC_SLAVE_CMD, PIC_SLAVE_DATA};

        let mut pci = PciConfig::new();
        let mut pic = DualPic::new();
        pic.port_write(PIC_MASTER_CMD, 1, 0x11);
        pic.port_write(PIC_MASTER_DATA, 1, 0x08);
        pic.port_write(PIC_MASTER_DATA, 1, 0x04);
        pic.port_write(PIC_MASTER_DATA, 1, 0x01);
        pic.port_write(PIC_SLAVE_CMD, 1, 0x11);
        pic.port_write(PIC_SLAVE_DATA, 1, 0x70);
        pic.port_write(PIC_SLAVE_DATA, 1, 0x02);
        pic.port_write(PIC_SLAVE_DATA, 1, 0x01);
        pic.port_write(PIC_MASTER_DATA, 1, 0xDF);

        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 0, PCI_PIIX_ISA_PIRQRC_OFFSET, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 1, 0x05);
        pci.set_pirq_line(0, true);
        pci.sync_pirq_to_pic(&mut pic);
        assert_eq!(pic.poll_irq(), Some(0x0D));
        pic.port_write(PIC_MASTER_CMD, 1, 0x20); // EOI

        // Disable route while still asserted.
        pci.port_write(PCI_CONFIG_DATA, 1, 0x80);
        pci.sync_pirq_to_pic(&mut pic);
        assert_eq!(pci.pirq_pic_driven, 0);
        assert_eq!(pic.poll_irq(), None);
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

    /// Spec: Intel 82371SB / UHCI — I/O regs reset to 0; no decode until BAR0+IO.
    #[test]
    fn piix_usb_uhci_reset_default_no_decode() {
        let pci = PciConfig::new();
        assert_eq!(pci.uhci_io, [0; PCI_PIIX_USB_UHCI_IO_SIZE as usize]);
        assert_eq!(pci.uhci_io_base(), None);
        assert!(!pci.uhci_owns_port(0xD000));
        assert!(!pci.uhci_owns_port(0x0000));
    }

    /// Spec: Intel UHCI — USBCMD/USBSTS/FRNUM/FLBASEADD/PORTSC store/readback at BAR0.
    #[test]
    fn piix_usb_uhci_store_readback_when_io_enabled() {
        let mut pci = PciConfig::new();
        // Program UHCI BAR0 = 0xD000 (I/O form → 0xD001).
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 2, PCI_PIIX_USB_BAR0_OFFSET, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 4, 0x0000_D000);
        // Enable I/O Space.
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 2, PCI_COMMAND_OFFSET, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 2, u32::from(PCI_COMMAND_IO));

        assert_eq!(pci.uhci_io_base(), Some(0xD000));
        assert!(pci.uhci_owns_port(0xD000));
        assert!(pci.uhci_owns_port(0xD01F));
        assert!(!pci.uhci_owns_port(0xD020));

        // Spec: UHCI I/O — USBCMD/USBSTS/USBINTR/FRNUM/FLBASEADD/SOFMOD/PORTSC.
        pci.port_write(0xD000 + u16::from(PCI_PIIX_USB_UHCI_USBCMD), 2, 0x0001);
        pci.port_write(0xD000 + u16::from(PCI_PIIX_USB_UHCI_USBSTS), 2, 0x0020);
        pci.port_write(0xD000 + u16::from(PCI_PIIX_USB_UHCI_USBINTR), 2, 0x000F);
        pci.port_write(0xD000 + u16::from(PCI_PIIX_USB_UHCI_FRNUM), 2, 0x03FF);
        pci.port_write(
            0xD000 + u16::from(PCI_PIIX_USB_UHCI_FLBASEADD),
            4,
            0x0011_2200,
        );
        pci.port_write(0xD000 + u16::from(PCI_PIIX_USB_UHCI_SOFMOD), 1, 0x40);
        pci.port_write(0xD000 + u16::from(PCI_PIIX_USB_UHCI_PORTSC1), 2, 0x0080);
        pci.port_write(0xD000 + u16::from(PCI_PIIX_USB_UHCI_PORTSC2), 2, 0x0083);

        assert_eq!(
            pci.port_read(0xD000 + u16::from(PCI_PIIX_USB_UHCI_USBCMD), 2) as u16,
            0x0001
        );
        assert_eq!(
            pci.port_read(0xD000 + u16::from(PCI_PIIX_USB_UHCI_USBSTS), 2) as u16,
            0x0020
        );
        assert_eq!(
            pci.port_read(0xD000 + u16::from(PCI_PIIX_USB_UHCI_USBINTR), 2) as u16,
            0x000F
        );
        assert_eq!(
            pci.port_read(0xD000 + u16::from(PCI_PIIX_USB_UHCI_FRNUM), 2) as u16,
            0x03FF
        );
        assert_eq!(
            pci.port_read(0xD000 + u16::from(PCI_PIIX_USB_UHCI_FLBASEADD), 4),
            0x0011_2200
        );
        assert_eq!(
            pci.port_read(0xD000 + u16::from(PCI_PIIX_USB_UHCI_SOFMOD), 1) as u8,
            0x40
        );
        assert_eq!(
            pci.port_read(0xD000 + u16::from(PCI_PIIX_USB_UHCI_PORTSC1), 2) as u16,
            0x0080
        );
        assert_eq!(
            pci.port_read(0xD000 + u16::from(PCI_PIIX_USB_UHCI_PORTSC2), 2) as u16,
            0x0083
        );

        pci.reset();
        assert_eq!(pci.uhci_io, [0; PCI_PIIX_USB_UHCI_IO_SIZE as usize]);
        assert_eq!(pci.uhci_io_base(), None);
    }

    /// Spec: PCI Command I/O Space Enable — clear → UHCI BAR0 not decoded.
    #[test]
    fn piix_usb_uhci_disabled_when_io_command_clear() {
        let mut pci = PciConfig::new();
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 2, PCI_PIIX_USB_BAR0_OFFSET, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 4, 0x0000_D000);
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 2, PCI_COMMAND_OFFSET, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 2, u32::from(PCI_COMMAND_IO));
        assert_eq!(pci.uhci_io_base(), Some(0xD000));
        pci.port_write(0xD000, 1, 0x55);
        assert_eq!(pci.port_read(0xD000, 1) as u8, 0x55);

        // Clear IO; BusMaster alone must not enable decode.
        pci.port_write(
            PCI_CONFIG_ADDRESS,
            4,
            PciConfig::make_address(0, 1, 2, PCI_COMMAND_OFFSET, true),
        );
        pci.port_write(PCI_CONFIG_DATA, 2, u32::from(PCI_COMMAND_BUS_MASTER));
        assert_eq!(pci.uhci_io_base(), None);
        assert!(!pci.uhci_owns_port(0xD000));
        // Writes while disabled must not mutate the register file.
        pci.port_write(0xD000, 1, 0xAA);
        // Re-enable IO — prior store while disabled discarded; last good value remains.
        pci.port_write(PCI_CONFIG_DATA, 2, u32::from(PCI_COMMAND_IO));
        assert_eq!(pci.port_read(0xD000, 1) as u8, 0x55);
    }
}
