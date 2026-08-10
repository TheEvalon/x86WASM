//! Machine-default fw_cfg `bootorder` (HDD → CD-ROM → floppy).
//!
//! Spec / interface: ADR-0005 bootorder blob layout; SeaBIOS OpenFirmware-style
//! device paths on pc-i440fx. Docs: `docs/fwcfg-r4-selectors.md`.

use devices::{FW_CFG_DEFAULT_BOOT_ORDER, FW_CFG_FILE_BOOTORDER};
use machine_pc::Machine;

fn bootorder_bytes(paths: &[&str]) -> Vec<u8> {
    let mut blob = Vec::new();
    for p in paths {
        blob.extend_from_slice(p.as_bytes());
        blob.push(b'\n');
    }
    blob.push(0);
    blob
}

#[test]
fn sync_publishes_default_hdd_cd_floppy_bootorder() {
    let m = Machine::new(4 * 1024 * 1024);
    let selector = m
        .fw_cfg
        .file_selector(FW_CFG_FILE_BOOTORDER)
        .expect("bootorder present after sync");
    let len = m.fw_cfg.item(selector).map(|i| i.data.len()).unwrap();
    assert_eq!(
        m.fw_cfg.item(selector).map(|i| i.data.clone()),
        Some(bootorder_bytes(FW_CFG_DEFAULT_BOOT_ORDER))
    );
    assert_eq!(len, bootorder_bytes(FW_CFG_DEFAULT_BOOT_ORDER).len());
    assert_eq!(
        FW_CFG_DEFAULT_BOOT_ORDER,
        [
            "/pci@i0cf8/ide@1,1/drive@0/disk@0",
            "/pci@i0cf8/ide@1,1/drive@2/disk@0",
            "/pci@i0cf8/isa@1/fdc@03f0/floppy@0",
        ]
    );
}

#[test]
fn host_bootorder_override_survives_sync_and_can_restore_default() {
    let mut m = Machine::new(4 * 1024 * 1024);
    m.set_fw_cfg_boot_order(&["/pci@i0cf8/ide@1,1/drive@2/disk@0"]);
    assert_eq!(
        m.fw_cfg
            .file_selector(FW_CFG_FILE_BOOTORDER)
            .and_then(|s| m.fw_cfg.item(s).map(|i| i.data.clone())),
        Some(bootorder_bytes(&["/pci@i0cf8/ide@1,1/drive@2/disk@0"]))
    );

    // Empty override removes the file and sticks across sync.
    m.set_fw_cfg_boot_order(&[]);
    assert_eq!(m.fw_cfg.file_selector(FW_CFG_FILE_BOOTORDER), None);
    m.sync_firmware_configuration();
    assert_eq!(m.fw_cfg.file_selector(FW_CFG_FILE_BOOTORDER), None);

    m.use_default_fw_cfg_boot_order();
    assert_eq!(
        m.fw_cfg
            .file_selector(FW_CFG_FILE_BOOTORDER)
            .and_then(|s| m.fw_cfg.item(s).map(|i| i.data.clone())),
        Some(bootorder_bytes(FW_CFG_DEFAULT_BOOT_ORDER))
    );
}
