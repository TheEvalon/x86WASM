use devices::{PortDevice, PIT_CH1_DATA, PIT_CONTROL, PORT61_REFRESH_TOGGLE, PORT_SYSTEM_CONTROL};
use machine_pc::Machine;
use x86_core::{CpuState, SegmentReg};

/// Spec: Intel 8254 mode 2 + IBM PC/AT System Control Port B — channel-1
/// refresh edges toggle read-only port `0x61` bit 4 without asserting IRQ0.
#[test]
fn guest_reads_refresh_toggle_and_cannot_overwrite_it() {
    let mut machine = Machine::new(64 * 1024);
    machine.pit.port_write(PIT_CONTROL, 1, 0x74); // ch1 lohi mode 2
    machine.pit.port_write(PIT_CH1_DATA, 1, 0x02);
    machine.pit.port_write(PIT_CH1_DATA, 1, 0x00);

    let master_irr_before = machine.pic.master.irr;
    assert!(machine.pit.tick_ch1(4)); // first refresh rising edge
    assert_eq!(machine.pic.master.irr, master_irr_before);

    let program = [
        0xB0,
        0x00, // mov al, 0: attempt to clear all port-61 bits
        0xE6,
        PORT_SYSTEM_CONTROL as u8, // out 0x61, al
        0xE4,
        PORT_SYSTEM_CONTROL as u8, // in al, 0x61
        0xF4,                      // hlt
    ];
    for (offset, byte) in program.into_iter().enumerate() {
        machine.mem.write_u8(offset as u64, byte).unwrap();
    }
    machine.cpu = CpuState::reset();
    machine.cpu.cs = SegmentReg::real_mode_code(0);
    machine.cpu.set_ip16(0);
    machine.cpu.halted = false;

    machine.run(16).unwrap();

    assert!(machine.cpu.halted);
    assert_eq!(
        machine.cpu.al() & PORT61_REFRESH_TOGGLE,
        PORT61_REFRESH_TOGGLE
    );
    assert_eq!(machine.pic.master.irr, master_irr_before);

    machine.reset();
    assert_eq!(
        machine.pit.port61_read() & PORT61_REFRESH_TOGGLE,
        0,
        "machine reset must clear refresh detect"
    );
}

/// Spec: IBM PC/AT refresh — [`Machine::tick_pit`] advances ch1 so bit4 toggles
/// without a separate `tick_ch1` call (POST step-clock path).
#[test]
fn tick_pit_advances_refresh_detect_bit4() {
    let mut machine = Machine::new(64 * 1024);
    machine.pit.port_write(PIT_CONTROL, 1, 0x74); // ch1 lohi mode 2
    machine.pit.port_write(PIT_CH1_DATA, 1, 0x02);
    machine.pit.port_write(PIT_CH1_DATA, 1, 0x00);
    assert_eq!(machine.pit.port61_read() & PORT61_REFRESH_TOGGLE, 0);
    let irr_before = machine.pic.master.irr;
    machine.tick_pit(4); // load + countdown + rising refresh edge
    assert_eq!(
        machine.pit.port61_read() & PORT61_REFRESH_TOGGLE,
        PORT61_REFRESH_TOGGLE
    );
    assert_eq!(
        machine.pic.master.irr, irr_before,
        "ch1 refresh must not assert IRQ0"
    );
}
