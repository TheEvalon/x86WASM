//! Base Address Register sizing through the all-ones protocol.
//!
//! Spec: PCI Local Bus Specification Revision 3.0 §6.2.5.1 "Base Addresses":
//!
//! - "Devices are allowed to consume more address space than required, but
//!   decoding down to a 4 KB space for memory is suggested for devices that
//!   need less than that amount ... A device that wants a 1 MB memory address
//!   space (using a 32-bit base address register) would build the top 12 bits
//!   of the address register, hardwiring the other bits to 0."
//! - "Software saves the original value of the Base Address register, writes
//!   0 FFFF FFFFh to the register, then reads it back. Size calculation can be
//!   done from the 32-bit value read by first clearing encoding information
//!   bits (bit 0 for I/O, bits 0-3 for memory), inverting all 32 bits (logical
//!   NOT), then incrementing by 1. The resultant 32-bit value is the memory/IO
//!   range size decoded by the register."
//! - Figure 6-6 / 6-7: bit 0 is the read-only space indicator (0 = memory,
//!   1 = I/O); for a memory register bits 2:1 are the type and bit 3
//!   prefetchable; for an I/O register bit 1 is reserved and reads zero.
//! - §6.2.5.1 "Expansion ROM Base Address Register" at `0x30`: a device with
//!   no expansion ROM returns zero from every bit of the register.
//!
//! Every Base Address Register this machine's PCI tree implements is an I/O
//! register: PIIX IDE BMIBA (`00:01.1` offset `0x20`, 16 bytes) and PIIX USB
//! UHCI (`00:01.2` offset `0x20`, 32 bytes). Every other BAR position on every
//! function — including all six on the host bridge, the ISA bridge and the
//! ACPI function, and the expansion ROM register everywhere — is
//! unimplemented and must read as zero, because a register that stores what
//! firmware wrote during sizing reports a region the device does not decode.

use devices::{
    PciConfig, PortDevice, PCI_BAR_IO_SPACE, PCI_CONFIG_ADDRESS, PCI_CONFIG_DATA,
    PCI_PIIX_IDE_BMIBA_OFFSET, PCI_PIIX_USB_BAR0_OFFSET,
};

/// Type 0 header Base Address Register offsets, BAR0–BAR5.
const BAR_OFFSETS: [u8; 6] = [0x10, 0x14, 0x18, 0x1C, 0x20, 0x24];
/// Expansion ROM Base Address Register offset.
const ROM_BAR_OFFSET: u8 = 0x30;

/// Every `(device, function)` this machine answers for.
const FUNCTIONS: [(u8, u8); 5] = [(0, 0), (1, 0), (1, 1), (1, 2), (1, 3)];

/// Spec: Intel 82371SB — Bus Master IDE register block is 16 bytes.
const BMIDE_BAR_SIZE: u32 = 16;
/// Spec: Universal Host Controller Interface — 32-byte I/O footprint.
const UHCI_BAR_SIZE: u32 = 32;

fn select(pci: &mut PciConfig, device: u8, function: u8, reg: u8) {
    let addr = PciConfig::make_address(0, device, function, reg, true);
    pci.port_write(PCI_CONFIG_ADDRESS, 4, addr);
}

fn read_cfg(pci: &mut PciConfig, device: u8, function: u8, reg: u8) -> u32 {
    select(pci, device, function, reg);
    pci.port_read(PCI_CONFIG_DATA, 4)
}

fn write_cfg(pci: &mut PciConfig, device: u8, function: u8, reg: u8, value: u32) {
    select(pci, device, function, reg);
    pci.port_write(PCI_CONFIG_DATA, 4, value);
}

/// The sizing arithmetic §6.2.5.1 prescribes, for an I/O register.
fn io_region_size(readback: u32) -> u32 {
    (!(readback & !0x3u32)).wrapping_add(1)
}

