//! fw_cfg numeric selectors and named files this machine can answer truthfully.
//!
//! Spec: the [QEMU Firmware Configuration (fw_cfg) Device] specification for the
//! selector/data protocol, the file directory layout, and the rule that reads
//! past the end of an item return `0x00`.
//!
//! Key numbers and blob layouts come from QEMU's `fw_cfg.h` and SeaBIOS's
//! headers as an **interface reference only**, under ADR-0005
//! (`docs/adr/0005-fw-cfg-key-list-interface-reference.md`): key numbers, field
//! widths and blob layouts are facts two implementations must agree on. No
//! implementation was read or copied.
//!
//! The discipline this file exists to hold: a selector or file this machine
//! cannot fill **truthfully** stays absent, so firmware gets the specification's
//! "past the end of the item" answer of `0x00` instead of a fabricated value.
//!
//! [QEMU Firmware Configuration (fw_cfg) Device]: https://www.qemu.org/docs/master/specs/fw_cfg.html

use devices::{FwCfg, PortDevice, FW_CFG_DATA, FW_CFG_FILE_TABLE_LOADER, FW_CFG_SELECTOR};

/// Interface reference (ADR-0005): system UUID, 16 bytes.
const KEY_UUID: u16 = 0x0002;
/// Interface reference (ADR-0005): nographic flag, 16-bit little-endian.
const KEY_NOGRAPHIC: u16 = 0x0004;
/// Interface reference (ADR-0005): boot CPU count, 16-bit little-endian.
const KEY_NB_CPUS: u16 = 0x0005;
/// Interface reference (ADR-0005): maximum CPU count, 16-bit little-endian.
const KEY_MAX_CPUS: u16 = 0x000F;

const FILE_MAX_CPUS: &str = "etc/max-cpus";
const FILE_SYSTEM_STATES: &str = "etc/system-states";
const FILE_BOOTORDER: &str = "bootorder";

fn select(cfg: &mut FwCfg, selector: u16) {
    cfg.port_write(FW_CFG_SELECTOR, 2, u32::from(selector));
}

fn read_n(cfg: &mut FwCfg, selector: u16, n: usize) -> Vec<u8> {
    select(cfg, selector);
    (0..n)
        .map(|_| cfg.port_read(FW_CFG_DATA, 1) as u8)
        .collect()
}

fn read_le16(cfg: &mut FwCfg, selector: u16) -> u16 {
    let b = read_n(cfg, selector, 2);
    u16::from_le_bytes([b[0], b[1]])
}

fn file_bytes(cfg: &mut FwCfg, name: &str) -> Option<Vec<u8>> {
    let selector = cfg.file_selector(name)?;
    let len = cfg.item(selector).map(|i| i.data.len()).unwrap_or(0);
    Some(read_n(cfg, selector, len))
}

/// This machine runs exactly one CPU: there is no SMP anywhere in the tree, no
/// second execution context, and no way to configure one. So the boot-CPU count
/// and the maximum CPU count are both truthfully 1, through all three views
/// firmware may use.
#[test]
fn cpu_count_selectors_and_file_report_the_one_cpu_this_machine_has() {
    let mut cfg = FwCfg::new();

    assert_eq!(read_le16(&mut cfg, KEY_NB_CPUS), 1);
    assert_eq!(read_le16(&mut cfg, KEY_MAX_CPUS), 1);
    assert_eq!(cfg.cpu_count(), 1);
    assert_eq!(
        file_bytes(&mut cfg, FILE_MAX_CPUS),
        Some(vec![0x01, 0x00]),
        "etc/max-cpus is the same 16-bit little-endian count"
    );
}

/// The host owns the CPU count; the three views must never disagree, because
/// firmware picks whichever one it knows about.
#[test]
fn setting_the_cpu_count_keeps_every_view_consistent() {
    let mut cfg = FwCfg::new();
    cfg.set_cpu_count(4);

    assert_eq!(cfg.cpu_count(), 4);
    assert_eq!(read_le16(&mut cfg, KEY_NB_CPUS), 4);
    assert_eq!(read_le16(&mut cfg, KEY_MAX_CPUS), 4);
    assert_eq!(file_bytes(&mut cfg, FILE_MAX_CPUS), Some(vec![0x04, 0x00]));

    // A machine with no CPU cannot boot, so zero is clamped rather than stored.
    cfg.set_cpu_count(0);
    assert_eq!(cfg.cpu_count(), 1);
    assert_eq!(read_le16(&mut cfg, KEY_NB_CPUS), 1);
}

