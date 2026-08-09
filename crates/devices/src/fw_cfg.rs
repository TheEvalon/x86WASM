//! QEMU fw_cfg I/O subset — selector `0x510` + data `0x511`.
//!
//! Spec: [QEMU Firmware Configuration (fw_cfg) Device](https://www.qemu.org/docs/master/specs/fw_cfg.html)
//! — x86 I/O: selector 16-bit LE at `0x510`, data 8-bit at `0x511`; selector
//! write resets the data offset; reads past end return `0x00`; data-port writes
//! ignored (QEMU ≥2.4 traditional interface).
//!
//! Spec: ACPI Specification §15 "System Address Map Interfaces", Table 15.4
//! "Address Range Descriptor Structure" and §15.2 "Address Range Types" — the
//! encoding of the `etc/e820` firmware file.
//!
//! This device: signature key `0x0000` (`QEMU`), ID key `0x0001`, RAM-size key
//! `0x0003`, and named files in the file directory (`FW_CFG_FILE_DIR` /
//! `FW_CFG_FILE_FIRST`).
//!
//! Firmware files are keyed by name: [`FwCfg::add_file`] rejects a duplicate,
//! [`FwCfg::set_file`] replaces contents while keeping the selector stable, and
//! [`FwCfg::remove_file`] drops both the item and its directory entry.
//! [`FwCfg::set_e820_entries`] publishes a host-supplied system memory map as
//! `etc/e820`, encoded as ACPI address range descriptors (ACPI Specification
//! §15, Table 15.4); an empty map removes the file rather than advertising one
//! with no content.
//!
//! # Selector key numbers and ADR-0005
//!
//! The fw_cfg specification defines only the signature (`0x0000`), the
//! revision/feature bitmap (`0x0001`), and the file directory (`0x0019`),
//! then says of every other key: "Please consult the QEMU source for the most
//! up-to-date and authoritative list of selector keys and their respective
//! items' purpose, format and writeability."
//!
//! `docs/adr/0005-fw-cfg-key-list-interface-reference.md` settles what that
//! means here: QEMU's `fw_cfg.h` and SeaBIOS's headers are approved as an
//! **interface reference only** — key numbers, field widths, blob layouts and
//! firmware file names, which are facts two implementations must agree on to
//! interoperate. No implementation logic was read or copied, and the approval
//! covers fw_cfg interface definitions and nothing else.
//!
//! What this device publishes under that approval:
//!
//! - [`FW_CFG_NB_CPUS`] (`0x0005`), [`FW_CFG_MAX_CPUS`] (`0x000F`) and
//!   [`FW_CFG_FILE_MAX_CPUS`], all 16-bit little-endian and all kept in step by
//!   [`FwCfg::set_cpu_count`]. Default `1`, which is the number of CPUs this
//!   tree actually has.
//! - Host-settable and **absent by default**: [`FW_CFG_UUID`] (`0x0002`),
//!   [`FW_CFG_NOGRAPHIC`] (`0x0004`), [`FW_CFG_FILE_BOOTORDER`] and
//!   [`FW_CFG_FILE_SYSTEM_STATES`]. Each describes a machine fact this device
//!   cannot state on its own, so it stays absent until a host supplies it.
//!
//! # Still not implemented
//!
//! - `etc/table-loader`. It is the ACPI table build script, and this tree
//!   builds no ACPI tables, so there is nothing to load — not even a
//!   host-settable blob, because no honest content exists for it.
//! - Every other numeric key. Absent items read as the specification's "past
//!   the end of the item" answer of `0x00` rather than a fabricated value.
//! - Item writeability (selector bit 14 / DMA control bit 4).
//!
//! DMA interface: the 64-bit big-endian address register lives at `0x514`
//! (high half) / `0x518` (low half, triggering). Writing it latches the guest
//! address of a `FWCfgDmaAccess { be32 control; be32 length; be64 address; }`
//! structure, which the host services through [`FwCfg::service_dma`] with
//! guest-memory callbacks (the device crate never touches host memory). Read
//! (control bit 1), skip (bit 2), and select (bit 3) are implemented; write
//! (bit 4) is rejected with the spec's error bit because item writeability is
//! not modeled.

use std::collections::BTreeMap;

use crate::PortDevice;

/// Selector (control) register — x86 I/O.
pub const FW_CFG_SELECTOR: u16 = 0x510;
/// Data register — x86 I/O (byte access).
pub const FW_CFG_DATA: u16 = 0x511;

/// Signature item — four ASCII bytes `QEMU`.
pub const FW_CFG_SIGNATURE: u16 = 0x0000;
/// Revision / feature bitmap item (32-bit little-endian).
pub const FW_CFG_ID: u16 = 0x0001;
/// Guest RAM size in bytes (64-bit little-endian).
pub const FW_CFG_RAM_SIZE: u16 = 0x0003;
/// File directory item (`FWCfgFiles`).
pub const FW_CFG_FILE_DIR: u16 = 0x0019;
/// First named-file selector key.
pub const FW_CFG_FILE_FIRST: u16 = 0x0020;

/// System UUID item — 16 raw bytes.
///
/// Interface reference (ADR-0005): the key number, the item name and the
/// 16-byte width are facts two implementations must agree on. Absent unless a
/// host supplies a UUID it can justify — see [`FwCfg::set_system_uuid`].
pub const FW_CFG_UUID: u16 = 0x0002;
/// Length of the system UUID item, in bytes.
pub const FW_CFG_UUID_SIZE: usize = 16;
/// "No graphics adapter" flag — 16-bit little-endian.
///
/// Interface reference (ADR-0005). Absent unless the host states it; see
/// [`FwCfg::set_nographic`].
pub const FW_CFG_NOGRAPHIC: u16 = 0x0004;
/// Boot CPU count — 16-bit little-endian.
///
/// Interface reference (ADR-0005). This is the selector firmware reads during
/// POST to size its per-CPU structures.
pub const FW_CFG_NB_CPUS: u16 = 0x0005;
/// Maximum CPU count — 16-bit little-endian.
///
/// Interface reference (ADR-0005). Same width and encoding as
/// [`FW_CFG_NB_CPUS`]; this tree reports the same value in both because it has
/// no hotplug.
pub const FW_CFG_MAX_CPUS: u16 = 0x000F;

/// Firmware file carrying the maximum CPU count, as 16-bit little-endian.
///
/// Interface reference (ADR-0005): the file name and the layout. It duplicates
/// [`FW_CFG_MAX_CPUS`] for firmware that reads the file instead of the numeric
/// selector, and this device keeps the two in step.
pub const FW_CFG_FILE_MAX_CPUS: &str = "etc/max-cpus";
/// Firmware file describing supported ACPI sleep states.
///
/// Interface reference (ADR-0005): six bytes indexed by S-state, each with bit
/// 7 marking the state as supported and bits 6:4 carrying the `SLP_TYP` value
/// to write. **Absent by default** — this tree implements no ACPI power-state
/// machine, so it has nothing truthful to say here. See
/// [`FwCfg::set_system_states`].
pub const FW_CFG_FILE_SYSTEM_STATES: &str = "etc/system-states";
/// Number of bytes in the `etc/system-states` blob (one per S-state, S0–S5).
pub const FW_CFG_SYSTEM_STATES_SIZE: usize = 6;
/// `etc/system-states` per-state bit 7 — this sleep state is supported.
pub const FW_CFG_SYSTEM_STATE_ENABLED: u8 = 0x80;
/// Firmware file carrying the boot order as newline-separated device paths.
///
/// Interface reference (ADR-0005): the file name and the NUL-terminated,
/// newline-separated encoding. **Absent by default** — this machine states no
/// boot policy. See [`FwCfg::set_boot_order`].
pub const FW_CFG_FILE_BOOTORDER: &str = "bootorder";

/// CPU count this device reports when the host states nothing else.
///
/// This tree has one execution context and no SMP anywhere, so one CPU is a
/// fact about the machine rather than a placeholder.
pub const FW_CFG_DEFAULT_CPU_COUNT: u16 = 1;

