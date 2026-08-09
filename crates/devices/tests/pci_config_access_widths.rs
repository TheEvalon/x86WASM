//! PCI configuration Mechanism #1 access-width matrix at `0xCF8`/`0xCFC`.
//!
//! Spec: PCI Local Bus Specification Revision 3.0 §3.2.2.3.2 "Configuration
//! Mechanism #1":
//!
//! - "Two DWORD I/O locations are used ... The first DWORD location (CF8h)
//!   references a read/write register that is named CONFIG_ADDRESS. The second
//!   DWORD address (CFCh) references a read/write register named CONFIG_DATA."
//! - "Bit 31 is an enable flag ... Bits 30 to 24 are reserved, read-only, and
//!   must return 0's when read ... Bits 1 and 0 are read-only and must return
//!   0's when read." (Figure 3-2)
//! - "Anytime a host bridge sees a full DWORD I/O write from the host to
//!   CONFIG_ADDRESS, the bridge must latch the data into its CONFIG_ADDRESS
//!   register. On full DWORD I/O reads to CONFIG_ADDRESS, the bridge must
//!   return the data in CONFIG_ADDRESS. **Any other types of accesses to this
//!   address (non-DWORD) have no effect on CONFIG_ADDRESS and are executed as
//!   normal I/O transactions on the PCI bus.**"
//! - "When a host bridge sees an I/O access that **falls inside the DWORD
//!   beginning at CONFIG_DATA address**, it checks the Enable bit and the Bus
//!   Number in the CONFIG_ADDRESS register."
//! - "In both Type 0 and Type 1 translations, byte enables for the data
//!   transfers must be directly copied from the processor bus."
//! - Footnote 15: "If the Device Number field selects an IDSEL line that the
//!   bridge does not implement, the bridge must complete the processor access
//!   normally, dropping the data on writes and returning all ones on reads."
//!
//! Nothing responds to the ordinary I/O transactions the first two rules fall
//! back to on this machine, so they read as ISA open bus (all ones) and drop
//! writes. That is the behavior asserted here.
//!
//! There is no compatibility escape hatch: the spec rules are the only
//! behavior. Round 3 added `OUT DX, eAX` (`EF`) to the interpreter, so guest
//! code programs CONFIG_ADDRESS the way hardware requires and the byte-lane
//! policy that stood in for it was removed.

use devices::{PciConfig, PortDevice, PCI_CONFIG_ADDRESS, PCI_CONFIG_DATA};

/// Host bridge `00:00.0` identity dword: device `0x1237`, vendor `0x8086`.
const HOST_BRIDGE_ID_DWORD: u32 = 0x1237_8086;
/// Byte lanes of that dword, low lane first.
const HOST_BRIDGE_ID_BYTES: [u8; 4] = [0x86, 0x80, 0x37, 0x12];

fn select(pci: &mut PciConfig, bus: u8, device: u8, function: u8, reg: u8) {
    let addr = PciConfig::make_address(bus, device, function, reg, true);
    pci.port_write(PCI_CONFIG_ADDRESS, 4, addr);
}

/// Spec: PCI 3.0 §3.2.2.3.2 Figure 3-2 — a full-dword write latches, a full
/// dword read returns the latch, and reserved bits 30:24 / 1:0 read back zero
/// no matter what the guest wrote into them.
#[test]
fn config_address_dword_write_latches_and_masks_reserved_bits() {
    let mut pci = PciConfig::new();
    let addr = PciConfig::make_address(0, 1, 3, 0x40, true);

    // Reserved bits 30:24 and 1:0 set on the way in.
    pci.port_write(PCI_CONFIG_ADDRESS, 4, addr | 0x7F00_0003);

    assert_eq!(pci.port_read(PCI_CONFIG_ADDRESS, 4), addr);
    assert_eq!(pci.port_read(PCI_CONFIG_ADDRESS, 4) & 0x7F00_0003, 0);
}

/// Spec: PCI 3.0 §3.2.2.3.2 — "Any other types of accesses to this address
/// (non-DWORD) have no effect on CONFIG_ADDRESS and are executed as normal I/O
/// transactions on the PCI bus." Nothing on this machine claims those, so the
/// latch is unchanged and the reads are open bus.
#[test]
fn config_address_sub_dword_accesses_have_no_effect() {
    let mut pci = PciConfig::new();
    let addr = PciConfig::make_address(0, 0, 0, 0x00, true);
    pci.port_write(PCI_CONFIG_ADDRESS, 4, addr);

    for port in 0xCF8u16..=0xCFB {
        for size in [1u8, 2] {
            pci.port_write(port, size, 0x5A5A_5A5A);
            assert_eq!(
                pci.port_read(PCI_CONFIG_ADDRESS, 4),
                addr,
                "size {size} write at {port:#06X} must not disturb CONFIG_ADDRESS"
            );
            assert_eq!(
                pci.port_read(port, size),
                0xFFFF_FFFF,
                "size {size} read at {port:#06X} is an ordinary I/O cycle"
            );
        }
    }

    // The latch still selects the host bridge identity dword.
    assert_eq!(pci.port_read(PCI_CONFIG_DATA, 4), HOST_BRIDGE_ID_DWORD);
}