/// A byte, word or dword read of the data port must deliver the same little-
/// endian bytes; firmware reads these counts at more than one width.
#[test]
fn cpu_count_reads_the_same_at_every_data_port_width() {
    let mut cfg = FwCfg::new();
    cfg.set_cpu_count(0x0102);

    select(&mut cfg, KEY_NB_CPUS);
    assert_eq!(cfg.port_read(FW_CFG_DATA, 2), 0x0102);

    select(&mut cfg, KEY_NB_CPUS);
    assert_eq!(cfg.port_read(FW_CFG_DATA, 1), 0x02);
    assert_eq!(cfg.port_read(FW_CFG_DATA, 1), 0x01);
    // Past the end of a two-byte item, the spec's answer is 0x00.
    assert_eq!(cfg.port_read(FW_CFG_DATA, 1), 0x00);
}

/// This machine has no system UUID. Rather than invent one, the item stays
/// absent until a host supplies it — and an absent item reads as zeros, which
/// is the null UUID rather than a fabricated identity.
#[test]
fn uuid_is_absent_until_a_host_supplies_one() {
    let mut cfg = FwCfg::new();
    assert!(cfg.item(KEY_UUID).is_none());
    assert_eq!(read_n(&mut cfg, KEY_UUID, 16), vec![0u8; 16]);

    let uuid = [
        0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE,
        0xFF,
    ];
    cfg.set_system_uuid(uuid);
    assert_eq!(read_n(&mut cfg, KEY_UUID, 16), uuid.to_vec());
    // The blob is exactly 16 bytes; the 17th read is past the end.
    select(&mut cfg, KEY_UUID);
    let _ = read_n(&mut cfg, KEY_UUID, 16);
    assert_eq!(cfg.port_read(FW_CFG_DATA, 1), 0);

    cfg.clear_system_uuid();
    assert!(cfg.item(KEY_UUID).is_none());
}

/// Whether the machine has a display is the machine's fact, not this device's.
/// The item is absent until the host states it, and then it states it exactly.
#[test]
fn nographic_is_absent_until_the_host_states_it() {
    let mut cfg = FwCfg::new();
    assert!(cfg.item(KEY_NOGRAPHIC).is_none());

    cfg.set_nographic(false);
    assert_eq!(read_le16(&mut cfg, KEY_NOGRAPHIC), 0);

    cfg.set_nographic(true);
    assert_eq!(read_le16(&mut cfg, KEY_NOGRAPHIC), 1);
}

/// `bootorder` is a newline-separated, NUL-terminated list of firmware device
/// paths. The bare `FwCfg` device leaves it absent; an empty list removes it
/// rather than publishing an empty policy. (A running `Machine` publishes
/// `FW_CFG_DEFAULT_BOOT_ORDER` through sync — see machine-pc tests.)
#[test]
fn bootorder_is_absent_until_a_host_states_a_policy() {
    let mut cfg = FwCfg::new();
    assert_eq!(cfg.file_selector(FILE_BOOTORDER), None);

    let selector = cfg
        .set_boot_order(&["/pci@i0cf8/ide@1,1/drive@0/disk@0", "/pci@i0cf8/ide@1,1"])
        .expect("non-empty boot order publishes the file");
    assert_eq!(cfg.file_selector(FILE_BOOTORDER), Some(selector));
    assert_eq!(
        file_bytes(&mut cfg, FILE_BOOTORDER),
        Some(b"/pci@i0cf8/ide@1,1/drive@0/disk@0\n/pci@i0cf8/ide@1,1\n\0".to_vec())
    );

    assert_eq!(cfg.set_boot_order(&[]), None);
    assert_eq!(cfg.file_selector(FILE_BOOTORDER), None);
}