/// Spec: §6.2.5.1 — write all ones, read back, clear the encoding bits, invert,
/// increment. Both implemented registers must report their real region size,
/// and the bits below that size must be hardwired so the region is naturally
/// aligned.
#[test]
fn implemented_io_bars_report_their_size_through_the_all_ones_protocol() {
    for (device, function, offset, size) in [
        (1u8, 1u8, PCI_PIIX_IDE_BMIBA_OFFSET, BMIDE_BAR_SIZE),
        (1, 2, PCI_PIIX_USB_BAR0_OFFSET, UHCI_BAR_SIZE),
    ] {
        let mut pci = PciConfig::new();
        let original = read_cfg(&mut pci, device, function, offset);

        write_cfg(&mut pci, device, function, offset, 0xFFFF_FFFF);
        let readback = read_cfg(&mut pci, device, function, offset);

        assert_eq!(
            readback,
            !(size - 1) | PCI_BAR_IO_SPACE,
            "{device:02x}:{function} BAR at {offset:#04x}: writable bits above the region size, \
             bit0 hardwired to I/O space, bit1 reserved zero"
        );
        assert_eq!(
            io_region_size(readback),
            size,
            "{device:02x}:{function} BAR at {offset:#04x} must size to {size} bytes"
        );

        // Restoring the saved value is the other half of the protocol.
        write_cfg(&mut pci, device, function, offset, original);
        assert_eq!(read_cfg(&mut pci, device, function, offset), original);
        assert_eq!(
            original, PCI_BAR_IO_SPACE,
            "the read-only I/O-space bit must already read back before sizing, or firmware \
             classifies the register as memory"
        );
    }
}

/// Spec: §6.2.5.1 — "the other bits hardwired to 0". A BAR reads back the
/// programmed base with every bit below the region size cleared, so the region
/// is always naturally aligned no matter what firmware wrote.
#[test]
fn bar_reads_back_the_programmed_base_masked_by_the_size() {
    let mut pci = PciConfig::new();

    // 16-byte BMIDE register: bits 3:1 are hardwired zero, bit 0 hardwired one.
    write_cfg(&mut pci, 1, 1, PCI_PIIX_IDE_BMIBA_OFFSET, 0x0000_C00E);
    assert_eq!(
        read_cfg(&mut pci, 1, 1, PCI_PIIX_IDE_BMIBA_OFFSET),
        0x0000_C001
    );

    // 32-byte UHCI register: bits 4:1 hardwired zero, so 0xC010 aligns down.
    write_cfg(&mut pci, 1, 2, PCI_PIIX_USB_BAR0_OFFSET, 0x0000_C010);
    assert_eq!(
        read_cfg(&mut pci, 1, 2, PCI_PIIX_USB_BAR0_OFFSET),
        0x0000_C001
    );

    // Bit 0 is read-only: clearing it does not turn an I/O register into a
    // memory one.
    write_cfg(&mut pci, 1, 2, PCI_PIIX_USB_BAR0_OFFSET, 0x0000_C000);
    assert_eq!(
        read_cfg(&mut pci, 1, 2, PCI_PIIX_USB_BAR0_OFFSET) & PCI_BAR_IO_SPACE,
        PCI_BAR_IO_SPACE
    );
}

/// The same masking applies to byte and word accesses, which reach one BAR
/// through the CONFIG_DATA byte lanes rather than a single dword write.
#[test]
fn narrow_writes_to_a_bar_keep_the_type_bits_and_alignment() {
    let mut pci = PciConfig::new();

    select(&mut pci, 1, 1, PCI_PIIX_IDE_BMIBA_OFFSET);
    pci.port_write(PCI_CONFIG_DATA, 1, 0xFF); // low byte only
    assert_eq!(
        read_cfg(&mut pci, 1, 1, PCI_PIIX_IDE_BMIBA_OFFSET),
        0x0000_00F1,
        "low byte: bits 7:4 stored, bits 3:1 zero, bit 0 one"
    );

    select(&mut pci, 1, 1, PCI_PIIX_IDE_BMIBA_OFFSET);
    pci.port_write(PCI_CONFIG_DATA, 2, 0xC00E);
    assert_eq!(
        read_cfg(&mut pci, 1, 1, PCI_PIIX_IDE_BMIBA_OFFSET),
        0x0000_C001
    );
}

