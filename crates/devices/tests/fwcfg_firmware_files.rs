//! Guest-visible behavior of the fw_cfg firmware-file set: the `etc/e820`
//! memory map and the named-file directory that carries it.
//!
//! Spec:
//!
//! - QEMU Firmware Configuration (fw_cfg) Device — "File Directory (Key
//!   0x0019, FW_CFG_FILE_DIR)": `FWCfgFiles { uint32 count; FWCfgFile f[]; }`
//!   with `FWCfgFile { uint32 size; uint16 select; uint16 reserved; char
//!   name[56]; }`, counts and selectors big-endian, names NUL-terminated ASCII;
//!   items at selector `0x0020` (`FW_CFG_FILE_FIRST`) or higher have a
//!   directory entry. "Data Register": reads past the end of an item return
//!   `0x00`.
//! - ACPI Specification §15 "System Address Map Interfaces", Table 15.4
//!   "Address Range Descriptor Structure" — the 20-byte minimum descriptor
//!   (`BaseAddrLow`, `BaseAddrHigh`, `LengthLow`, `LengthHigh`, `Type`) and
//!   §15.2 "Address Range Types" (1 = AddressRangeMemory, 2 =
//!   AddressRangeReserved).
//!
//! Integration tests may only use the crate's re-exported surface, so the
//! `etc/e820` file name, entry size, and range types are repeated here as local
//! literals with their citation, and entries are built through
//! `FwCfg::e820_entry`, until `devices/src/lib.rs` re-exports the
//! `FW_CFG_FILE_E820` / `FW_CFG_E820_*` / `E820_TYPE_*` / `E820Entry` items.

use devices::{
    FwCfg, PortDevice, FW_CFG_DATA, FW_CFG_FILE_DIR, FW_CFG_FILE_FIRST, FW_CFG_SELECTOR,
};

/// Spec: QEMU fw_cfg externally provided items — the memory map firmware file.
const FILE_E820: &str = "etc/e820";
/// Spec: ACPI §15 Table 15.4 — the 20-byte minimum Address Range Descriptor.
const E820_ENTRY_SIZE: usize = 20;
/// Spec: ACPI §15.2 Address Range Types.
const E820_TYPE_MEMORY: u32 = 1;
const E820_TYPE_RESERVED: u32 = 2;

fn select(cfg: &mut FwCfg, selector: u16) {
    cfg.port_write(FW_CFG_SELECTOR, 2, u32::from(selector));
}

fn read_n(cfg: &mut FwCfg, n: usize) -> Vec<u8> {
    (0..n)
        .map(|_| cfg.port_read(FW_CFG_DATA, 1) as u8)
        .collect()
}

/// One directory entry as the guest parses it.
struct DirEntry {
    size: u32,
    select: u16,
    name: String,
}

fn read_file_dir(cfg: &mut FwCfg) -> Vec<DirEntry> {
    select(cfg, FW_CFG_FILE_DIR);
    let count = {
        let b = read_n(cfg, 4);
        u32::from_be_bytes([b[0], b[1], b[2], b[3]])
    };
    (0..count)
        .map(|_| {
            let size = {
                let b = read_n(cfg, 4);
                u32::from_be_bytes([b[0], b[1], b[2], b[3]])
            };
            let sel = {
                let b = read_n(cfg, 2);
                u16::from_be_bytes([b[0], b[1]])
            };
            let reserved = read_n(cfg, 2);
            assert_eq!(reserved, [0, 0], "FWCfgFile.reserved must be zero");
            let name = {
                let b = read_n(cfg, 56);
                let nul = b.iter().position(|&c| c == 0).unwrap_or(b.len());
                String::from_utf8_lossy(&b[..nul]).into_owned()
            };
            DirEntry {
                size,
                select: sel,
                name,
            }
        })
        .collect()
}

fn has_file(cfg: &mut FwCfg, name: &str) -> bool {
    read_file_dir(cfg).iter().any(|e| e.name == name)
}