// Interface reference (ADR-0005): the numeric keys below `FW_CFG_FILE_FIRST`
// are a flat, non-overlapping space, and the two CPU-count views share a width.
const _: () = assert!(FW_CFG_UUID < FW_CFG_RAM_SIZE);
const _: () = assert!(FW_CFG_NOGRAPHIC < FW_CFG_NB_CPUS);
const _: () = assert!(FW_CFG_MAX_CPUS < FW_CFG_FILE_DIR);
const _: () = assert!(FW_CFG_UUID_SIZE == 16);
const _: () = assert!(FW_CFG_SYSTEM_STATES_SIZE == 6);
const _: () = assert!(FW_CFG_SYSTEM_STATE_ENABLED == 1 << 7);
const _: () = assert!(FW_CFG_DEFAULT_CPU_COUNT >= 1);

/// DMA address register — high 32 bits (big-endian), x86 I/O.
pub const FW_CFG_DMA_ADDR_HIGH: u16 = 0x514;
/// DMA address register — low 32 bits (big-endian); a write here triggers.
pub const FW_CFG_DMA_ADDR_LOW: u16 = 0x518;
/// Width of the DMA address register in I/O ports.
pub const FW_CFG_DMA_ADDR_SIZE: u16 = 8;

/// Base fw_cfg interface revision bit, always present.
pub const FW_CFG_VERSION: u32 = 1 << 0;
/// DMA-interface capability bit; set only while the DMA register is live.
pub const FW_CFG_VERSION_DMA: u32 = 1 << 1;

/// Value returned by reads of the DMA address register (`"QEMU CFG"`, big-endian).
pub const FW_CFG_DMA_SIGNATURE: u64 = 0x5145_4D55_2043_4647;

/// `FWCfgDmaAccess.control` bit 0 — error.
pub const FW_CFG_DMA_CTL_ERROR: u32 = 1 << 0;
/// `FWCfgDmaAccess.control` bit 1 — read (item → guest RAM).
pub const FW_CFG_DMA_CTL_READ: u32 = 1 << 1;
/// `FWCfgDmaAccess.control` bit 2 — skip (advance offset only).
pub const FW_CFG_DMA_CTL_SKIP: u32 = 1 << 2;
/// `FWCfgDmaAccess.control` bit 3 — select; upper 16 bits are the selector.
pub const FW_CFG_DMA_CTL_SELECT: u32 = 1 << 3;
/// `FWCfgDmaAccess.control` bit 4 — write (guest RAM → item); unsupported here.
pub const FW_CFG_DMA_CTL_WRITE: u32 = 1 << 4;

/// Size of the `FWCfgDmaAccess` structure in guest RAM.
pub const FW_CFG_DMA_ACCESS_SIZE: u64 = 16;

/// Truthful feature bitmap for a device with or without a live DMA register.
pub const fn cfg_id_value(dma_enabled: bool) -> u32 {
    if dma_enabled {
        FW_CFG_VERSION | FW_CFG_VERSION_DMA
    } else {
        FW_CFG_VERSION
    }
}

/// Why a DMA operation set the spec's error bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FwCfgDmaError {
    /// The `FWCfgDmaAccess` structure would wrap the physical address space.
    AccessStructWrap,
    /// The `address`/`length` buffer would wrap the physical address space.
    BufferWrap,
    /// Control bit 4 (write): item writeability is not modeled in this tree.
    WriteUnsupported,
}

/// Result of one serviced DMA operation (host diagnostics; not guest-visible
/// beyond the control writeback).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FwCfgDmaOutcome {
    /// Guest address of the `FWCfgDmaAccess` structure.
    pub access_addr: u64,
    /// Requested control word (big-endian field, host order).
    pub control: u32,
    /// Requested length in bytes.
    pub length: u32,
    /// Requested guest buffer address.
    pub address: u64,
    /// Bytes actually copied into guest RAM (0 for skip/select-only).
    pub transferred: u32,
    /// Control word written back to the structure (`0` or [`FW_CFG_DMA_CTL_ERROR`]).
    pub result_control: u32,
    /// Failure classification when the error bit was set.
    pub error: Option<FwCfgDmaError>,
}

impl FwCfgDmaOutcome {
    pub fn error(&self) -> bool {
        self.result_control & FW_CFG_DMA_CTL_ERROR != 0
    }
}

/// Selector bit14 — write mode (data-port writes still ignored here).
pub const FW_CFG_SEL_WRITE: u16 = 0x4000;

/// Built-in signature blob.
pub const FW_CFG_SIGNATURE_BYTES: &[u8] = b"QEMU";

/// Default test-file name (NUL-padded to 56 in the directory entry).
pub const FW_CFG_TEST_FILE_NAME: &str = "opt/org.x86wasm/test";
/// Default test-file contents (`FWCF` — fw_cfg family tag for probes).
pub const FW_CFG_TEST_FILE_BYTES: &[u8] = b"FWCF";

/// Longest storable firmware-file name.
///
/// Spec: QEMU fw_cfg "File Directory" — `char name[56]` holding a
/// NUL-terminated ASCII string.
pub const FW_CFG_FILE_NAME_MAX: usize = 55;

/// Firmware file carrying the system memory map.
///
/// The name is the QEMU firmware-interface convention for this blob; the
/// *contents* follow the ACPI address-range descriptor format below. Because
/// the fw_cfg specification does not define what a machine model must place
/// here, this device never synthesizes entries — see
/// [`FwCfg::set_e820_entries`].
pub const FW_CFG_FILE_E820: &str = "etc/e820";

/// Size of one `etc/e820` entry, in bytes.
///
/// Spec: ACPI Specification §15 "System Address Map Interfaces", Table 15.4
/// "Address Range Descriptor Structure" — "The minimum size that must be
/// supported by both the BIOS and the caller is 20 bytes": `BaseAddrLow` (0),
/// `BaseAddrHigh` (4), `LengthLow` (8), `LengthHigh` (12), `Type` (16). The
/// ACPI 3.0 Extended Attributes dword at offset 20 is not emitted.
pub const FW_CFG_E820_ENTRY_SIZE: usize = 20;

/// ACPI address range type 1 — `AddressRangeMemory`, RAM usable by the OS.
///
/// Spec: ACPI Specification §15.2 "Address Range Types".
pub const E820_TYPE_MEMORY: u32 = 1;
/// ACPI address range type 2 — `AddressRangeReserved`.
pub const E820_TYPE_RESERVED: u32 = 2;
/// ACPI address range type 3 — `AddressRangeACPI` (ACPI reclaim memory).
pub const E820_TYPE_ACPI: u32 = 3;
/// ACPI address range type 4 — `AddressRangeNVS` (ACPI NVS memory).
pub const E820_TYPE_NVS: u32 = 4;
/// ACPI address range type 5 — `AddressRangeUnusable`.
pub const E820_TYPE_UNUSABLE: u32 = 5;

// Spec: ACPI §15.2 Address Range Types are consecutive from 1.
const _: () = assert!(E820_TYPE_MEMORY == 1);
const _: () = assert!(E820_TYPE_RESERVED == E820_TYPE_MEMORY + 1);
const _: () = assert!(E820_TYPE_ACPI == E820_TYPE_RESERVED + 1);
const _: () = assert!(E820_TYPE_NVS == E820_TYPE_ACPI + 1);
const _: () = assert!(E820_TYPE_UNUSABLE == E820_TYPE_NVS + 1);

/// One system-address-map range for the `etc/e820` firmware file.
///
/// Spec: ACPI Specification §15, Table 15.4 "Address Range Descriptor
/// Structure" — a 64-bit base address, a 64-bit length in bytes, and a 32-bit
/// range type from §15.2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct E820Entry {
    /// Physical address of the start of the range.
    pub base: u64,
    /// Physical contiguous length of the range, in bytes.
    pub length: u64,
    /// Range type — one of the `E820_TYPE_*` values.
    pub kind: u32,
}