/// Spec: §6.2.5.1 — a device reports the address space it needs by implementing
/// only the registers it decodes; an unimplemented Base Address Register reads
/// as all zeros. Storing what firmware wrote during sizing would report a
/// region this machine does not decode, which is the failure this test exists
/// to catch.
#[test]
fn unimplemented_bars_are_read_only_zero_on_every_function() {
    let implemented = [
        (1u8, 1u8, PCI_PIIX_IDE_BMIBA_OFFSET),
        (1, 2, PCI_PIIX_USB_BAR0_OFFSET),
    ];

    let mut pci = PciConfig::new();
    for (device, function) in FUNCTIONS {
        for offset in BAR_OFFSETS {
            if implemented.contains(&(device, function, offset)) {
                continue;
            }
            assert_eq!(
                read_cfg(&mut pci, device, function, offset),
                0,
                "{device:02x}:{function} BAR {offset:#04x} must read zero at reset"
            );
            write_cfg(&mut pci, device, function, offset, 0xFFFF_FFFF);
            assert_eq!(
                read_cfg(&mut pci, device, function, offset),
                0,
                "{device:02x}:{function} BAR {offset:#04x} must stay zero after an all-ones \
                 sizing write"
            );
        }
    }
}

/// Spec: §6.2.5.1 "Expansion ROM Base Address Register" — no function in this
/// machine has an expansion ROM, so the register reads zero everywhere and a
/// sizing write leaves it there.
#[test]
fn expansion_rom_bar_is_read_only_zero_on_every_function() {
    let mut pci = PciConfig::new();
    for (device, function) in FUNCTIONS {
        assert_eq!(read_cfg(&mut pci, device, function, ROM_BAR_OFFSET), 0);
        write_cfg(&mut pci, device, function, ROM_BAR_OFFSET, 0xFFFF_FFFF);
        assert_eq!(
            read_cfg(&mut pci, device, function, ROM_BAR_OFFSET),
            0,
            "{device:02x}:{function} expansion ROM BAR must stay zero"
        );
    }
}

/// A sizing pass writes all ones into a BAR. Until firmware writes a real base
/// the register must not decode anything: an all-ones base is outside the
/// 16-bit x86 I/O space, and reset returns both registers to zero.
#[test]
fn sizing_write_does_not_produce_a_decoding_io_window() {
    let mut pci = PciConfig::new();
    // Enable I/O space on both functions first, so only the base is in question.
    write_cfg(&mut pci, 1, 1, 0x04, 0x0000_0005);
    write_cfg(&mut pci, 1, 2, 0x04, 0x0000_0007);

    write_cfg(&mut pci, 1, 1, PCI_PIIX_IDE_BMIBA_OFFSET, 0xFFFF_FFFF);
    write_cfg(&mut pci, 1, 2, PCI_PIIX_USB_BAR0_OFFSET, 0xFFFF_FFFF);

    assert_eq!(pci.bmide_io_base(), None, "all-ones base is not I/O space");
    assert_eq!(pci.uhci_io_base(), None, "all-ones base is not I/O space");
    for port in [0x0000u16, 0x00F0, 0xFFF0, 0xFFE0] {
        assert!(!pci.bmide_owns_port(port));
        assert!(!pci.uhci_owns_port(port));
    }

    pci.reset();
    assert_eq!(
        read_cfg(&mut pci, 1, 1, PCI_PIIX_IDE_BMIBA_OFFSET),
        PCI_BAR_IO_SPACE
    );
    assert_eq!(
        read_cfg(&mut pci, 1, 2, PCI_PIIX_USB_BAR0_OFFSET),
        PCI_BAR_IO_SPACE
    );
    assert_eq!(pci.bmide_io_base(), None, "I/O Space Enable is clear again");
    assert_eq!(pci.uhci_io_base(), None, "I/O Space Enable is clear again");
}

/// Sizing one function's BAR must not disturb another's — the registers are
/// per-function state, and a sizing pass walks every function in turn.
#[test]
fn sizing_one_function_leaves_the_others_alone() {
    let mut pci = PciConfig::new();
    write_cfg(&mut pci, 1, 1, PCI_PIIX_IDE_BMIBA_OFFSET, 0x0000_C000);
    write_cfg(&mut pci, 1, 2, PCI_PIIX_USB_BAR0_OFFSET, 0x0000_D000);

    write_cfg(&mut pci, 1, 1, PCI_PIIX_IDE_BMIBA_OFFSET, 0xFFFF_FFFF);

    assert_eq!(
        read_cfg(&mut pci, 1, 2, PCI_PIIX_USB_BAR0_OFFSET),
        0x0000_D001
    );
    assert_eq!(read_cfg(&mut pci, 0, 0, PCI_PIIX_IDE_BMIBA_OFFSET), 0);
}
