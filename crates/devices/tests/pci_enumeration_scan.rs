//! What a firmware `pci_probe_devices` scan of bus 0 sees on this machine.
//!
//! Spec: PCI Local Bus Specification Revision 3.0:
//!
//! - §6.1 Figure 6-1 "Type 00h Configuration Space Header".
//! - §6.2.1 "Device Identification": Vendor ID is "read-only" and "FFFFh is an
//!   invalid value for Vendor ID"; Revision ID, Class Code and Header Type are
//!   read-only; Header Type "bit 7 in this register is used to identify a
//!   multi-function device ... If the bit is 0, then the device is
//!   single-function."
//! - §6.2.4 "Miscellaneous Registers": the Capabilities Pointer "is used to
//!   point to a linked list of new capabilities ... This register is only valid
//!   if the 'Capabilities List' bit in the Status Register is set"; BIST "is
//!   optional ... Devices that do not support BIST must always return a value
//!   of 0"; Interrupt Pin "read-only", value 0 meaning "the device does not use
//!   an interrupt pin"; Interrupt Line is read/write.
//! - §6.2.5.3 "Subsystem Vendor ID / Subsystem ID" and the CardBus CIS Pointer:
//!   read-only.
//!
//! A scan walks bus 0, device 0-31, function 0-7, reading the Vendor ID first
//! and consulting the multi-function bit before it looks past function 0. This
//! machine answers on exactly five of those 256 addresses.

use devices::{
    PciConfig, PortDevice, PCI_CLASS_BRIDGE, PCI_CLASS_SERIAL_BUS, PCI_CLASS_STORAGE,
    PCI_CONFIG_ADDRESS, PCI_CONFIG_DATA, PCI_DEVICE_I440FX, PCI_DEVICE_PIIX3_IDE,
    PCI_DEVICE_PIIX3_ISA, PCI_DEVICE_PIIX3_USB, PCI_DEVICE_PIIX_ACPI, PCI_HEADER_MULTIFUNCTION,
    PCI_PROG_IF_UHCI, PCI_STATUS_CAP_LIST, PCI_STATUS_OFFSET, PCI_SUBCLASS_HOST_BRIDGE,
    PCI_SUBCLASS_IDE, PCI_SUBCLASS_ISA_BRIDGE, PCI_SUBCLASS_OTHER_BRIDGE, PCI_SUBCLASS_USB,
    PCI_VENDOR_INTEL,
};

/// Type 0 header register offsets this file asserts against.
const REG_VENDOR_DEVICE: u8 = 0x00;
const REG_REVISION_CLASS: u8 = 0x08;
const REG_BIST_HEADER_LATENCY_CACHE: u8 = 0x0C;
const REG_CARDBUS_CIS: u8 = 0x28;
const REG_SUBSYSTEM: u8 = 0x2C;
const REG_CAP_POINTER: u8 = 0x34;
const REG_RESERVED_38: u8 = 0x38;
const REG_INTERRUPT: u8 = 0x3C;

/// Byte offsets inside the `0x0C` dword.
const BIST_BYTE_LANE: u16 = 3;
const HEADER_TYPE_BYTE_LANE: u16 = 2;

/// One function this machine's PCI tree implements, as a scan sees it.
#[derive(Clone, Copy)]
struct Function {
    device: u8,
    function: u8,
    vendor: u16,
    device_id: u16,
    class: u8,
    subclass: u8,
    prog_if: u8,
    header_type: u8,
}

impl Function {
    const fn at(&self) -> (u8, u8) {
        (self.device, self.function)
    }
}

