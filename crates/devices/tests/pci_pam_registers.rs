//! Guest-visible behavior of the i440FX PMC Programmable Attribute Map
//! registers (PAM0–PAM6) at host-bridge `00:00.0` config offsets `0x59`–`0x5F`.
//!
//! Spec: Intel 440FX PCIset 82441FX (PMC) datasheet, §3.2.18 "PAM —
//! Programmable Attribute Map Registers (PAM[6:0])" — address offset `59h`
//! (PAM0) through `5Fh` (PAM6), default value `00h`, attribute Read/Write;
//! Table 2 "Attribute Bit Assignment" (bits [7,6,3,2] Reserved, bits [5,1] WE,
//! bits [4,0] RE); Table 3 "PAM Registers and Associated Memory Segments".
//!
//! Reserved-bit treatment follows the PCI Local Bus Specification rule that
//! reserved configuration fields read as zero.
//!
//! These are integration tests, so they may only use the crate's re-exported
//! surface. The PAM offsets and masks are therefore repeated here as local
//! literals with their spec citation until `devices/src/lib.rs` re-exports the
//! `PCI_PMC_PAM_*` constants.

use devices::{PciConfig, PortDevice, PCI_CONFIG_ADDRESS, PCI_CONFIG_DATA};

/// Spec: 440FX §3.2.18 — PAM0 lives at PMC config offset `59h`.
const PAM0_OFFSET: u8 = 0x59;
/// Spec: 440FX §3.2.18 — seven registers, PAM0 (`59h`) … PAM6 (`5Fh`).
const PAM_COUNT: usize = 7;
/// Spec: 440FX §3.2.18 — "Default Value: 00h".
const PAM_DEFAULT: u8 = 0x00;
/// Spec: 440FX Table 2 — RE occupies bits [4, 0] and WE bits [5, 1] of a
/// register, i.e. both 4-bit attribute fields expose only RE|WE.
const PAM_WRITABLE_MASK: u8 = 0x33;
/// Spec: 440FX Table 3 — `PAM0[3:0]` is Reserved, so only the high nibble of
/// PAM0 (the `0F0000-0FFFFFh` BIOS Area) is writable.
const PAM0_WRITABLE_MASK: u8 = 0x30;

fn cfg_read_u8(pci: &mut PciConfig, offset: u8) -> u8 {
    let addr = PciConfig::make_address(0, 0, 0, offset, true);
    pci.port_write(PCI_CONFIG_ADDRESS, 4, addr);
    pci.port_read(PCI_CONFIG_DATA + u16::from(offset & 0x03), 1) as u8
}

fn cfg_write_u8(pci: &mut PciConfig, offset: u8, value: u8) {
    let addr = PciConfig::make_address(0, 0, 0, offset, true);
    pci.port_write(PCI_CONFIG_ADDRESS, 4, addr);
    pci.port_write(
        PCI_CONFIG_DATA + u16::from(offset & 0x03),
        1,
        u32::from(value),
    );
}

/// Spec: 440FX §3.2.18 — "Default Value: 00h" for PAM0 through PAM6. With
/// every attribute field zero, Table 2 encoding `00` means "Disabled. DRAM is
/// disabled and all accesses are directed to PCI", which is how the platform
/// comes out of reset running from the BIOS ROM.
#[test]
fn pam_registers_default_to_zero_at_reset() {
    let mut pci = PciConfig::new();
    for i in 0..PAM_COUNT {
        assert_eq!(
            cfg_read_u8(&mut pci, PAM0_OFFSET + i as u8),
            PAM_DEFAULT,
            "PAM{i} must reset to 00h"
        );
    }
}

