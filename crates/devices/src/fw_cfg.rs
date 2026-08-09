//! QEMU fw_cfg I/O subset — selector `0x510` + data `0x511`.
//!
//! Spec: [QEMU Firmware Configuration (fw_cfg) Device](https://www.qemu.org/docs/master/specs/fw_cfg.html)
//! — x86 I/O: selector 16-bit LE at `0x510`, data 8-bit at `0x511`; selector
//! write resets the data offset; reads past end return `0x00`; data-port writes
//! ignored (QEMU ≥2.4 traditional interface).
//!
//! This device: signature key `0x0000` (`QEMU`), ID key `0x0001`, RAM-size key
//! `0x0003`, and named files in the file directory (`FW_CFG_FILE_DIR` /
//! `FW_CFG_FILE_FIRST`).
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
        cfg
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

    /// Insert a named file at the next free selector ≥ [`FW_CFG_FILE_FIRST`].
    ///
    /// Rebuilds [`FW_CFG_FILE_DIR`]. Names longer than 55 ASCII chars (plus NUL)
    /// are rejected.
    pub fn add_file(&mut self, name: &str, data: impl Into<Vec<u8>>) -> Result<u16, &'static str> {
        if name.len() > 55 || name.contains('\0') {
            return Err("fw_cfg file name must be ≤55 ASCII chars without NUL");
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
        assert_eq!(count, 1);

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
        assert_eq!(count, 2);
        cfg.port_write(FW_CFG_SELECTOR, 2, u32::from(extra));
        assert_eq!(read_n(&mut cfg, 1), b"x");
    }
}