impl E820Entry {
    pub const fn new(base: u64, length: u64, kind: u32) -> Self {
        Self { base, length, kind }
    }

    /// Encode as the 20-byte ACPI address range descriptor.
    ///
    /// The fields are little-endian: the fw_cfg data register is
    /// "string-preserving", so a blob is delivered to the guest byte for byte
    /// and must already be in the guest's native order.
    pub fn to_descriptor(self) -> [u8; FW_CFG_E820_ENTRY_SIZE] {
        let mut out = [0u8; FW_CFG_E820_ENTRY_SIZE];
        out[0..8].copy_from_slice(&self.base.to_le_bytes());
        out[8..16].copy_from_slice(&self.length.to_le_bytes());
        out[16..20].copy_from_slice(&self.kind.to_le_bytes());
        out
    }
}

/// One fw_cfg configuration item (blob).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FwCfgItem {
    pub data: Vec<u8>,
}

impl FwCfgItem {
    pub fn from_bytes(data: impl Into<Vec<u8>>) -> Self {
        Self { data: data.into() }
    }
}

/// Minimal QEMU-compatible fw_cfg device (traditional I/O only).
#[derive(Clone, Debug)]
pub struct FwCfg {
    selector: u16,
    offset: usize,
    items: BTreeMap<u16, FwCfgItem>,
    /// Named files: (selector key ≥ [`FW_CFG_FILE_FIRST`], name).
    files: Vec<(u16, String)>,
    /// DMA address register (0 at startup and after each operation).
    dma_addr: u64,
    /// A write to the low half armed an operation awaiting [`FwCfg::service_dma`].
    dma_pending: bool,
    /// Whether the DMA register decodes and the ID bit advertises it.
    dma_enabled: bool,
}

impl Default for FwCfg {
    fn default() -> Self {
        Self::new()
    }
}

impl FwCfg {
    /// Signature + truthful ID + one test file (`opt/org.x86wasm/test` → `FWCF`).
    ///
    /// Use [`Self::with_ram_size`] when the host machine's RAM size is known.
    pub fn new() -> Self {
        let mut cfg = Self {
            selector: 0,
            offset: 0,
            items: BTreeMap::new(),
            files: Vec::new(),
            dma_addr: 0,
            dma_pending: false,
            dma_enabled: true,
        };
        cfg.set_item(
            FW_CFG_SIGNATURE,
            FwCfgItem::from_bytes(FW_CFG_SIGNATURE_BYTES),
        );
        cfg.refresh_id_item();
        cfg.add_file(FW_CFG_TEST_FILE_NAME, FW_CFG_TEST_FILE_BYTES)
            .expect("default test file fits fw_cfg name limit");
        cfg.set_cpu_count(FW_CFG_DEFAULT_CPU_COUNT);
        cfg
    }

    /// Boot CPU count currently published, as stored in [`FW_CFG_NB_CPUS`].
    pub fn cpu_count(&self) -> u16 {
        self.item(FW_CFG_NB_CPUS)
            .filter(|item| item.data.len() >= 2)
            .map(|item| u16::from_le_bytes([item.data[0], item.data[1]]))
            .unwrap_or(FW_CFG_DEFAULT_CPU_COUNT)
    }

    /// Publish the machine's CPU count through every view firmware may read.
    ///
    /// Interface reference (ADR-0005): [`FW_CFG_NB_CPUS`], [`FW_CFG_MAX_CPUS`]
    /// and [`FW_CFG_FILE_MAX_CPUS`] are all 16-bit little-endian counts. They
    /// are written together so a guest cannot see two different answers.
    ///
    /// A count of zero is clamped to one: a machine with no CPU cannot execute
    /// the firmware asking the question, so zero is never a truthful answer.
    /// This tree has no CPU hotplug, so the maximum equals the boot count.
    pub fn set_cpu_count(&mut self, count: u16) {
        let count = count.max(1);
        let le = count.to_le_bytes();
        self.set_item(FW_CFG_NB_CPUS, FwCfgItem::from_bytes(le));
        self.set_item(FW_CFG_MAX_CPUS, FwCfgItem::from_bytes(le));
        self.set_file(FW_CFG_FILE_MAX_CPUS, le)
            .expect("etc/max-cpus is a valid fw_cfg file name");
    }

    /// Publish a system UUID at [`FW_CFG_UUID`].
    ///
    /// Interface reference (ADR-0005): 16 raw bytes. Nothing in this tree
    /// generates a UUID, so the item is absent until a host states one; an
    /// absent item reads as zeros, which is the null UUID rather than an
    /// invented identity.
    pub fn set_system_uuid(&mut self, uuid: [u8; FW_CFG_UUID_SIZE]) {
        self.set_item(FW_CFG_UUID, FwCfgItem::from_bytes(uuid));
    }

    /// Remove the system UUID item, returning to the absent default.
    pub fn clear_system_uuid(&mut self) {
        self.items.remove(&FW_CFG_UUID);
    }

    /// Publish the nographic flag at [`FW_CFG_NOGRAPHIC`] (16-bit LE, 1 = no
    /// graphics adapter).
    ///
    /// Whether the machine has a display is the machine's fact, not this
    /// device's, so the item is absent until a host states it.
    pub fn set_nographic(&mut self, nographic: bool) {
        self.set_item(
            FW_CFG_NOGRAPHIC,
            FwCfgItem::from_bytes(u16::from(nographic).to_le_bytes()),
        );
    }

    /// Publish [`FW_CFG_FILE_BOOTORDER`] from a list of firmware device paths.
    ///
    /// Interface reference (ADR-0005): the blob is the paths separated by
    /// newlines, with a trailing newline, NUL-terminated. An empty list
    /// *removes* the file: a machine with no boot policy must be silent rather
    /// than publish an empty one, which firmware would read as "boot nothing".
    pub fn set_boot_order(&mut self, entries: &[&str]) -> Option<u16> {
        if entries.is_empty() {
            self.remove_file(FW_CFG_FILE_BOOTORDER);
            return None;
        }
        let mut blob = Vec::new();
        for entry in entries {
            blob.extend_from_slice(entry.as_bytes());
            blob.push(b'\n');
        }
        blob.push(0);
        self.set_file(FW_CFG_FILE_BOOTORDER, blob)
            .expect("bootorder is a valid fw_cfg file name")
            .into()
    }

    /// Publish [`FW_CFG_FILE_SYSTEM_STATES`] from a host-supplied blob.
    ///
    /// Interface reference (ADR-0005): six bytes indexed by S-state, each with
    /// [`FW_CFG_SYSTEM_STATE_ENABLED`] marking the state as supported and bits
    /// 6:4 carrying the `SLP_TYP` value.
    ///
    /// This device never publishes the file on its own. Nothing in this tree
    /// implements an ACPI power-state machine — the PIIX PM I/O block is a noop
    /// store/readback — so it has no state it could honestly claim.
    pub fn set_system_states(&mut self, states: [u8; FW_CFG_SYSTEM_STATES_SIZE]) -> u16 {
        self.set_file(FW_CFG_FILE_SYSTEM_STATES, states)
            .expect("etc/system-states is a valid fw_cfg file name")
    }

    /// Construct the traditional interface with the host-configured RAM size.
    pub fn with_ram_size(ram_size: u64) -> Self {
        let mut cfg = Self::new();
        cfg.set_ram_size(ram_size);
        cfg
    }

    /// Replace the RAM-size host configuration entry.
    pub fn set_ram_size(&mut self, ram_size: u64) {
        self.set_item(
            FW_CFG_RAM_SIZE,
            FwCfgItem::from_bytes(ram_size.to_le_bytes()),
        );
    }

    /// Reset guest-visible stream and DMA register state while preserving host
    /// configuration (items, files, RAM size, DMA availability).
    pub fn reset(&mut self) {
        self.selector = 0;
        self.offset = 0;
        self.dma_addr = 0;
        self.dma_pending = false;
    }