/// Spec: 440FX §3.2.18 — "Attribute: Read/Write". Each defined RE/WE bit is
/// stored and read back through configuration mechanism #1.
#[test]
fn pam_registers_store_and_read_back_defined_bits() {
    let mut pci = PciConfig::new();

    // Read/Write both regions of PAM1–PAM6 (Table 2 encoding `11`).
    for i in 1..PAM_COUNT {
        cfg_write_u8(&mut pci, PAM0_OFFSET + i as u8, PAM_WRITABLE_MASK);
        assert_eq!(
            cfg_read_u8(&mut pci, PAM0_OFFSET + i as u8),
            PAM_WRITABLE_MASK
        );
    }
    // PAM0 high nibble only: BIOS area Read/Write.
    cfg_write_u8(&mut pci, PAM0_OFFSET, PAM0_WRITABLE_MASK);
    assert_eq!(cfg_read_u8(&mut pci, PAM0_OFFSET), PAM0_WRITABLE_MASK);

    // Read Only (RE=1, WE=0) — the shadowed, write-protected BIOS state.
    cfg_write_u8(&mut pci, PAM0_OFFSET, 0x10);
    assert_eq!(cfg_read_u8(&mut pci, PAM0_OFFSET), 0x10);
    // Write Only (RE=0, WE=1) — the state used while copying the ROM into DRAM.
    cfg_write_u8(&mut pci, PAM0_OFFSET, 0x20);
    assert_eq!(cfg_read_u8(&mut pci, PAM0_OFFSET), 0x20);
}

/// Spec: 440FX Table 2 — bits [7, 6, 3, 2] are Reserved; Table 3 — `PAM0[3:0]`
/// is Reserved. PCI Local Bus Specification: reserved configuration fields read
/// as zero, so writing all ones leaves only RE/WE set.
#[test]
fn pam_reserved_bits_read_back_as_zero() {
    let mut pci = PciConfig::new();

    cfg_write_u8(&mut pci, PAM0_OFFSET, 0xFF);
    assert_eq!(
        cfg_read_u8(&mut pci, PAM0_OFFSET),
        PAM0_WRITABLE_MASK,
        "PAM0[3:0] and the reserved nibble bits must read back zero"
    );

    for i in 1..PAM_COUNT {
        cfg_write_u8(&mut pci, PAM0_OFFSET + i as u8, 0xFF);
        assert_eq!(
            cfg_read_u8(&mut pci, PAM0_OFFSET + i as u8),
            PAM_WRITABLE_MASK,
            "PAM{i} reserved bits [7,6,3,2] must read back zero"
        );
    }
}

/// A dword write covering the PAM block must land the same masked values as
/// byte writes; SeaBIOS-class firmware programs shadowing a register at a time,
/// but nothing prevents a wider access.
#[test]
fn pam_dword_write_applies_the_same_reserved_mask() {
    let mut pci = PciConfig::new();
    // 0x5C–0x5F = PAM3, PAM4, PAM5, PAM6.
    let addr = PciConfig::make_address(0, 0, 0, 0x5C, true);
    pci.port_write(PCI_CONFIG_ADDRESS, 4, addr);
    pci.port_write(PCI_CONFIG_DATA, 4, 0xFFFF_FFFF);
    for offset in 0x5Cu8..=0x5F {
        assert_eq!(cfg_read_u8(&mut pci, offset), PAM_WRITABLE_MASK);
    }
}

/// Spec: 440FX §3.2.18 — the PAM block resets to `00h`; `PciConfig::reset`
/// returns the whole PMC register file to its power-on state.
#[test]
fn pam_registers_clear_on_reset() {
    let mut pci = PciConfig::new();
    for i in 0..PAM_COUNT {
        cfg_write_u8(&mut pci, PAM0_OFFSET + i as u8, 0x33);
    }
    pci.reset();
    for i in 0..PAM_COUNT {
        assert_eq!(cfg_read_u8(&mut pci, PAM0_OFFSET + i as u8), PAM_DEFAULT);
    }
}

/// The PAM block belongs to the host bridge only; the PIIX functions keep
/// whatever those offsets mean for them and are not aliased.
#[test]
fn pam_writes_do_not_leak_into_piix_functions() {
    let mut pci = PciConfig::new();
    cfg_write_u8(&mut pci, PAM0_OFFSET + 1, 0x33);

    for function in 0u8..=3 {
        let addr = PciConfig::make_address(0, 1, function, PAM0_OFFSET + 1, true);
        pci.port_write(PCI_CONFIG_ADDRESS, 4, addr);
        let byte = pci.port_read(PCI_CONFIG_DATA + u16::from((PAM0_OFFSET + 1) & 0x03), 1) as u8;
        assert_eq!(
            byte, 0x00,
            "00:01.{function} must not alias host-bridge PAM"
        );
    }
}