/// An empty memory map is not a memory map. A device that cannot describe the
/// address space must leave `etc/e820` out of the directory entirely rather
/// than advertise a zero-length file that firmware would read as "no RAM".
#[test]
fn e820_file_is_absent_until_the_host_supplies_entries() {
    let mut cfg = FwCfg::with_ram_size(16 * 1024 * 1024);
    assert!(!has_file(&mut cfg, FILE_E820));

    assert_eq!(cfg.set_e820_entries(&[]), None);
    assert!(!has_file(&mut cfg, FILE_E820));
}

/// Spec: ACPI §15 Table 15.4 — 64-bit base, 64-bit length, 32-bit type, packed
/// into the 20-byte minimum descriptor and read back through the data port.
#[test]
fn e820_entries_encode_as_acpi_address_range_descriptors() {
    let mut cfg = FwCfg::with_ram_size(16 * 1024 * 1024);
    let low = FwCfg::e820_entry(0x0000_0000, 0x0009_FC00, E820_TYPE_MEMORY);
    let ebda = FwCfg::e820_entry(0x0009_FC00, 0x0000_0400, E820_TYPE_RESERVED);
    let high = FwCfg::e820_entry(0x0010_0000, 0x00F0_0000, E820_TYPE_MEMORY);

    let selector = cfg
        .set_e820_entries(&[low, ebda, high])
        .expect("non-empty map publishes the file");
    assert!(selector >= FW_CFG_FILE_FIRST);

    let entry = read_file_dir(&mut cfg)
        .into_iter()
        .find(|e| e.name == FILE_E820)
        .expect("etc/e820 present once entries exist");
    assert_eq!(entry.size as usize, 3 * E820_ENTRY_SIZE);
    assert_eq!(entry.select, selector);

    select(&mut cfg, selector);
    let blob = read_n(&mut cfg, entry.size as usize);

    let decode = |i: usize| -> (u64, u64, u32) {
        let e = &blob[i * E820_ENTRY_SIZE..(i + 1) * E820_ENTRY_SIZE];
        (
            u64::from_le_bytes(e[0..8].try_into().unwrap()),
            u64::from_le_bytes(e[8..16].try_into().unwrap()),
            u32::from_le_bytes(e[16..20].try_into().unwrap()),
        )
    };
    assert_eq!(decode(0), (0x0000_0000, 0x0009_FC00, E820_TYPE_MEMORY));
    assert_eq!(decode(1), (0x0009_FC00, 0x0000_0400, E820_TYPE_RESERVED));
    assert_eq!(decode(2), (0x0010_0000, 0x00F0_0000, E820_TYPE_MEMORY));

    // Spec: "Data Register" — past the end of the item, reads return 0x00.
    assert_eq!(cfg.port_read(FW_CFG_DATA, 1) as u8, 0);
}

/// Entries above 4 GB must survive the encoding, which is the whole point of
/// the descriptor's 64-bit base and length fields.
#[test]
fn e820_entries_carry_full_64_bit_base_and_length() {
    let mut cfg = FwCfg::new();
    let entry = FwCfg::e820_entry(
        0x0000_0001_0000_0000,
        0x0000_0004_0000_0000,
        E820_TYPE_MEMORY,
    );
    let selector = cfg.set_e820_entries(&[entry]).unwrap();

    select(&mut cfg, selector);
    let blob = read_n(&mut cfg, E820_ENTRY_SIZE);
    assert_eq!(
        u64::from_le_bytes(blob[0..8].try_into().unwrap()),
        0x0000_0001_0000_0000
    );
    assert_eq!(
        u64::from_le_bytes(blob[8..16].try_into().unwrap()),
        0x0000_0004_0000_0000
    );
}