/// `etc/system-states` describes which ACPI sleep states the platform supports.
/// The bare [`FwCfg`] device leaves the file absent; a host (or
/// `Machine::sync_firmware_configuration`) may publish a truthful blob via
/// [`FwCfg::set_system_states`]. Spec / model: ADR-0005, docs/fwcfg-r8-system-states.md.
#[test]
fn system_states_absent_on_bare_device_and_publishable() {
    let mut cfg = FwCfg::new();
    assert_eq!(cfg.file_selector(FILE_SYSTEM_STATES), None);
    assert!(!cfg.file_names().contains(&FILE_SYSTEM_STATES));

    // Six bytes indexed by S-state; bit 7 marks a state as supported.
    let states = [0x80u8, 0, 0, 0, 0, 0x80];
    let selector = cfg.set_system_states(states);
    assert_eq!(cfg.file_selector(FILE_SYSTEM_STATES), Some(selector));
    assert_eq!(
        file_bytes(&mut cfg, FILE_SYSTEM_STATES),
        Some(states.to_vec())
    );
}

/// `etc/table-loader` is the QEMU/SeaBIOS ACPI table-loader command stream.
/// This tree builds no ACPI tables (no RSDP/XSDT/FADT), so the honest policy is
/// to omit the file — never publish a zero-entry loader that would still claim
/// the protocol. Spec / policy: ADR-0008, ADR-0005, `docs/fwcfg-r4-selectors.md`.
#[test]
fn table_loader_is_omitted_and_name_lookup_fails_cleanly() {
    assert_eq!(FW_CFG_FILE_TABLE_LOADER, "etc/table-loader");
    let cfg = FwCfg::new();

    // Host name lookup fails cleanly.
    assert_eq!(cfg.file_selector(FW_CFG_FILE_TABLE_LOADER), None);
    assert!(!cfg.file_names().contains(&FW_CFG_FILE_TABLE_LOADER));

    // The file directory does not advertise the name either.
    for name in cfg.file_names() {
        assert_ne!(
            name, FW_CFG_FILE_TABLE_LOADER,
            "directory must not list etc/table-loader"
        );
    }

    // There is no setter: publishing would invent tables that do not exist.
    // Generic `add_file` remains available for host experiments; default and
    // `Machine::sync_firmware_configuration` paths must leave it absent
    // (ADR-0008).
}

/// Host configuration survives a device reset; only the guest-visible selector
/// and read offset are cleared.
#[test]
fn host_configuration_survives_reset() {
    let mut cfg = FwCfg::new();
    cfg.set_cpu_count(2);
    cfg.set_system_uuid([0xA5; 16]);
    cfg.set_nographic(true);

    select(&mut cfg, KEY_NB_CPUS);
    let _ = cfg.port_read(FW_CFG_DATA, 1);
    cfg.reset();

    assert_eq!(cfg.selector(), 0);
    assert_eq!(cfg.offset(), 0);
    assert_eq!(cfg.cpu_count(), 2);
    assert_eq!(read_le16(&mut cfg, KEY_NB_CPUS), 2);
    assert_eq!(read_n(&mut cfg, KEY_UUID, 16), vec![0xA5; 16]);
    assert_eq!(read_le16(&mut cfg, KEY_NOGRAPHIC), 1);
    assert_eq!(file_bytes(&mut cfg, FILE_MAX_CPUS), Some(vec![0x02, 0x00]));
}

/// The named files this device publishes must each appear once in the file
/// directory, so a guest that walks it finds a single unambiguous selector.
#[test]
fn published_files_appear_once_each_in_the_directory() {
    let mut cfg = FwCfg::new();
    cfg.set_boot_order(&["/pci@i0cf8"]);

    let names = cfg.file_names();
    for name in [FILE_MAX_CPUS, FILE_BOOTORDER] {
        assert_eq!(
            names.iter().filter(|n| **n == name).count(),
            1,
            "{name} must appear exactly once"
        );
    }

    // Replacing a policy keeps the selector stable.
    let first = cfg.file_selector(FILE_BOOTORDER);
    cfg.set_boot_order(&["/pci@i0cf8/ide@1,1"]);
    assert_eq!(cfg.file_selector(FILE_BOOTORDER), first);
}