/// The decoded host accessor is what the machine layer wires to its physical
/// memory attribute model. Spec: 440FX Table 3 — thirteen segments in
/// ascending address order, the twelve 16 KiB expansion/extension regions from
/// `0C0000h` through `0EFFFFh` followed by the 64 KiB BIOS Area at
/// `0F0000-0FFFFFh`.
#[test]
fn decoded_pam_regions_cover_table_3_in_ascending_address_order() {
    let pci = PciConfig::new();
    let regions = pci.pam_regions();
    assert_eq!(regions.len(), 13);

    for (i, region) in regions.iter().take(12).enumerate() {
        assert_eq!(region.start, 0x000C_0000 + (i as u32) * 0x4000);
        assert_eq!(region.end, region.start + 0x3FFF);
    }
    assert_eq!(regions[12].start, 0x000F_0000);
    assert_eq!(regions[12].end, 0x000F_FFFF);

    // Reset default 00h: Table 2 encoding `00` — disabled, everything to PCI.
    for region in regions.iter() {
        assert!(!region.read_from_ram);
        assert!(!region.write_to_ram);
    }
}

/// Spec: 440FX §3.2.18 — "RE Read Enable. When RE=1, CPU read accesses to the
/// corresponding memory segment are claimed by the PMC and directed to main
/// memory … when RE=0, the CPU read accesses are directed to PCI." WE behaves
/// the same way for writes. Table 3 assigns the BIOS Area to `PAM0[7:4]`.
#[test]
fn decoded_bios_area_follows_pam0_high_nibble() {
    let mut pci = PciConfig::new();

    // Write Only: the datasheet's documented shadowing step — reads still come
    // from the expansion bus while the copy is written into DRAM.
    cfg_write_u8(&mut pci, PAM0_OFFSET, 0x20);
    let bios = pci.pam_regions()[12];
    assert!(!bios.read_from_ram);
    assert!(bios.write_to_ram);

    // Read Only: "After the BIOS is shadowed, the attributes for that memory
    // area are set to read only so that all writes are forwarded to the
    // expansion bus."
    cfg_write_u8(&mut pci, PAM0_OFFSET, 0x10);
    let bios = pci.pam_regions()[12];
    assert!(bios.read_from_ram);
    assert!(!bios.write_to_ram);
}

/// Table 3 — each PAM register controls two regions: the low nibble the lower
/// segment, the high nibble the upper segment.
#[test]
fn decoded_low_and_high_nibbles_select_adjacent_segments() {
    let mut pci = PciConfig::new();

    // PAM1 (`5Ah`): low nibble `0C0000-0C3FFFh`, high nibble `0C4000-0C7FFFh`.
    // Low = Read Only, high = Write Only.
    cfg_write_u8(&mut pci, 0x5A, 0x21);
    let regions = pci.pam_regions();

    assert_eq!(regions[0].start, 0x000C_0000);
    assert!(regions[0].read_from_ram);
    assert!(!regions[0].write_to_ram);

    assert_eq!(regions[1].start, 0x000C_4000);
    assert!(!regions[1].read_from_ram);
    assert!(regions[1].write_to_ram);
}

/// Address lookup for the machine layer: an address inside a PAM segment
/// resolves to that segment; the video buffer area `A0000-BFFFFh`, which the
/// datasheet states "is not controlled by attribute bits", resolves to nothing.
#[test]
fn decoded_lookup_by_address_skips_unmapped_ranges() {
    let mut pci = PciConfig::new();
    cfg_write_u8(&mut pci, PAM0_OFFSET, 0x30);

    let hit = pci
        .pam_region_for_addr(0x000F_4000)
        .expect("BIOS area is PAM-controlled");
    assert_eq!(hit.start, 0x000F_0000);
    assert!(hit.read_from_ram);
    assert!(hit.write_to_ram);

    assert!(pci.pam_region_for_addr(0x000B_8000).is_none());
    assert!(pci.pam_region_for_addr(0x0009_FFFF).is_none());
    assert!(pci.pam_region_for_addr(0x0010_0000).is_none());
}