/// Reconfiguring the map replaces it: the selector stays stable so a guest that
/// already read the directory is not invalidated, and the directory reports the
/// new size. Clearing it removes the file again.
#[test]
fn e820_replacement_keeps_selector_and_clearing_removes_the_file() {
    let mut cfg = FwCfg::new();
    let a = FwCfg::e820_entry(0, 0x1000, E820_TYPE_MEMORY);
    let b = FwCfg::e820_entry(0x1000, 0x1000, E820_TYPE_RESERVED);

    let first = cfg.set_e820_entries(&[a]).unwrap();
    let second = cfg.set_e820_entries(&[a, b]).unwrap();
    assert_eq!(first, second);

    let entry = read_file_dir(&mut cfg)
        .into_iter()
        .find(|e| e.name == FILE_E820)
        .unwrap();
    assert_eq!(entry.size as usize, 2 * E820_ENTRY_SIZE);

    assert_eq!(cfg.set_e820_entries(&[]), None);
    assert!(!has_file(&mut cfg, FILE_E820));
    // The selector now reads as an unknown item: 0x00, not stale content.
    select(&mut cfg, second);
    assert_eq!(
        read_n(&mut cfg, E820_ENTRY_SIZE),
        vec![0u8; E820_ENTRY_SIZE]
    );
}

/// Spec: QEMU fw_cfg — a firmware file is identified by name, so two entries
/// with the same name would make the directory ambiguous.
#[test]
fn duplicate_file_names_are_rejected_and_set_file_replaces_instead() {
    let mut cfg = FwCfg::new();
    let selector = cfg.add_file("opt/org.x86wasm/a", b"one").unwrap();
    assert!(cfg.add_file("opt/org.x86wasm/a", b"two").is_err());

    assert_eq!(
        cfg.set_file("opt/org.x86wasm/a", b"three").unwrap(),
        selector
    );
    let dir = read_file_dir(&mut cfg);
    assert_eq!(
        dir.iter().filter(|e| e.name == "opt/org.x86wasm/a").count(),
        1
    );

    select(&mut cfg, selector);
    assert_eq!(read_n(&mut cfg, 5), b"three");
}

/// Spec: QEMU fw_cfg — `char name[56]`, NUL-terminated, so 55 characters is the
/// longest storable name.
#[test]
fn file_names_longer_than_the_directory_field_are_rejected() {
    let mut cfg = FwCfg::new();
    let ok = "o".repeat(55);
    let too_long = "o".repeat(56);
    assert!(cfg.add_file(&ok, b"x").is_ok());
    assert!(cfg.add_file(&too_long, b"x").is_err());
    assert!(cfg.add_file("opt/has\0nul", b"x").is_err());
}

/// Host configuration, including the memory map, survives a device reset the
/// way the RAM-size item already does; only the read stream is rewound.
#[test]
fn e820_file_survives_reset() {
    let mut cfg = FwCfg::with_ram_size(16 * 1024 * 1024);
    let entry = FwCfg::e820_entry(0x0010_0000, 0x00F0_0000, E820_TYPE_MEMORY);
    let selector = cfg.set_e820_entries(&[entry]).unwrap();
    select(&mut cfg, selector);
    let _ = cfg.port_read(FW_CFG_DATA, 1);

    cfg.reset();

    assert_eq!(cfg.selector(), 0);
    assert_eq!(cfg.offset(), 0);
    assert_eq!(cfg.file_selector(FILE_E820), Some(selector));
    select(&mut cfg, selector);
    let blob = read_n(&mut cfg, E820_ENTRY_SIZE);
    assert_eq!(
        u64::from_le_bytes(blob[0..8].try_into().unwrap()),
        0x0010_0000
    );
}

/// The selectors this device does not implement must read as absent rather than
/// as a fabricated value. Firmware probing NB_CPUS or a boot order gets `0x00`,
/// which is the spec's "past the end of the item" answer for an unknown key.
#[test]
fn unimplemented_numeric_selectors_are_absent_not_invented() {
    let mut cfg = FwCfg::with_ram_size(16 * 1024 * 1024);
    // 0x0002 UUID, 0x0004 nographic, 0x0005 NB_CPUS, 0x000F max-cpus … none of
    // these are populated by this device.
    for selector in [0x0002u16, 0x0004, 0x0005, 0x000F, 0x0010, 0x0018] {
        select(&mut cfg, selector);
        assert_eq!(
            read_n(&mut cfg, 8),
            vec![0u8; 8],
            "selector {selector:#06x} must not report invented data"
        );
    }
    for name in [
        "etc/max-cpus",
        "etc/system-states",
        "etc/table-loader",
        "bootorder",
    ] {
        assert_eq!(cfg.file_selector(name), None, "{name} must be absent");
    }
}