    pub fn owns_port(port: u16) -> bool {
        port == FW_CFG_SELECTOR
            || port == FW_CFG_DATA
            || (FW_CFG_DMA_ADDR_HIGH..FW_CFG_DMA_ADDR_HIGH + FW_CFG_DMA_ADDR_SIZE).contains(&port)
    }

    /// Whether the DMA address register decodes and ID bit 1 is advertised.
    pub fn dma_enabled(&self) -> bool {
        self.dma_enabled
    }

    /// Enable/disable the DMA interface, keeping ID bit 1 truthful.
    ///
    /// A host that cannot service [`Self::service_dma`] must disable it, which
    /// clears the feature bit and leaves `0x514`–`0x51B` as open bus.
    pub fn set_dma_enabled(&mut self, enabled: bool) {
        self.dma_enabled = enabled;
        self.dma_addr = 0;
        self.dma_pending = false;
        self.refresh_id_item();
    }

    /// Current DMA address register value (0 at startup and after an operation).
    pub fn dma_address(&self) -> u64 {
        self.dma_addr
    }

    /// Whether a triggered DMA operation is awaiting [`Self::service_dma`].
    pub fn dma_pending(&self) -> bool {
        self.dma_pending
    }

    fn refresh_id_item(&mut self) {
        self.set_item(
            FW_CFG_ID,
            FwCfgItem::from_bytes(cfg_id_value(self.dma_enabled).to_le_bytes()),
        );
    }

    pub fn selector(&self) -> u16 {
        self.selector
    }

    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Canonical item key: clear write-mode bit14 (arch bit15 kept).
    pub fn item_key(selector: u16) -> u16 {
        selector & !FW_CFG_SEL_WRITE
    }

    pub fn set_item(&mut self, key: u16, item: FwCfgItem) {
        self.items.insert(Self::item_key(key), item);
        if Self::item_key(key) == FW_CFG_FILE_DIR {
            return;
        }
        self.rebuild_file_dir();
    }

    /// Insert a new named file at the next free selector ≥ [`FW_CFG_FILE_FIRST`].
    ///
    /// Spec: QEMU fw_cfg "File Directory" — a firmware file is identified by
    /// its `char name[56]` NUL-terminated name, so a duplicate name would make
    /// the directory ambiguous and is rejected here. Use [`Self::set_file`] to
    /// replace the contents of an existing file.
    pub fn add_file(&mut self, name: &str, data: impl Into<Vec<u8>>) -> Result<u16, &'static str> {
        Self::check_file_name(name)?;
        if self.file_selector(name).is_some() {
            return Err("fw_cfg file name already present");
        }
        let key = self
            .files
            .iter()
            .map(|(k, _)| *k)
            .max()
            .map(|k| k.saturating_add(1))
            .unwrap_or(FW_CFG_FILE_FIRST);
        if key < FW_CFG_FILE_FIRST {
            return Err("fw_cfg file selector exhausted");
        }
        self.items.insert(key, FwCfgItem::from_bytes(data.into()));
        self.files.push((key, name.to_string()));
        self.rebuild_file_dir();
        Ok(key)
    }

    /// Insert or replace a named file, keeping the selector stable across a
    /// replacement so a guest that already walked the directory stays valid.
    pub fn set_file(&mut self, name: &str, data: impl Into<Vec<u8>>) -> Result<u16, &'static str> {
        Self::check_file_name(name)?;
        match self.file_selector(name) {
            Some(key) => {
                self.items.insert(key, FwCfgItem::from_bytes(data.into()));
                self.rebuild_file_dir();
                Ok(key)
            }
            None => self.add_file(name, data),
        }
    }

    /// Remove a named file and its directory entry. Returns the freed selector.
    ///
    /// The selector is not reused, so a stale guest reference reads an unknown
    /// item (all `0x00`) rather than someone else's blob.
    pub fn remove_file(&mut self, name: &str) -> Option<u16> {
        let key = self.file_selector(name)?;
        self.files.retain(|(k, _)| *k != key);
        self.items.remove(&key);
        self.rebuild_file_dir();
        Some(key)
    }

    /// Selector currently assigned to a named firmware file, if present.
    pub fn file_selector(&self, name: &str) -> Option<u16> {
        self.files
            .iter()
            .find(|(_, n)| n == name)
            .map(|(key, _)| *key)
    }

    /// Names of every firmware file in the directory, in selector order.
    pub fn file_names(&self) -> Vec<&str> {
        self.files.iter().map(|(_, n)| n.as_str()).collect()
    }

    fn check_file_name(name: &str) -> Result<(), &'static str> {
        if name.len() > FW_CFG_FILE_NAME_MAX || name.contains('\0') {
            return Err("fw_cfg file name must be ≤55 ASCII chars without NUL");
        }
        Ok(())
    }

    /// Build an [`E820Entry`] without having to name the type.
    ///
    /// Convenience for callers outside this crate that only need to hand a map
    /// to [`Self::set_e820_entries`].
    pub const fn e820_entry(base: u64, length: u64, kind: u32) -> E820Entry {
        E820Entry::new(base, length, kind)
    }

    /// Publish the system memory map as the `etc/e820` firmware file.
    ///
    /// Spec: ACPI Specification §15, Table 15.4 — each entry is encoded as the
    /// 20-byte address range descriptor (little-endian 64-bit base, 64-bit
    /// length, 32-bit type from §15.2). Returns the file's selector, or `None`
    /// when `entries` is empty, in which case the file is *removed*: the fw_cfg
    /// specification does not say what a machine model must place in this blob,
    /// so an emulator that cannot describe its address space must be silent
    /// rather than publish an empty map that firmware would read as "no RAM".
    ///
    /// This device never synthesizes entries from the RAM-size item; the host
    /// supplies the map it can actually justify.
    pub fn set_e820_entries(&mut self, entries: &[E820Entry]) -> Option<u16> {
        if entries.is_empty() {
            self.remove_file(FW_CFG_FILE_E820);
            return None;
        }
        let mut blob = Vec::with_capacity(entries.len() * FW_CFG_E820_ENTRY_SIZE);
        for entry in entries {
            blob.extend_from_slice(&entry.to_descriptor());
        }
        self.set_file(FW_CFG_FILE_E820, blob)
            .expect("etc/e820 is a valid fw_cfg file name")
            .into()
    }

    pub fn item(&self, key: u16) -> Option<&FwCfgItem> {
        self.items.get(&Self::item_key(key))
    }

    fn rebuild_file_dir(&mut self) {
        // Spec: QEMU fw_cfg — FWCfgFiles { be32 count; FWCfgFile[count] }.
        let count = self.files.len() as u32;
        let mut dir = Vec::with_capacity(4 + self.files.len() * 64);
        dir.extend_from_slice(&count.to_be_bytes());
        for (select, name) in &self.files {
            let size = self
                .items
                .get(select)
                .map(|i| i.data.len() as u32)
                .unwrap_or(0);
            dir.extend_from_slice(&size.to_be_bytes());
            dir.extend_from_slice(&select.to_be_bytes());
            dir.extend_from_slice(&0u16.to_be_bytes());
            let mut name_buf = [0u8; 56];
            let n = name.len().min(55);
            name_buf[..n].copy_from_slice(&name.as_bytes()[..n]);
            dir.extend_from_slice(&name_buf);
        }
        self.items
            .insert(FW_CFG_FILE_DIR, FwCfgItem::from_bytes(dir));
    }

    fn select(&mut self, selector: u16) {
        self.selector = selector;
        self.offset = 0;
    }

    /// Next byte of the selected item, advancing the offset (0 past the end).
    fn next_item_byte(&mut self) -> u8 {
        let key = Self::item_key(self.selector);
        let b = self
            .items
            .get(&key)
            .and_then(|item| item.data.get(self.offset))
            .copied()
            .unwrap_or(0);
        self.offset = self.offset.saturating_add(1);
        b
    }

    /// Service a triggered DMA operation against guest memory.
    ///
    /// Spec: QEMU fw_cfg "Guest-side DMA Interface" — reads the big-endian
    /// `FWCfgDmaAccess` structure at the latched address, applies select /
    /// read / skip, and writes the result control word back to the structure
    /// (all bits clear on success, bit 0 set on error).
    ///
    /// Returns `None` when no operation is pending. `mem_read` / `mem_write`
    /// are guest-physical byte accessors supplied by the host.
    pub fn service_dma<R, W>(
        &mut self,
        mut mem_read: R,
        mut mem_write: W,
    ) -> Option<FwCfgDmaOutcome>
    where
        R: FnMut(u64) -> u8,
        W: FnMut(u64, u8),
    {
        if !self.dma_pending {
            return None;
        }
        self.dma_pending = false;
        let access_addr = self.dma_addr;
        // Spec: the register value is 0 at startup and after an operation.
        self.dma_addr = 0;

        let Some(access_end) = access_addr.checked_add(FW_CFG_DMA_ACCESS_SIZE - 1) else {
            // The structure itself is unreachable, so there is nowhere to write
            // the error bit back to.
            return Some(FwCfgDmaOutcome {
                access_addr,
                control: 0,
                length: 0,
                address: 0,
                transferred: 0,
                result_control: FW_CFG_DMA_CTL_ERROR,
                error: Some(FwCfgDmaError::AccessStructWrap),
            });
        };
        let _ = access_end;

        let mut raw = [0u8; FW_CFG_DMA_ACCESS_SIZE as usize];
        for (i, slot) in raw.iter_mut().enumerate() {
            *slot = mem_read(access_addr + i as u64);
        }
        let control = u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]);
        let length = u32::from_be_bytes([raw[4], raw[5], raw[6], raw[7]]);
        let address = u64::from_be_bytes([
            raw[8], raw[9], raw[10], raw[11], raw[12], raw[13], raw[14], raw[15],
        ]);

        if control & FW_CFG_DMA_CTL_SELECT != 0 {
            self.select((control >> 16) as u16);
        }

        let mut transferred = 0u32;
        let mut error = None;
        if control & FW_CFG_DMA_CTL_READ != 0 {
            if length > 0 && address.checked_add(u64::from(length) - 1).is_none() {
                error = Some(FwCfgDmaError::BufferWrap);
            } else {
                for i in 0..length {
                    let b = self.next_item_byte();
                    mem_write(address + u64::from(i), b);
                }
                transferred = length;
            }
        } else if control & FW_CFG_DMA_CTL_WRITE != 0 {
            // Item writeability is not modeled; reject instead of corrupting.
            error = Some(FwCfgDmaError::WriteUnsupported);
        } else if control & FW_CFG_DMA_CTL_SKIP != 0 {
            self.offset = self.offset.saturating_add(length as usize);
        }

        let result_control = if error.is_some() {
            FW_CFG_DMA_CTL_ERROR
        } else {
            0
        };
        for (i, b) in result_control.to_be_bytes().into_iter().enumerate() {
            mem_write(access_addr + i as u64, b);
        }

        Some(FwCfgDmaOutcome {
            access_addr,
            control,
            length,
            address,
            transferred,
            result_control,
            error,
        })
    }

    /// Big-endian byte of the DMA address register window (signature on read).
    fn dma_signature_byte(offset: u16) -> u8 {
        FW_CFG_DMA_SIGNATURE.to_be_bytes()[offset as usize]
    }

    fn read_bytes(&mut self, n: usize) -> u32 {
        let key = Self::item_key(self.selector);
        let mut out = 0u32;
        for i in 0..n {
            let b = self
                .items
                .get(&key)
                .and_then(|item| item.data.get(self.offset))
                .copied()
                .unwrap_or(0);
            self.offset = self.offset.saturating_add(1);
            out |= u32::from(b) << (8 * i);
        }
        out
    }
}

