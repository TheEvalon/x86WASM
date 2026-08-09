//! The BIOS-shadowing seam: guest PCI configuration writes to the i440FX PMC
//! host bridge re-attribute physical memory.
//!
//! Two halves of this exist in different crates. `devices::PciConfig` owns the
//! PMC configuration bytes at `00:00.0` offsets `0x59`–`0x5F` and has no memory
//! to remap; `machine_pc::PhysMem` owns the region attributes and has no
//! configuration space. Neither can shadow anything alone, and they describe
//! the same thirteen segments in different shapes — a decoded `PamRegion`
//! array on one side, raw register bytes plus a region index on the other.
//! These tests check that they agree and that a guest can drive the whole
//! sequence through Configuration Mechanism #1.
//!
//! Spec: Intel 440FX PCIset 82441FX (PMC) datasheet, order 290549-001, §3.2.18
//! "PAM — Programmable Attribute Map Registers (PAM[6:0])", Table 2 "Attribute
//! Bit Assignment" and Table 3 "PAM Registers and Associated Memory Segments";
//! PCI Local Bus Specification Configuration Mechanism #1.

use devices::{
    PciConfig, PortDevice, PCI_PMC_PAM0_OFFSET, PCI_PMC_PAM_COUNT, PCI_PMC_PAM_RE,
    PCI_PMC_PAM_REGION_COUNT, PCI_PMC_PAM_WE,
};
use machine_pc::{
    Machine, PamRead, PamWrite, PhysMem, PAM_BIOS_REGION, PAM_FIELD_RE, PAM_FIELD_WE, PAM_REGIONS,
    PAM_REGION_COUNT, PAM_REGISTER_FIRST, PAM_REGISTER_LAST,
};

/// The two halves describe the same Table 3 segments in the same order.
///
/// Both sides claim "ascending address order with index 12 as the BIOS area".
/// This checks it rather than trusting it, field by field: the segment bounds,
/// the constants that name the register block, and — the part that would
/// actually silently corrupt shadowing if it drifted — which region index each
/// register nibble owns.
#[test]
fn pci_and_phys_mem_agree_on_pam_region_ordering() {
    assert_eq!(PAM_REGISTER_FIRST, PCI_PMC_PAM0_OFFSET);
    assert_eq!(
        usize::from(PAM_REGISTER_LAST - PAM_REGISTER_FIRST) + 1,
        PCI_PMC_PAM_COUNT
    );
    assert_eq!(PAM_REGION_COUNT, PCI_PMC_PAM_REGION_COUNT);
    assert_eq!(PAM_FIELD_RE, PCI_PMC_PAM_RE);
    assert_eq!(PAM_FIELD_WE, PCI_PMC_PAM_WE);

    let regions = PciConfig::new().pam_regions();
    for (index, (base, len)) in PAM_REGIONS.iter().enumerate() {
        assert_eq!(
            u64::from(regions[index].start),
            *base,
            "region {index} start"
        );
        assert_eq!(
            u64::from(regions[index].end),
            base + len - 1,
            "region {index} end"
        );
        assert_eq!(
            PhysMem::pam_region_index(*base),
            Some(index),
            "region {index} lookup"
        );
    }

    // Index 12 is the 64 KiB BIOS Area on both sides.
    assert_eq!(PAM_BIOS_REGION, PAM_REGION_COUNT - 1);
    assert_eq!(regions[PAM_BIOS_REGION].start, 0x000F_0000);
    assert_eq!(regions[PAM_BIOS_REGION].end, 0x000F_FFFF);

    // Each register nibble flips the same region index on both sides.
    for offset in PAM_REGISTER_FIRST..=PAM_REGISTER_LAST {
        for high in [false, true] {
            let mut pci = PciConfig::new();
            let byte = if high {
                PCI_PMC_PAM_RE << 4
            } else {
                PCI_PMC_PAM_RE
            };
            pci.set_pam_register(usize::from(offset - PAM_REGISTER_FIRST), byte);
            let flipped: Vec<usize> = pci
                .pam_regions()
                .iter()
                .enumerate()
                .filter(|(_, r)| r.read_from_ram)
                .map(|(i, _)| i)
                .collect();
            let expected = PhysMem::pam_region_for_register(offset, high);
            match expected {
                // `PAM0[3:0]` is Reserved: no segment, and nothing may flip.
                None => assert!(flipped.is_empty(), "{offset:#04X} high={high} {flipped:?}"),
                Some(region) => assert_eq!(flipped, vec![region], "{offset:#04X} high={high}"),
            }
        }
    }
}