const EXPECTED: [Function; 5] = [
    Function {
        device: 0,
        function: 0,
        vendor: PCI_VENDOR_INTEL,
        device_id: PCI_DEVICE_I440FX,
        class: PCI_CLASS_BRIDGE,
        subclass: PCI_SUBCLASS_HOST_BRIDGE,
        prog_if: 0x00,
        header_type: 0x00,
    },
    Function {
        device: 1,
        function: 0,
        vendor: PCI_VENDOR_INTEL,
        device_id: PCI_DEVICE_PIIX3_ISA,
        class: PCI_CLASS_BRIDGE,
        subclass: PCI_SUBCLASS_ISA_BRIDGE,
        prog_if: 0x00,
        header_type: PCI_HEADER_MULTIFUNCTION,
    },
    Function {
        device: 1,
        function: 1,
        vendor: PCI_VENDOR_INTEL,
        device_id: PCI_DEVICE_PIIX3_IDE,
        class: PCI_CLASS_STORAGE,
        subclass: PCI_SUBCLASS_IDE,
        prog_if: 0x80,
        header_type: 0x00,
    },
    Function {
        device: 1,
        function: 2,
        vendor: PCI_VENDOR_INTEL,
        device_id: PCI_DEVICE_PIIX3_USB,
        class: PCI_CLASS_SERIAL_BUS,
        subclass: PCI_SUBCLASS_USB,
        prog_if: PCI_PROG_IF_UHCI,
        header_type: 0x00,
    },
    Function {
        device: 1,
        function: 3,
        vendor: PCI_VENDOR_INTEL,
        device_id: PCI_DEVICE_PIIX_ACPI,
        class: PCI_CLASS_BRIDGE,
        subclass: PCI_SUBCLASS_OTHER_BRIDGE,
        prog_if: 0x00,
        header_type: 0x00,
    },
];

fn select(pci: &mut PciConfig, device: u8, function: u8, reg: u8) {
    let addr = PciConfig::make_address(0, device, function, reg, true);
    pci.port_write(PCI_CONFIG_ADDRESS, 4, addr);
}

fn read_dword(pci: &mut PciConfig, device: u8, function: u8, reg: u8) -> u32 {
    select(pci, device, function, reg);
    pci.port_read(PCI_CONFIG_DATA, 4)
}

fn read_byte(pci: &mut PciConfig, device: u8, function: u8, reg: u8, lane: u16) -> u8 {
    select(pci, device, function, reg);
    pci.port_read(PCI_CONFIG_DATA + lane, 1) as u8
}

fn write_dword(pci: &mut PciConfig, device: u8, function: u8, reg: u8, value: u32) {
    select(pci, device, function, reg);
    pci.port_write(PCI_CONFIG_DATA, 4, value);
}

fn write_byte(pci: &mut PciConfig, device: u8, function: u8, reg: u8, lane: u16, value: u8) {
    select(pci, device, function, reg);
    pci.port_write(PCI_CONFIG_DATA + lane, 1, u32::from(value));
}

/// Spec: §6.2.1 — "FFFFh is an invalid value for Vendor ID", which is how a
/// scan learns a function is absent. Sweeping all 256 `(device, function)`
/// addresses on bus 0 must find exactly the five this machine implements.
#[test]
fn a_full_bus_zero_scan_finds_exactly_the_five_implemented_functions() {
    let mut pci = PciConfig::new();
    let mut present: Vec<(u8, u8)> = Vec::new();

    for device in 0u8..32 {
        for function in 0u8..8 {
            let id = read_dword(&mut pci, device, function, REG_VENDOR_DEVICE);
            if id & 0xFFFF == 0xFFFF {
                assert_eq!(
                    id, 0xFFFF_FFFF,
                    "{device:02x}:{function} absent: the whole dword master-aborts"
                );
                continue;
            }
            present.push((device, function));
        }
    }

    let expected: Vec<(u8, u8)> = EXPECTED.iter().map(Function::at).collect();
    assert_eq!(present, expected);
}

/// Spec: §6.2.1 Header Type bit 7 — a scan only probes functions 1-7 of a
/// device whose function 0 sets it. Exactly one device here is multi-function,
/// and probing the way firmware does must still reach all five functions.
#[test]
fn only_the_isa_bridge_device_advertises_multiple_functions() {
    let mut pci = PciConfig::new();
    let mut reached: Vec<(u8, u8)> = Vec::new();

    for device in 0u8..32 {
        if read_dword(&mut pci, device, 0, REG_VENDOR_DEVICE) & 0xFFFF == 0xFFFF {
            continue;
        }
        reached.push((device, 0));
        let header_type = read_byte(
            &mut pci,
            device,
            0,
            REG_BIST_HEADER_LATENCY_CACHE,
            HEADER_TYPE_BYTE_LANE,
        );
        if header_type & PCI_HEADER_MULTIFUNCTION == 0 {
            continue;
        }
        for function in 1u8..8 {
            if read_dword(&mut pci, device, function, REG_VENDOR_DEVICE) & 0xFFFF != 0xFFFF {
                reached.push((device, function));
            }
        }
    }

    let expected: Vec<(u8, u8)> = EXPECTED.iter().map(Function::at).collect();
    assert_eq!(reached, expected);

    // The multi-function bit belongs to function 0 of the device; the sibling
    // functions must not repeat it, or a scan of a device behind a bridge could
    // recurse on the wrong header.
    for f in EXPECTED {
        assert_eq!(
            read_byte(
                &mut pci,
                f.device,
                f.function,
                REG_BIST_HEADER_LATENCY_CACHE,
                HEADER_TYPE_BYTE_LANE
            ),
            f.header_type,
            "{:02x}:{} header type",
            f.device,
            f.function
        );
    }
}