impl PortDevice for FwCfg {
    fn port_read(&mut self, port: u16, size: u8) -> u32 {
        if !Self::owns_port(port) {
            return 0xFFFFFFFF;
        }
        match port {
            // Spec: selector is write-only; read returns open-bus style 0xFF… .
            FW_CFG_SELECTOR => 0xFFFFFFFF,
            FW_CFG_DATA => {
                let n = match size {
                    2 => 2,
                    4 => 4,
                    _ => 1,
                };
                self.read_bytes(n)
            }
            // Spec: reading the DMA address register returns the big-endian
            // `QEMU CFG` signature when the DMA interface is available.
            _ if self.dma_enabled => {
                let off = port - FW_CFG_DMA_ADDR_HIGH;
                let n = u16::from(match size {
                    2 => 2u8,
                    4 => 4,
                    _ => 1,
                });
                if off + n > FW_CFG_DMA_ADDR_SIZE {
                    return 0xFFFFFFFF;
                }
                let mut out = 0u32;
                for i in 0..n {
                    out |= u32::from(Self::dma_signature_byte(off + i)) << (8 * i);
                }
                out
            }
            _ => 0xFFFFFFFF,
        }
    }

    fn port_write(&mut self, port: u16, size: u8, value: u32) {
        if !Self::owns_port(port) {
            return;
        }
        match port {
            FW_CFG_SELECTOR => {
                // Spec: 16-bit little-endian selector; byte write updates low byte.
                let sel = match size {
                    2 | 4 => value as u16,
                    _ => (self.selector & 0xFF00) | (value as u8 as u16),
                };
                self.select(sel);
            }
            // Spec: QEMU ≥2.4 — traditional data-port writes are no-ops.
            FW_CFG_DATA => {}
            // Spec: 64-bit big-endian DMA address register; a write to the
            // least significant half (offset 4) triggers the operation.
            // Only the two 32-bit halves are accepted here.
            FW_CFG_DMA_ADDR_HIGH if self.dma_enabled && size == 4 => {
                let high = u64::from(value.swap_bytes());
                self.dma_addr = (high << 32) | (self.dma_addr & 0xFFFF_FFFF);
            }
            FW_CFG_DMA_ADDR_LOW if self.dma_enabled && size == 4 => {
                let low = u64::from(value.swap_bytes());
                self.dma_addr = (self.dma_addr & !0xFFFF_FFFF) | low;
                self.dma_pending = true;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_n(cfg: &mut FwCfg, n: usize) -> Vec<u8> {
        (0..n)
            .map(|_| cfg.port_read(FW_CFG_DATA, 1) as u8)
            .collect()
    }

    #[test]
    fn signature_qemu_via_selector_data() {
        let mut cfg = FwCfg::new();
        cfg.port_write(FW_CFG_SELECTOR, 2, u32::from(FW_CFG_SIGNATURE));
        assert_eq!(read_n(&mut cfg, 4), b"QEMU");
        // Past end → 0x00.
        assert_eq!(cfg.port_read(FW_CFG_DATA, 1) as u8, 0);
    }

    /// QEMU fw_cfg spec: selector 0x0001 is a LE32 feature bitmap. Bit 0 is
    /// the base revision; bit 1 advertises the optional DMA interface.
    #[test]
    fn id_selector_is_a_le32_feature_bitmap() {
        assert_eq!(FW_CFG_ID, 0x0001);
        assert_eq!(FW_CFG_VERSION, 0x0000_0001);
        assert_eq!(FW_CFG_VERSION_DMA, 0x0000_0002);

        let mut cfg = FwCfg::new();
        cfg.port_write(FW_CFG_SELECTOR, 2, u32::from(FW_CFG_ID));
        assert_eq!(read_n(&mut cfg, 4), cfg_id_value(true).to_le_bytes());
    }

    /// QEMU fw_cfg spec: selector 0x0003 is the RAM byte count as LE64.
    #[test]
    fn configured_ram_size_uses_le64_and_survives_reset() {
        assert_eq!(FW_CFG_RAM_SIZE, 0x0003);

        let ram_size = 16u64 * 1024 * 1024;
        let mut cfg = FwCfg::with_ram_size(ram_size);

        cfg.port_write(FW_CFG_SELECTOR, 2, u32::from(FW_CFG_RAM_SIZE));
        assert_eq!(
            read_n(&mut cfg, 8),
            [0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00]
        );
        cfg.port_write(FW_CFG_SELECTOR, 2, u32::from(FW_CFG_RAM_SIZE));
        let _ = cfg.port_read(FW_CFG_DATA, 1);

        cfg.reset();

        assert_eq!(cfg.selector(), 0);
        assert_eq!(cfg.offset(), 0);
        cfg.port_write(FW_CFG_SELECTOR, 2, u32::from(FW_CFG_RAM_SIZE));
        assert_eq!(read_n(&mut cfg, 8), ram_size.to_le_bytes());
    }

    #[test]
    fn selector_write_resets_offset() {
        let mut cfg = FwCfg::new();
        cfg.port_write(FW_CFG_SELECTOR, 2, u32::from(FW_CFG_SIGNATURE));
        assert_eq!(cfg.port_read(FW_CFG_DATA, 1) as u8, b'Q');
        assert_eq!(cfg.port_read(FW_CFG_DATA, 1) as u8, b'E');
        cfg.port_write(FW_CFG_SELECTOR, 2, u32::from(FW_CFG_SIGNATURE));
        assert_eq!(cfg.port_read(FW_CFG_DATA, 1) as u8, b'Q');
    }

    #[test]
    fn test_file_fwcf_via_file_dir() {
        let mut cfg = FwCfg::new();
        cfg.port_write(FW_CFG_SELECTOR, 2, u32::from(FW_CFG_FILE_DIR));
        let count = {
            let b = read_n(&mut cfg, 4);
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        };
        // The default directory holds the probe file and `etc/max-cpus`, which
        // `new()` publishes because this tree's CPU count is a known fact.
        assert_eq!(count, 2);

        cfg.port_write(FW_CFG_SELECTOR, 2, u32::from(FW_CFG_FILE_DIR));
        let _ = read_n(&mut cfg, 4); // count
        let size = {
            let b = read_n(&mut cfg, 4);
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        };
        let select = {
            let b = read_n(&mut cfg, 2);
            u16::from_be_bytes([b[0], b[1]])
        };
        let _reserved = read_n(&mut cfg, 2);
        let name = {
            let b = read_n(&mut cfg, 56);
            let nul = b.iter().position(|&c| c == 0).unwrap_or(b.len());
            String::from_utf8_lossy(&b[..nul]).into_owned()
        };
        assert_eq!(size, FW_CFG_TEST_FILE_BYTES.len() as u32);
        assert_eq!(select, FW_CFG_FILE_FIRST);
        assert_eq!(name, FW_CFG_TEST_FILE_NAME);

        cfg.port_write(FW_CFG_SELECTOR, 2, u32::from(select));
        assert_eq!(read_n(&mut cfg, size as usize), FW_CFG_TEST_FILE_BYTES);
    }

    #[test]
    fn data_port_writes_ignored() {
        let mut cfg = FwCfg::new();
        cfg.port_write(FW_CFG_SELECTOR, 2, u32::from(FW_CFG_SIGNATURE));
        cfg.port_write(FW_CFG_DATA, 1, u32::from(b'X'));
        assert_eq!(read_n(&mut cfg, 4), b"QEMU");
    }

    #[test]
    fn unknown_selector_reads_zero() {
        let mut cfg = FwCfg::new();
        cfg.port_write(FW_CFG_SELECTOR, 2, 0x00FF);
        assert_eq!(cfg.port_read(FW_CFG_DATA, 1) as u8, 0);
    }

    #[test]
    fn owns_ports() {
        assert!(FwCfg::owns_port(FW_CFG_SELECTOR));
        assert!(FwCfg::owns_port(FW_CFG_DATA));
        assert!(FwCfg::owns_port(FW_CFG_DMA_ADDR_HIGH));
        assert!(FwCfg::owns_port(FW_CFG_DMA_ADDR_LOW));
        assert!(FwCfg::owns_port(FW_CFG_DMA_ADDR_HIGH + 7));
        assert!(!FwCfg::owns_port(FW_CFG_DMA_ADDR_HIGH + 8));
        assert!(!FwCfg::owns_port(0x3F8));
    }

    /// Guest RAM model for DMA tests: flat `Vec` based at physical 0.
    struct TestRam(Vec<u8>);

    impl TestRam {
        fn new() -> Self {
            Self(vec![0u8; 0x400])
        }

        fn put_access(&mut self, at: u64, control: u32, length: u32, address: u64) {
            let at = at as usize;
            self.0[at..at + 4].copy_from_slice(&control.to_be_bytes());
            self.0[at + 4..at + 8].copy_from_slice(&length.to_be_bytes());
            self.0[at + 8..at + 16].copy_from_slice(&address.to_be_bytes());
        }

        fn control(&self, at: u64) -> u32 {
            let at = at as usize;
            u32::from_be_bytes([self.0[at], self.0[at + 1], self.0[at + 2], self.0[at + 3]])
        }
    }

    /// Trigger a DMA operation the way a guest does: latch the `FWCfgDmaAccess`
    /// address (big-endian) then service it against `ram`.
    fn run_dma(cfg: &mut FwCfg, ram: &mut TestRam, access_addr: u64) -> FwCfgDmaOutcome {
        let high = (access_addr >> 32) as u32;
        let low = access_addr as u32;
        cfg.port_write(FW_CFG_DMA_ADDR_HIGH, 4, high.swap_bytes());
        cfg.port_write(FW_CFG_DMA_ADDR_LOW, 4, low.swap_bytes());
        assert!(cfg.dma_pending());
        let snapshot = ram.0.clone();
        let outcome = cfg
            .service_dma(
                |phys| snapshot.get(phys as usize).copied().unwrap_or(0xFF),
                |phys, b| {
                    if let Some(slot) = ram.0.get_mut(phys as usize) {
                        *slot = b;
                    }
                },
            )
            .expect("pending DMA operation");
        assert!(!cfg.dma_pending());
        outcome
    }

    /// QEMU fw_cfg spec, "Guest-side DMA Interface": reading the DMA address
    /// register returns `0x51454d5520434647` ("QEMU CFG") in big-endian format.
    #[test]
    fn dma_address_register_reads_qemu_cfg_signature() {
        let mut cfg = FwCfg::new();
        assert_eq!(FW_CFG_DMA_SIGNATURE, 0x5145_4D55_2043_4647);
        assert_eq!(FW_CFG_DMA_ADDR_HIGH, 0x514);
        assert_eq!(FW_CFG_DMA_ADDR_LOW, 0x518);

        let bytes: Vec<u8> = (0..8)
            .map(|i| cfg.port_read(FW_CFG_DMA_ADDR_HIGH + i, 1) as u8)
            .collect();
        assert_eq!(bytes, FW_CFG_DMA_SIGNATURE.to_be_bytes());

        // A 32-bit IN sees the big-endian halves byte-swapped into the register.
        assert_eq!(
            cfg.port_read(FW_CFG_DMA_ADDR_HIGH, 4),
            0x5145_4D55u32.swap_bytes()
        );
        assert_eq!(
            cfg.port_read(FW_CFG_DMA_ADDR_LOW, 4),
            0x2043_4647u32.swap_bytes()
        );
    }

    /// QEMU fw_cfg spec: bit 1 (read) copies `length` bytes of the current
    /// selector/offset into guest RAM at `address`; control writes back all-clear.
    #[test]
    fn dma_read_copies_selected_item_into_guest_ram() {
        let mut cfg = FwCfg::new();
        let mut ram = TestRam::new();
        ram.put_access(
            0x100,
            FW_CFG_DMA_CTL_SELECT | FW_CFG_DMA_CTL_READ | (u32::from(FW_CFG_SIGNATURE) << 16),
            4,
            0x200,
        );

        let outcome = run_dma(&mut cfg, &mut ram, 0x100);

        assert_eq!(outcome.transferred, 4);
        assert!(!outcome.error());
        assert_eq!(outcome.result_control, 0);
        assert_eq!(&ram.0[0x200..0x204], b"QEMU");
        assert_eq!(ram.control(0x100), 0);
        assert_eq!(cfg.selector(), FW_CFG_SIGNATURE);
        assert_eq!(cfg.offset(), 4);
    }

    /// QEMU fw_cfg spec: bit 3 (select) uses the upper 16 bits as the selector
    /// index and has the same effect as writing the selector register.
    #[test]
    fn dma_select_only_sets_selector_and_resets_offset() {
        let mut cfg = FwCfg::new();
        let mut ram = TestRam::new();
        cfg.port_write(FW_CFG_SELECTOR, 2, u32::from(FW_CFG_SIGNATURE));
        let _ = cfg.port_read(FW_CFG_DATA, 1);
        assert_eq!(cfg.offset(), 1);

        ram.put_access(
            0x100,
            FW_CFG_DMA_CTL_SELECT | (u32::from(FW_CFG_RAM_SIZE) << 16),
            0,
            0,
        );
        let outcome = run_dma(&mut cfg, &mut ram, 0x100);

        assert!(!outcome.error());
        assert_eq!(cfg.selector(), FW_CFG_RAM_SIZE);
        assert_eq!(cfg.offset(), 0);
        assert_eq!(ram.control(0x100), 0);
    }

    /// QEMU fw_cfg spec: bit 2 (skip) advances the offset by `length` with no
    /// guest-memory access.
    #[test]
    fn dma_skip_advances_offset_without_touching_ram() {
        let mut cfg = FwCfg::new();
        let mut ram = TestRam::new();
        ram.put_access(
            0x100,
            FW_CFG_DMA_CTL_SELECT | FW_CFG_DMA_CTL_SKIP | (u32::from(FW_CFG_SIGNATURE) << 16),
            2,
            0x200,
        );

        let outcome = run_dma(&mut cfg, &mut ram, 0x100);

        assert!(!outcome.error());
        assert_eq!(outcome.transferred, 0);
        assert_eq!(cfg.offset(), 2);
        assert_eq!(&ram.0[0x200..0x204], &[0, 0, 0, 0]);

        // The next read continues from the skipped offset.
        ram.put_access(0x100, FW_CFG_DMA_CTL_READ, 2, 0x200);
        run_dma(&mut cfg, &mut ram, 0x100);
        assert_eq!(&ram.0[0x200..0x202], b"MU");
    }

    /// QEMU fw_cfg spec, "Data Register": past the end of an item, reads return
    /// `0x00`. The DMA read path follows the same rule.
    #[test]
    fn dma_read_past_end_of_item_zero_fills() {
        let mut cfg = FwCfg::new();
        let mut ram = TestRam::new();
        ram.0[0x200..0x208].fill(0xCC);
        ram.put_access(
            0x100,
            FW_CFG_DMA_CTL_SELECT | FW_CFG_DMA_CTL_READ | (u32::from(FW_CFG_SIGNATURE) << 16),
            8,
            0x200,
        );

        let outcome = run_dma(&mut cfg, &mut ram, 0x100);

        assert!(!outcome.error());
        assert_eq!(outcome.transferred, 8);
        assert_eq!(&ram.0[0x200..0x208], b"QEMU\0\0\0\0");
    }

    /// This tree does not model item writeability, so control bit 4 (write) is
    /// rejected with the spec's error bit instead of silently corrupting an item.
    #[test]
    fn dma_write_direction_reports_error_bit() {
        let mut cfg = FwCfg::new();
        let mut ram = TestRam::new();
        ram.0[0x200..0x204].copy_from_slice(b"ZZZZ");
        ram.put_access(
            0x100,
            FW_CFG_DMA_CTL_SELECT | FW_CFG_DMA_CTL_WRITE | (u32::from(FW_CFG_SIGNATURE) << 16),
            4,
            0x200,
        );

        let outcome = run_dma(&mut cfg, &mut ram, 0x100);

        assert!(outcome.error());
        assert_eq!(outcome.error, Some(FwCfgDmaError::WriteUnsupported));
        assert_eq!(ram.control(0x100), FW_CFG_DMA_CTL_ERROR);
        // The item is unchanged.
        cfg.port_write(FW_CFG_SELECTOR, 2, u32::from(FW_CFG_SIGNATURE));
        assert_eq!(read_n(&mut cfg, 4), b"QEMU");
    }

    /// QEMU fw_cfg spec: "The value for the register is 0 at startup and after
    /// an operation"; a 64-bit address needs the high half written first.
    #[test]
    fn dma_high_then_low_half_forms_64bit_address_and_register_clears() {
        let mut cfg = FwCfg::new();
        assert_eq!(cfg.dma_address(), 0);

        cfg.port_write(FW_CFG_DMA_ADDR_HIGH, 4, 0x1234_5678u32.swap_bytes());
        assert!(!cfg.dma_pending(), "high half must not trigger");
        assert_eq!(cfg.dma_address(), 0x1234_5678_0000_0000);

        cfg.port_write(FW_CFG_DMA_ADDR_LOW, 4, 0x9ABC_DEF0u32.swap_bytes());
        assert!(cfg.dma_pending());
        assert_eq!(cfg.dma_address(), 0x1234_5678_9ABC_DEF0);

        // Servicing an out-of-range struct still consumes the request and
        // clears the register.
        let outcome = cfg.service_dma(|_| 0xFF, |_, _| {}).expect("pending");
        assert_eq!(outcome.access_addr, 0x1234_5678_9ABC_DEF0);
        assert_eq!(cfg.dma_address(), 0);
        assert!(!cfg.dma_pending());
    }

    /// The DMA address register is a 64-bit register accessed as two 32-bit
    /// halves. Byte/word accesses are not part of the interface this tree
    /// implements and are ignored.
    #[test]
    fn dma_register_ignores_non_dword_writes() {
        let mut cfg = FwCfg::new();
        cfg.port_write(FW_CFG_DMA_ADDR_LOW, 1, 0x40);
        cfg.port_write(FW_CFG_DMA_ADDR_LOW, 2, 0x4040);
        cfg.port_write(FW_CFG_DMA_ADDR_HIGH + 2, 4, 0xFFFF_FFFF);
        assert!(!cfg.dma_pending());
        assert_eq!(cfg.dma_address(), 0);
    }

    /// QEMU fw_cfg spec: ID bit 1 advertises the DMA interface. It is only set
    /// while this device actually decodes and services `0x514`.
    #[test]
    fn id_selector_reports_dma_only_when_enabled() {
        let mut cfg = FwCfg::new();
        assert!(cfg.dma_enabled());
        cfg.port_write(FW_CFG_SELECTOR, 2, u32::from(FW_CFG_ID));
        assert_eq!(
            u32::from_le_bytes(read_n(&mut cfg, 4).try_into().unwrap()),
            FW_CFG_VERSION | FW_CFG_VERSION_DMA
        );

        cfg.set_dma_enabled(false);
        cfg.port_write(FW_CFG_SELECTOR, 2, u32::from(FW_CFG_ID));
        assert_eq!(
            u32::from_le_bytes(read_n(&mut cfg, 4).try_into().unwrap()),
            FW_CFG_VERSION
        );
        // With DMA off the register reads back as open bus and never triggers.
        assert_eq!(cfg.port_read(FW_CFG_DMA_ADDR_HIGH, 4), 0xFFFF_FFFF);
        cfg.port_write(FW_CFG_DMA_ADDR_LOW, 4, 0);
        assert!(!cfg.dma_pending());
    }

    /// Reset clears the guest-visible DMA register without disturbing host
    /// configuration (same discipline as the selector/offset reset).
    #[test]
    fn reset_clears_dma_register_state() {
        let mut cfg = FwCfg::new();
        cfg.port_write(FW_CFG_DMA_ADDR_HIGH, 4, 0x1111_1111);
        cfg.port_write(FW_CFG_DMA_ADDR_LOW, 4, 0x2222_2222);
        assert!(cfg.dma_pending());

        cfg.reset();

        assert!(!cfg.dma_pending());
        assert_eq!(cfg.dma_address(), 0);
        assert!(cfg.dma_enabled());
        assert!(cfg.service_dma(|_| 0, |_, _| {}).is_none());
    }

    /// Spec: ACPI §15 Table 15.4 — the 20-byte descriptor field offsets, and
    /// §15.2 — the range type values.
    #[test]
    fn e820_descriptor_field_layout_matches_acpi_table_15_4() {
        assert_eq!(FW_CFG_E820_ENTRY_SIZE, 20);
        let bytes = E820Entry::new(0x1122_3344_5566_7788, 0x99AA_BBCC_DDEE_FF00, E820_TYPE_NVS)
            .to_descriptor();

        assert_eq!(&bytes[0..8], &0x1122_3344_5566_7788u64.to_le_bytes());
        assert_eq!(&bytes[8..16], &0x99AA_BBCC_DDEE_FF00u64.to_le_bytes());
        assert_eq!(&bytes[16..20], &4u32.to_le_bytes());
    }

    /// Spec: QEMU fw_cfg "File Directory" — a named file gets a selector at or
    /// above `FW_CFG_FILE_FIRST` and a directory entry reporting its size.
    #[test]
    fn set_e820_entries_publishes_and_clears_the_file() {
        let mut cfg = FwCfg::new();
        assert_eq!(cfg.file_selector(FW_CFG_FILE_E820), None);

        let entries = [
            E820Entry::new(0, 0x0009_FC00, E820_TYPE_MEMORY),
            E820Entry::new(0x0010_0000, 0x00F0_0000, E820_TYPE_MEMORY),
        ];
        let selector = cfg.set_e820_entries(&entries).unwrap();
        assert!(selector >= FW_CFG_FILE_FIRST);
        assert_eq!(cfg.file_selector(FW_CFG_FILE_E820), Some(selector));
        assert_eq!(
            cfg.item(selector).map(|i| i.data.len()),
            Some(2 * FW_CFG_E820_ENTRY_SIZE)
        );
        assert!(cfg.file_names().contains(&FW_CFG_FILE_E820));

        assert_eq!(cfg.set_e820_entries(&[]), None);
        assert_eq!(cfg.file_selector(FW_CFG_FILE_E820), None);
        assert!(cfg.item(selector).is_none());
    }

    /// The named-file directory is keyed by name: adding a duplicate is an
    /// error, replacing keeps the selector, and removing frees the name.
    #[test]
    fn file_directory_is_keyed_by_name() {
        let mut cfg = FwCfg::new();
        let a = cfg.add_file("opt/org.x86wasm/a", b"1").unwrap();
        let b = cfg.add_file("opt/org.x86wasm/b", b"2").unwrap();
        assert_ne!(a, b);
        assert!(cfg.add_file("opt/org.x86wasm/a", b"3").is_err());

        assert_eq!(cfg.set_file("opt/org.x86wasm/a", b"333").unwrap(), a);
        assert_eq!(cfg.item(a).map(|i| i.data.clone()), Some(b"333".to_vec()));

        assert_eq!(cfg.remove_file("opt/org.x86wasm/a"), Some(a));
        assert_eq!(cfg.remove_file("opt/org.x86wasm/a"), None);
        assert_eq!(cfg.file_selector("opt/org.x86wasm/a"), None);
        // The freed selector is not recycled onto the next file.
        let c = cfg.add_file("opt/org.x86wasm/c", b"4").unwrap();
        assert_ne!(c, a);
    }

    /// Spec: QEMU fw_cfg "File Directory" — `char name[56]` NUL-terminated.
    #[test]
    fn file_name_length_limit_matches_the_directory_field() {
        assert_eq!(FW_CFG_FILE_NAME_MAX, 55);
        let mut cfg = FwCfg::new();
        assert!(cfg
            .add_file(&"n".repeat(FW_CFG_FILE_NAME_MAX), b"x")
            .is_ok());
        assert!(cfg
            .add_file(&"m".repeat(FW_CFG_FILE_NAME_MAX + 1), b"x")
            .is_err());
    }

    /// The `etc/e820` blob is reachable through the DMA interface, which is how
    /// firmware fetches a multi-entry memory map in one operation.
    #[test]
    fn e820_blob_readable_through_the_dma_interface() {
        let mut cfg = FwCfg::new();
        let selector = cfg
            .set_e820_entries(&[E820Entry::new(0x0010_0000, 0x00F0_0000, E820_TYPE_MEMORY)])
            .unwrap();

        let mut ram = TestRam::new();
        ram.put_access(
            0x100,
            FW_CFG_DMA_CTL_SELECT | FW_CFG_DMA_CTL_READ | (u32::from(selector) << 16),
            FW_CFG_E820_ENTRY_SIZE as u32,
            0x200,
        );
        let outcome = run_dma(&mut cfg, &mut ram, 0x100);

        assert!(!outcome.error());
        assert_eq!(outcome.transferred, FW_CFG_E820_ENTRY_SIZE as u32);
        assert_eq!(
            u64::from_le_bytes(ram.0[0x200..0x208].try_into().unwrap()),
            0x0010_0000
        );
        assert_eq!(
            u32::from_le_bytes(ram.0[0x210..0x214].try_into().unwrap()),
            E820_TYPE_MEMORY
        );
    }

    /// ADR-0005 approved the key list as an interface reference, so `0x0005`
    /// and `0x000F` are now published truthfully. Everything this device still
    /// cannot fill honestly stays absent and reads as `0x00`, which is the
    /// discipline the previous version of this test protected.
    #[test]
    fn only_truthfully_fillable_numeric_selectors_are_present() {
        let mut cfg = FwCfg::with_ram_size(16 * 1024 * 1024);

        assert!(cfg.item(FW_CFG_NB_CPUS).is_some());
        assert!(cfg.item(FW_CFG_MAX_CPUS).is_some());

        // Machine facts this device cannot state on its own.
        for selector in [FW_CFG_UUID, FW_CFG_NOGRAPHIC] {
            assert!(cfg.item(selector).is_none());
            cfg.port_write(FW_CFG_SELECTOR, 2, u32::from(selector));
            assert_eq!(read_n(&mut cfg, 4), [0, 0, 0, 0]);
        }

        // An arbitrary unassigned key is still absent.
        cfg.port_write(FW_CFG_SELECTOR, 2, 0x00FE);
        assert_eq!(read_n(&mut cfg, 4), [0, 0, 0, 0]);
    }

    #[test]
    fn reset_preserves_host_files_and_resets_read_state() {
        let mut cfg = FwCfg::new();
        cfg.port_write(FW_CFG_SELECTOR, 2, u32::from(FW_CFG_SIGNATURE));
        let _ = cfg.port_read(FW_CFG_DATA, 1);
        let extra = cfg.add_file("opt/org.x86wasm/extra", b"x").unwrap();
        cfg.reset();
        assert_eq!(cfg.selector(), 0);
        assert_eq!(cfg.offset(), 0);
        cfg.port_write(FW_CFG_SELECTOR, 2, u32::from(FW_CFG_FILE_DIR));
        let count = {
            let b = read_n(&mut cfg, 4);
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        };
        // Probe file, `etc/max-cpus`, and the one this test added.
        assert_eq!(count, 3);
        cfg.port_write(FW_CFG_SELECTOR, 2, u32::from(extra));
        assert_eq!(read_n(&mut cfg, 1), b"x");
    }
}