/// Program CONFIG_ADDRESS for host-bridge `00:00.0` register `reg`.
///
/// Written with four 8-bit `OUT DX, AL` stores because this build's primary
/// opcode map has no `ED`/`EF` accumulator port I/O — only the byte forms
/// `E4`/`E6`/`EC`/`EE` exist.
///
/// Real hardware does not accept this. PCI Local Bus Specification Revision 3.0
/// §3.2.2.3.2 says non-dword accesses to CONFIG_ADDRESS "have no effect on
/// CONFIG_ADDRESS and are executed as normal I/O transactions on the PCI bus",
/// which is the emulator's default. The test therefore arms the documented
/// compatibility policy (`set_config_address_byte_lane_compat`) explicitly, and
/// this whole helper can be replaced with a single `OUT DX, EAX` once `EF`
/// decodes.
#[rustfmt::skip]
fn config_address_for_host_bridge(reg: u8) -> Vec<u8> {
    let dword = reg & !0x03;
    vec![
        0xBA, 0xF8, 0x0C,       // MOV DX, 0x0CF8
        0xB0, dword,            // MOV AL, reg & ~3
        0xEE,                   // OUT DX, AL
        0xBA, 0xF9, 0x0C,       // MOV DX, 0x0CF9
        0xB0, 0x00,             // MOV AL, 0 (bus 0)
        0xEE,                   // OUT DX, AL
        0xBA, 0xFA, 0x0C,       // MOV DX, 0x0CFA
        0xB0, 0x00,             // MOV AL, 0 (device 0, function 0)
        0xEE,                   // OUT DX, AL
        0xBA, 0xFB, 0x0C,       // MOV DX, 0x0CFB
        0xB0, 0x80,             // MOV AL, enable (bit 31)
        0xEE,                   // OUT DX, AL
    ]
}

/// Store `value` into CONFIG_DATA at the byte lane register `reg` selects.
#[rustfmt::skip]
fn config_data_byte(reg: u8, value: u8) -> Vec<u8> {
    let port = 0x0CFCu16 + u16::from(reg & 0x03);
    let [lo, hi] = port.to_le_bytes();
    vec![
        0xBA, lo, hi,           // MOV DX, CONFIG_DATA + lane
        0xB0, value,            // MOV AL, value
        0xEE,                   // OUT DX, AL
    ]
}

/// PAM0 (`0x59`) — high nibble attributes the `0xF0000` BIOS Area.
const PAM0: u8 = 0x59;

/// 64 KiB BIOS image whose reset vector far-jumps to `F000:0000`.
///
/// Spec: Intel SDM Vol. 3 §9.1.4 — the first fetch is `0xFFFFFFF0` with
/// `CS.base = 0xFFFF0000`; the far `JMP ptr16:16` moves execution to the
/// below-1 MiB alias, which is the window PAM attributes.
fn bios_image_64k(code: &[u8]) -> Vec<u8> {
    let mut rom = vec![0xF4u8; 64 * 1024];
    rom[..code.len()].copy_from_slice(code);
    rom[0xFFF0..0xFFF5].copy_from_slice(&[0xEA, 0x00, 0x00, 0x00, 0xF0]);
    rom
}