/// Spec: §6.2.1 — Vendor ID, Device ID, Revision ID, Class Code and Header Type
/// are read-only. A scan that could change them by writing would misreport the
/// machine to the next reader.
#[test]
fn identity_class_and_header_type_registers_are_read_only() {
    let mut pci = PciConfig::new();

    for f in EXPECTED {
        let (device, function) = f.at();
        let id = read_dword(&mut pci, device, function, REG_VENDOR_DEVICE);
        assert_eq!(id & 0xFFFF, u32::from(f.vendor));
        assert_eq!(id >> 16, u32::from(f.device_id));

        let class_dword = read_dword(&mut pci, device, function, REG_REVISION_CLASS);
        assert_eq!((class_dword >> 24) as u8, f.class);
        assert_eq!((class_dword >> 16) as u8, f.subclass);
        assert_eq!(
            (class_dword >> 8) as u8,
            f.prog_if,
            "{device:02x}:{function} programming interface"
        );

        write_dword(&mut pci, device, function, REG_VENDOR_DEVICE, 0xDEAD_BEEF);
        write_dword(&mut pci, device, function, REG_REVISION_CLASS, 0xDEAD_BEEF);
        write_byte(
            &mut pci,
            device,
            function,
            REG_BIST_HEADER_LATENCY_CACHE,
            HEADER_TYPE_BYTE_LANE,
            0xFF,
        );

        assert_eq!(
            read_dword(&mut pci, device, function, REG_VENDOR_DEVICE),
            id
        );
        assert_eq!(
            read_dword(&mut pci, device, function, REG_REVISION_CLASS),
            class_dword
        );
        assert_eq!(
            read_byte(
                &mut pci,
                device,
                function,
                REG_BIST_HEADER_LATENCY_CACHE,
                HEADER_TYPE_BYTE_LANE
            ),
            f.header_type
        );
    }
}

/// Spec: §6.2.3 Status bit 4 and §6.7 — the Capabilities Pointer is valid only
/// when the Capabilities List bit is set. No function here has a capability
/// list, so the bit is clear and the pointer must read zero and stay there.
#[test]
fn capabilities_pointer_is_honestly_zero_and_read_only() {
    let mut pci = PciConfig::new();

    for f in EXPECTED {
        let (device, function) = f.at();
        let status =
            (read_dword(&mut pci, device, function, PCI_STATUS_OFFSET & 0xFC) >> 16) as u16;
        assert_eq!(
            status & PCI_STATUS_CAP_LIST,
            0,
            "{device:02x}:{function} advertises a capability list it does not have"
        );

        assert_eq!(read_dword(&mut pci, device, function, REG_CAP_POINTER), 0);
        write_dword(&mut pci, device, function, REG_CAP_POINTER, 0xFFFF_FFFF);
        assert_eq!(
            read_dword(&mut pci, device, function, REG_CAP_POINTER),
            0,
            "{device:02x}:{function} capabilities pointer must stay zero"
        );

        // The reserved dword at 0x38 is read-only zero for the same reason.
        write_dword(&mut pci, device, function, REG_RESERVED_38, 0xFFFF_FFFF);
        assert_eq!(read_dword(&mut pci, device, function, REG_RESERVED_38), 0);
    }
}

