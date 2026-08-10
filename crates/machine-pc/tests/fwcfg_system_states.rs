//! Machine-default fw_cfg `etc/system-states` (S0 + S5 soft-off).
//!
//! Spec / interface: ADR-0005 blob layout; PM1a sleep stub docs/acpi-r8-pm1-sleep.md;
//! docs/fwcfg-r8-system-states.md.

use devices::{
    FW_CFG_DEFAULT_SYSTEM_STATES, FW_CFG_FILE_SYSTEM_STATES, FW_CFG_SYSTEM_STATES_SIZE,
    FW_CFG_SYSTEM_STATE_ENABLED,
};
use machine_pc::Machine;

#[test]
fn sync_publishes_s0_and_s5_system_states() {
    let m = Machine::new(4 * 1024 * 1024);
    let selector = m
        .fw_cfg
        .file_selector(FW_CFG_FILE_SYSTEM_STATES)
        .expect("etc/system-states present after sync");
    assert_eq!(
        m.fw_cfg.item(selector).map(|i| i.data.clone()),
        Some(FW_CFG_DEFAULT_SYSTEM_STATES.to_vec())
    );
    assert_eq!(
        FW_CFG_DEFAULT_SYSTEM_STATES.len(),
        FW_CFG_SYSTEM_STATES_SIZE
    );
    assert_eq!(FW_CFG_DEFAULT_SYSTEM_STATES[0], FW_CFG_SYSTEM_STATE_ENABLED);
    assert_eq!(FW_CFG_DEFAULT_SYSTEM_STATES[1], 0);
    assert_eq!(FW_CFG_DEFAULT_SYSTEM_STATES[2], 0);
    assert_eq!(FW_CFG_DEFAULT_SYSTEM_STATES[3], 0);
    assert_eq!(FW_CFG_DEFAULT_SYSTEM_STATES[4], 0);
    assert_eq!(FW_CFG_DEFAULT_SYSTEM_STATES[5], FW_CFG_SYSTEM_STATE_ENABLED);
}

#[test]
fn system_states_survives_machine_reset() {
    let mut m = Machine::new(4 * 1024 * 1024);
    m.reset();
    assert_eq!(
        m.fw_cfg
            .file_selector(FW_CFG_FILE_SYSTEM_STATES)
            .and_then(|s| m.fw_cfg.item(s).map(|i| i.data.clone())),
        Some(FW_CFG_DEFAULT_SYSTEM_STATES.to_vec())
    );
}