/// Spec: PCI 3.0 §3.2.2.3.2 — only a full dword at CF8h is CONFIG_ADDRESS;
/// "the only I/O Space consumed by this register is a DWORD at the given
/// address", so a dword access starting mid-register is an ordinary I/O cycle.
#[test]
fn config_address_dword_access_at_unaligned_port_has_no_effect() {
    let mut pci = PciConfig::new();
    let addr = PciConfig::make_address(0, 0, 0, 0x00, true);
    pci.port_write(PCI_CONFIG_ADDRESS, 4, addr);

    for port in 0xCF9u16..=0xCFB {
        pci.port_write(port, 4, 0xDEAD_BEEF);
        assert_eq!(pci.port_read(PCI_CONFIG_ADDRESS, 4), addr);
        assert_eq!(pci.port_read(port, 4), 0xFFFF_FFFF);
    }
}

/// Spec: PCI 3.0 §3.2.2.3.2 — "byte enables for the data transfers must be
/// directly copied from the processor bus", and the I/O access selects them by
/// where it falls inside the dword beginning at CFCh.
#[test]
fn config_data_byte_and_word_lanes_within_the_dword() {
    let mut pci = PciConfig::new();
    select(&mut pci, 0, 0, 0, 0x00);

    for (lane, expected) in HOST_BRIDGE_ID_BYTES.iter().enumerate() {
        let port = PCI_CONFIG_DATA + lane as u16;
        assert_eq!(pci.port_read(port, 1) as u8, *expected, "byte lane {lane}");
    }

    // Word accesses at every offset that still fits inside the dword,
    // including the unaligned lane 1 pair.
    assert_eq!(pci.port_read(PCI_CONFIG_DATA, 2) as u16, 0x8086);
    assert_eq!(pci.port_read(PCI_CONFIG_DATA + 1, 2) as u16, 0x3780);
    assert_eq!(pci.port_read(PCI_CONFIG_DATA + 2, 2) as u16, 0x1237);

    assert_eq!(pci.port_read(PCI_CONFIG_DATA, 4), HOST_BRIDGE_ID_DWORD);
}

/// Spec: PCI 3.0 §3.2.2.3.2 — a configuration cycle is generated for an access
/// that "falls inside the DWORD beginning at CONFIG_DATA address". An access
/// running past CFFh does not, so it must not read or write the *next*
/// configuration register: the byte enables of one cycle cannot straddle two
/// dwords.
#[test]
fn config_data_accesses_straddling_the_dword_are_not_config_cycles() {
    let mut pci = PciConfig::new();

    // Program a recognizable value into host-bridge Cache Line Size (0x0C) so a
    // straddling access at register 0x08 would be visible if it folded over.
    select(&mut pci, 0, 0, 0, 0x0C);
    pci.port_write(PCI_CONFIG_DATA, 1, 0x40);
    assert_eq!(pci.port_read(PCI_CONFIG_DATA, 1) as u8, 0x40);

    select(&mut pci, 0, 0, 0, 0x08);
    let straddling: [(u16, u8); 4] = [
        (PCI_CONFIG_DATA + 3, 2),
        (PCI_CONFIG_DATA + 1, 4),
        (PCI_CONFIG_DATA + 2, 4),
        (PCI_CONFIG_DATA + 3, 4),
    ];
    for (port, size) in straddling {
        assert_eq!(
            pci.port_read(port, size),
            0xFFFF_FFFF,
            "read size {size} at {port:#06X} leaves the CFCh dword"
        );
        pci.port_write(port, size, 0x0000_0000);
    }

    // Neither the selected register nor the following one moved.
    select(&mut pci, 0, 0, 0, 0x08);
    let class_dword = pci.port_read(PCI_CONFIG_DATA, 4);
    assert_eq!((class_dword >> 24) as u8, 0x06, "class code still bridge");
    select(&mut pci, 0, 0, 0, 0x0C);
    assert_eq!(pci.port_read(PCI_CONFIG_DATA, 1) as u8, 0x40);
}

/// Spec: Intel SDM Vol. 2 `IN`/`OUT` transfer 1, 2, or 4 bytes; anything else
/// never reaches a configuration cycle. Recorded so a caller that invents a
/// width gets open bus rather than a partial register.
#[test]
fn config_data_unsupported_widths_are_open_bus() {
    let mut pci = PciConfig::new();
    select(&mut pci, 0, 0, 0, 0x00);
    for size in [0u8, 3, 5, 8] {
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, size), 0xFFFF_FFFF);
    }
    assert_eq!(pci.port_read(PCI_CONFIG_DATA, 4), HOST_BRIDGE_ID_DWORD);
}