/// Spec: §6.2.4 — "Devices that do not support BIST must always return a value
/// of 0". Nothing here implements a built-in self test.
#[test]
fn bist_is_read_only_zero_on_every_function() {
    let mut pci = PciConfig::new();

    for f in EXPECTED {
        let (device, function) = f.at();
        assert_eq!(
            read_byte(
                &mut pci,
                device,
                function,
                REG_BIST_HEADER_LATENCY_CACHE,
                BIST_BYTE_LANE
            ),
            0
        );
        write_byte(
            &mut pci,
            device,
            function,
            REG_BIST_HEADER_LATENCY_CACHE,
            BIST_BYTE_LANE,
            0xFF,
        );
        assert_eq!(
            read_byte(
                &mut pci,
                device,
                function,
                REG_BIST_HEADER_LATENCY_CACHE,
                BIST_BYTE_LANE
            ),
            0,
            "{device:02x}:{function} BIST must stay zero"
        );
    }
}

/// Spec: §6.2.4 — Interrupt Pin is read-only and "a value of 0 indicates that
/// the device does not use an interrupt pin"; Interrupt Line is read/write
/// scratch that POST fills in with the routed IRQ. Min_Gnt and Max_Lat are
/// read-only and zero for a device with no bus-timing requirement.
///
/// No function in this tree drives a PCI interrupt, so every Interrupt Pin is
/// honestly zero.
#[test]
fn interrupt_pin_is_zero_and_read_only_while_interrupt_line_stays_writable() {
    let mut pci = PciConfig::new();

    for f in EXPECTED {
        let (device, function) = f.at();
        assert_eq!(read_dword(&mut pci, device, function, REG_INTERRUPT), 0);

        write_dword(&mut pci, device, function, REG_INTERRUPT, 0xFFFF_FFFF);
        assert_eq!(
            read_dword(&mut pci, device, function, REG_INTERRUPT),
            0x0000_00FF,
            "{device:02x}:{function}: only Interrupt Line is writable"
        );

        write_dword(&mut pci, device, function, REG_INTERRUPT, 0x0000_000B);
        assert_eq!(
            read_dword(&mut pci, device, function, REG_INTERRUPT),
            0x0000_000B
        );
    }
}

/// Spec: §6.2.5.3 — Subsystem Vendor ID and Subsystem ID are read-only, and the
/// CardBus CIS Pointer is read-only. This machine assigns none of them, so a
/// scan must read zero rather than whatever was last written there.
#[test]
fn subsystem_and_cardbus_registers_are_read_only_zero() {
    let mut pci = PciConfig::new();

    for f in EXPECTED {
        let (device, function) = f.at();
        for reg in [REG_CARDBUS_CIS, REG_SUBSYSTEM] {
            assert_eq!(read_dword(&mut pci, device, function, reg), 0);
            write_dword(&mut pci, device, function, reg, 0xFFFF_FFFF);
            assert_eq!(
                read_dword(&mut pci, device, function, reg),
                0,
                "{device:02x}:{function} register {reg:#04x} must stay zero"
            );
        }
    }
}

/// A scan runs before firmware has programmed anything, and again after. Reset
/// must return the whole enumeration surface to its documented power-on state.
#[test]
fn reset_restores_the_enumeration_surface() {
    /// Every header register a scan reads to identify and place a function.
    const SURVEYED: [u8; 5] = [
        REG_VENDOR_DEVICE,
        REG_REVISION_CLASS,
        REG_BIST_HEADER_LATENCY_CACHE,
        REG_CAP_POINTER,
        REG_INTERRUPT,
    ];

    fn survey(pci: &mut PciConfig) -> Vec<u32> {
        let mut out = Vec::new();
        for f in EXPECTED {
            for reg in SURVEYED {
                out.push(read_dword(pci, f.device, f.function, reg));
            }
        }
        out
    }

    let mut pci = PciConfig::new();
    let before = survey(&mut pci);

    for f in EXPECTED {
        let (device, function) = f.at();
        write_dword(&mut pci, device, function, REG_INTERRUPT, 0x0000_000E);
        write_dword(
            &mut pci,
            device,
            function,
            REG_BIST_HEADER_LATENCY_CACHE,
            0xFFFF_FFFF,
        );
    }
    pci.reset();

    assert_eq!(before, survey(&mut pci));
}
