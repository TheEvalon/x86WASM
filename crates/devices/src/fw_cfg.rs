//! QEMU fw_cfg I/O subset — selector `0x510` + data `0x511`.
//!
//! Spec: [QEMU Firmware Configuration (fw_cfg) Device](https://www.qemu.org/docs/master/specs/fw_cfg.html)
//! — x86 I/O: selector 16-bit LE at `0x510`, data 8-bit at `0x511`; selector
//! write resets the data offset; reads past end return `0x00`; data-port writes
//! ignored (QEMU ≥2.4 traditional interface).
//!
//! This slice: signature key `0x0000` (`QEMU`), one named test file in the
//! file directory (`FW_CFG_FILE_DIR` / `FW_CFG_FILE_FIRST`), no DMA (`0x514`).

use std::collections::BTreeMap;

use crate::PortDevice;

/// Selector (control) register — x86 I/O.
pub const FW_CFG_SELECTOR: u16 = 0x510;
/// Data register — x86 I/O (byte access).
pub const FW_CFG_DATA: u16 = 0x511;

/// Signature item — four ASCII bytes `QEMU`.
pub const FW_CFG_SIGNATURE: u16 = 0x0000;
/// File directory item (`FWCfgFiles`).
pub const FW_CFG_FILE_DIR: u16 = 0x0019;
/// First named-file selector key.
pub const FW_CFG_FILE_FIRST: u16 = 0x0020;

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
}

impl Default for FwCfg {
    fn default() -> Self {
        Self::new()
    }
}

impl FwCfg {
    /// Signature + one test file (`opt/org.x86wasm/test` → `FWCF`).
    pub fn new() -> Self {
        let mut cfg = Self {
            selector: 0,
            offset: 0,
            items: BTreeMap::new(),
            files: Vec::new(),
        };
        cfg.set_item(
            FW_CFG_SIGNATURE,
            FwCfgItem::from_bytes(FW_CFG_SIGNATURE_BYTES),
        );
        cfg.add_file(FW_CFG_TEST_FILE_NAME, FW_CFG_TEST_FILE_BYTES)
            .expect("default test file fits fw_cfg name limit");
        cfg
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn owns_port(port: u16) -> bool {
        port == FW_CFG_SELECTOR || port == FW_CFG_DATA
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
        assert!(!FwCfg::owns_port(0x514));
        assert!(!FwCfg::owns_port(0x3F8));
    }

    #[test]
    fn reset_restores_defaults() {
        let mut cfg = FwCfg::new();
        cfg.port_write(FW_CFG_SELECTOR, 2, u32::from(FW_CFG_SIGNATURE));
        let _ = cfg.port_read(FW_CFG_DATA, 1);
        cfg.add_file("opt/org.x86wasm/extra", b"x").unwrap();
        cfg.reset();
        assert_eq!(cfg.selector(), 0);
        assert_eq!(cfg.offset(), 0);
        cfg.port_write(FW_CFG_SELECTOR, 2, u32::from(FW_CFG_FILE_DIR));
        let count = {
            let b = read_n(&mut cfg, 4);
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        };
        assert_eq!(count, 1);
    }
}