/// The whole `make_bios_writable()` shape, driven entirely by the guest.
///
/// Before this seam existed, the `OUT`s below changed a register file that
/// nothing read: PAM programming remapped no memory and the shadow copy went
/// nowhere. The test proves the guest's own configuration writes move memory,
/// by requiring the final instruction stream to come from the shadow copy.
#[test]
fn guest_programs_pam_through_mechanism_1_then_shadows_and_locks_the_bios() {
    #[rustfmt::skip]
    let copy_and_halt: &[u8] = &[
        0xFC,                   // CLD
        0xB8, 0x00, 0xF0,       // MOV AX, 0xF000
        0x8E, 0xD8,             // MOV DS, AX
        0x8E, 0xC0,             // MOV ES, AX
        0x31, 0xF6,             // XOR SI, SI
        0x31, 0xFF,             // XOR DI, DI
        0xB9, 0x00, 0x80,       // MOV CX, 0x8000
        0xF3, 0xA4,             // REP MOVSB
        0xB9, 0x00, 0x80,       // MOV CX, 0x8000
        0xF3, 0xA4,             // REP MOVSB
        0xF4,                   // HLT — the host patches the shadow copy here
    ];
    #[rustfmt::skip]
    let report_tag: &[u8] = &[
        0xB0, b'R',             // MOV AL, 'R' — shadow copy is patched to 'S'
        0xBA, 0x02, 0x04,       // MOV DX, 0x0402 (debug console)
        0xEE,                   // OUT DX, AL
        0xF4,                   // HLT
    ];

    let mut code = Vec::new();
    code.extend_from_slice(&config_address_for_host_bridge(PAM0));
    // Read ROM, write DRAM: the attribute pair the copy needs.
    code.extend_from_slice(&config_data_byte(PAM0, PAM_FIELD_WE << 4));
    code.extend_from_slice(copy_and_halt);
    // Lock: read DRAM, writes dropped.
    code.extend_from_slice(&config_data_byte(PAM0, PAM_FIELD_RE << 4));
    code.extend_from_slice(report_tag);

    let tag_immediate = code
        .windows(2)
        .position(|w| w == [0xB0, b'R'])
        .expect("tag instruction present")
        + 1;

    let rom = bios_image_64k(&code);
    let mut m = Machine::with_bios_rom(1024 * 1024, &rom).expect("map BIOS image");
    // See `config_address_for_host_bridge`: byte-lane CONFIG_ADDRESS programming
    // is a documented model choice this guest needs until `EF` decodes.
    m.pci.set_config_address_byte_lane_compat(true);
    m.reset();

    // Reset state: PAM0 is 0x00, so the BIOS area reads ROM and drops writes.
    assert_eq!(m.pci.pam_register(0), Some(0x00));
    assert_eq!(
        m.pam_attributes(PAM_BIOS_REGION).map(|a| (a.read, a.write)),
        Some((PamRead::Rom, PamWrite::Ignored))
    );

    let steps = m.run(200_000).expect("guest runs to the mid-sequence HLT");
    assert!(m.cpu.halted, "guest halted after the copy ({steps} steps)");

    // The guest's configuration write reached physical memory.
    assert_eq!(m.pci.pam_register(0), Some(PAM_FIELD_WE << 4));
    assert_eq!(
        m.pam_attributes(PAM_BIOS_REGION).map(|a| (a.read, a.write)),
        Some((PamRead::Rom, PamWrite::ShadowRam)),
        "a guest PAM write must re-attribute the BIOS region"
    );

    // Patch the shadow copy while writes still reach DRAM, so the fetch after
    // the lock can only produce 'S' if it came from shadow rather than ROM.
    m.mem
        .write_u8(0x000F_0000 + tag_immediate as u64, b'S')
        .expect("write reaches shadow DRAM while WE is set");

    m.cpu.halted = false;
    m.run(64).expect("guest locks the region and reports");

    assert!(m.cpu.halted);
    assert_eq!(m.pci.pam_register(0), Some(PAM_FIELD_RE << 4));
    assert_eq!(
        m.pam_attributes(PAM_BIOS_REGION).map(|a| (a.read, a.write)),
        Some((PamRead::ShadowRam, PamWrite::Ignored))
    );
    assert_eq!(
        m.debug_text(),
        "S",
        "the instruction stream after the lock came from the shadow copy"
    );

    // The top-of-4 GiB window is outside PAM and still holds the ROM image.
    assert_eq!(m.mem.read_u8(0xFFFF_0000 + tag_immediate as u64), Ok(b'R'));
}

/// A configuration write to a neighbouring host-bridge register is not a PAM
/// write, and a PAM-offset write to a different function is not either.
///
/// Spec: PCI Mechanism #1 — CONFIG_ADDRESS selects bus/device/function as well
/// as the register, so the overlap test has to check all of them.
#[test]
fn only_host_bridge_pam_offsets_re_attribute_memory() {
    let mut pci = PciConfig::new();

    // Host bridge 00:00.0, register 0x58 dword — lane 1 is PAM0 (0x59).
    pci.port_write(
        0x0CF8,
        4,
        PciConfig::make_address(0, 0, 0, PAM0, /* enable */ true),
    );
    assert!(pci.pam_config_write_overlaps(0x0CFD, 1));
    // Lane 0 of the same dword is 0x58 (DRAMT), which is not PAM.
    assert!(!pci.pam_config_write_overlaps(0x0CFC, 1));
    // A dword access to 0x58 does reach 0x59-0x5B.
    assert!(pci.pam_config_write_overlaps(0x0CFC, 4));

    // The same register offset on the PIIX ISA bridge is unrelated.
    pci.port_write(0x0CF8, 4, PciConfig::make_address(0, 1, 0, PAM0, true));
    assert!(!pci.pam_config_write_overlaps(0x0CFD, 1));

    // Disabled CONFIG_ADDRESS decodes nothing.
    pci.port_write(0x0CF8, 4, PciConfig::make_address(0, 0, 0, PAM0, false));
    assert!(!pci.pam_config_write_overlaps(0x0CFD, 1));
}
