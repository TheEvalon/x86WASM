//! Machine fw_cfg sync must not invent `etc/table-loader`.
//!
//! Spec / policy: ADR-0008, ADR-0005, `docs/fwcfg-r4-selectors.md` — the
//! QEMU/SeaBIOS table-loader command stream installs ACPI tables; this machine
//! has none, so `Machine::sync_firmware_configuration` keeps the name absent.

use devices::FW_CFG_FILE_TABLE_LOADER;
use machine_pc::Machine;

#[test]
fn sync_firmware_configuration_leaves_table_loader_absent() {
    let m = Machine::new(4 * 1024 * 1024);
    assert_eq!(m.fw_cfg.file_selector(FW_CFG_FILE_TABLE_LOADER), None);
    assert!(!m.fw_cfg.file_names().contains(&FW_CFG_FILE_TABLE_LOADER));
}