/// Spec: PCI 3.0 §3.2.2.3.2 — with the Enable bit clear the bridge does not
/// translate CONFIG_DATA accesses; they run as ordinary I/O transactions,
/// which nothing here claims. This is also the state at reset, before any
/// firmware has programmed CONFIG_ADDRESS at all.
#[test]
fn config_data_is_open_bus_until_config_address_is_programmed() {
    let mut pci = PciConfig::new();
    assert_eq!(pci.port_read(PCI_CONFIG_ADDRESS, 4), 0);

    for lane in 0u16..4 {
        assert_eq!(pci.port_read(PCI_CONFIG_DATA + lane, 1), 0xFFFF_FFFF);
        pci.port_write(PCI_CONFIG_DATA + lane, 1, 0x00);
    }
    assert_eq!(pci.port_read(PCI_CONFIG_DATA, 4), 0xFFFF_FFFF);

    // Explicitly disabled after a valid target was selected: same answer, and
    // the dropped writes did not reach the host bridge.
    select(&mut pci, 0, 0, 0, 0x0C);
    pci.port_write(PCI_CONFIG_DATA, 1, 0x20);
    let disabled = PciConfig::make_address(0, 0, 0, 0x0C, false);
    pci.port_write(PCI_CONFIG_ADDRESS, 4, disabled);
    assert_eq!(pci.port_read(PCI_CONFIG_DATA, 4), 0xFFFF_FFFF);
    pci.port_write(PCI_CONFIG_DATA, 1, 0x7F);

    select(&mut pci, 0, 0, 0, 0x0C);
    assert_eq!(pci.port_read(PCI_CONFIG_DATA, 1) as u8, 0x20);
}

/// Spec: PCI 3.0 §3.2.2.3.2 footnote 15 — an unimplemented IDSEL completes the
/// processor access "dropping the data on writes and returning all ones on
/// reads"; §3.2.2.3.4 — "Configuration transactions that are not claimed by a
/// device are terminated with Master-Abort". Every width and lane must agree.
#[test]
fn absent_device_and_function_return_all_ones_at_every_width() {
    let mut pci = PciConfig::new();
    // 00:1F.0 (no such device), 00:01.4 (PIIX stubs only 0-3), and bus 1
    // (a Type 1 cycle with no bridge behind this host bridge).
    for (bus, device, function) in [(0u8, 0x1Fu8, 0u8), (0, 1, 4), (1, 0, 0)] {
        select(&mut pci, bus, device, function, 0x00);
        for lane in 0u16..4 {
            assert_eq!(
                pci.port_read(PCI_CONFIG_DATA + lane, 1),
                0xFFFF_FFFF,
                "{bus:02X}:{device:02X}.{function} byte lane {lane}"
            );
        }
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 2), 0xFFFF_FFFF);
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 4), 0xFFFF_FFFF);

        // Writes are dropped without disturbing anything else.
        pci.port_write(PCI_CONFIG_DATA, 4, 0x1234_5678);
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 4), 0xFFFF_FFFF);
    }

    select(&mut pci, 0, 0, 0, 0x00);
    assert_eq!(pci.port_read(PCI_CONFIG_DATA, 4), HOST_BRIDGE_ID_DWORD);
}

/// Spec: PCI 3.0 §3.2.2.3.2 — the vendor ID of a present function never reads
/// all ones, which is how firmware distinguishes present from absent. The
/// SeaBIOS enumeration order (`00:00.0` then `00:01.0`-`00:01.3`) is what this
/// machine must answer for.
#[test]
fn present_functions_report_a_vendor_id_and_absent_ones_do_not() {
    let mut pci = PciConfig::new();
    for function in 0u8..4 {
        select(&mut pci, 0, 1, function, 0x00);
        let vendor = pci.port_read(PCI_CONFIG_DATA, 2) as u16;
        assert_eq!(vendor, 0x8086, "00:01.{function} vendor");
    }
    for function in 4u8..8 {
        select(&mut pci, 0, 1, function, 0x00);
        assert_eq!(pci.port_read(PCI_CONFIG_DATA, 2) as u16, 0xFFFF);
    }
}

/// Spec: PCI 3.0 §3.2.2.3.2 — the register number is bits 7:2, so the whole
/// 256-byte legacy configuration space is reachable and the last dword
/// (`0xFC`) must not run off the end of the register file.
#[test]
fn every_dword_of_the_legacy_config_space_is_addressable() {
    let mut pci = PciConfig::new();
    for reg in (0u16..=0xFC).step_by(4) {
        select(&mut pci, 0, 0, 0, reg as u8);
        let _ = pci.port_read(PCI_CONFIG_DATA, 4);
        assert_eq!(pci.port_read(PCI_CONFIG_DATA + 3, 1) & 0xFFFF_FF00, 0);
    }
}
