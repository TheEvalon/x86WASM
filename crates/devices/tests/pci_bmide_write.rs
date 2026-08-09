//! Device-level tests for the bounded PIIX BMIDE **write-direction** PRD walk
//! on the primary channel (guest memory → device buffer).
//!
//! Spec: Intel Programming Interface for Bus Master IDE Controller Rev 1.0
//! (May 1994) §§1.1–1.2 — Physical Region Descriptor format (physical base
//! \[31:1\], byte count \[15:1\] with 0 = 64 KiB, EOT in bit 7 of the last
//! byte) and the Bus Master IDE Command / Status registers; Intel 82371SB
//! §2.7 — BMICOM bit0 SSBM, BMICOM bit3 RWCON (`1` selects the write
//! direction), BMISTA bit0 Active and bit1 DMA Error, BMIDTP descriptor table
//! pointer.
//!
//! This remains a PRD walker: there is no ATA command engine behind it.

use devices::{
    BmidePrdError, PciConfig, PortDevice, PCI_COMMAND_BUS_MASTER, PCI_COMMAND_IO,
    PCI_COMMAND_OFFSET, PCI_CONFIG_ADDRESS, PCI_CONFIG_DATA, PCI_PIIX_IDE_BMIBA_OFFSET,
    PCI_PIIX_IDE_BMICOM_PRIMARY, PCI_PIIX_IDE_BMICOM_RWCON, PCI_PIIX_IDE_BMICOM_SSBM,
    PCI_PIIX_IDE_BMIDTP_PRIMARY, PCI_PIIX_IDE_BMISTA_ACTIVE, PCI_PIIX_IDE_BMISTA_PRIMARY,
    PCI_PIIX_IDE_PRD_BYTE_COUNT_64K, PCI_PIIX_IDE_PRD_ENTRY_SIZE, PCI_PIIX_IDE_PRD_EOT,
};

/// BMISTA bit1 DMA Error (Intel 82371SB §2.7.2); not re-exported by `devices`.
const BMISTA_ERROR: u8 = 1 << 1;
const BMIBA: u16 = 0xF000;

fn program_bmide(pci: &mut PciConfig, bus_master: bool) {
    pci.port_write(
        PCI_CONFIG_ADDRESS,
        4,
        PciConfig::make_address(0, 1, 1, PCI_PIIX_IDE_BMIBA_OFFSET, true),
    );
    pci.port_write(PCI_CONFIG_DATA, 4, u32::from(BMIBA));
    pci.port_write(
        PCI_CONFIG_ADDRESS,
        4,
        PciConfig::make_address(0, 1, 1, PCI_COMMAND_OFFSET, true),
    );
    let mut command = u32::from(PCI_COMMAND_IO);
    if bus_master {
        command |= u32::from(PCI_COMMAND_BUS_MASTER);
    }
    pci.port_write(PCI_CONFIG_DATA, 2, command);
}

fn set_prdt(pci: &mut PciConfig, prdt: u32) {
    pci.port_write(BMIBA + u16::from(PCI_PIIX_IDE_BMIDTP_PRIMARY), 4, prdt);
}

fn store_prd(mem: &mut [u8], prdt: u32, index: usize, phys: u32, count: u16, eot: bool) {
    let at = prdt as usize + index * PCI_PIIX_IDE_PRD_ENTRY_SIZE;
    let mut prd = [0u8; PCI_PIIX_IDE_PRD_ENTRY_SIZE];
    prd[0..4].copy_from_slice(&phys.to_le_bytes());
    prd[4..6].copy_from_slice(&count.to_le_bytes());
    if eot {
        prd[7] = PCI_PIIX_IDE_PRD_EOT;
    }
    mem[at..at + PCI_PIIX_IDE_PRD_ENTRY_SIZE].copy_from_slice(&prd);
}

fn bmicom(pci: &PciConfig) -> u8 {
    pci.bmide_io[PCI_PIIX_IDE_BMICOM_PRIMARY as usize]
}

fn bmista(pci: &PciConfig) -> u8 {
    pci.bmide_io[PCI_PIIX_IDE_BMISTA_PRIMARY as usize]
}

#[test]
fn write_direction_fills_the_device_buffer_and_sets_rwcon() {
    let mut pci = PciConfig::new();
    program_bmide(&mut pci, true);

    const PRDT: u32 = 0x0000_1000;
    const SRC: u32 = 0x0000_3000;
    set_prdt(&mut pci, PRDT);

    let mut mem = vec![0u8; 0x4000];
    store_prd(&mut mem, PRDT, 0, SRC, 6, true);
    mem[SRC as usize..SRC as usize + 6].copy_from_slice(&[1, 2, 3, 4, 5, 6]);

    let mut device = [0u8; 6];
    let xfer = pci
        .start_bm_write(&mut device, |phys| mem[phys as usize])
        .expect("bm write");

    assert_eq!(device, [1, 2, 3, 4, 5, 6]);
    assert_eq!(xfer.entry.phys_addr, SRC);
    assert_eq!(xfer.entry.byte_count, 6);
    assert!(xfer.entry.eot);
    assert_eq!(xfer.entries_walked, 1);
    assert_eq!(xfer.bytes_copied, 6);

    // RWCON stays latched at the write direction; SSBM and Active clear.
    assert_eq!(
        bmicom(&pci) & PCI_PIIX_IDE_BMICOM_RWCON,
        PCI_PIIX_IDE_BMICOM_RWCON
    );
    assert_eq!(bmicom(&pci) & PCI_PIIX_IDE_BMICOM_SSBM, 0);
    assert_eq!(bmista(&pci) & PCI_PIIX_IDE_BMISTA_ACTIVE, 0);
    assert_eq!(bmista(&pci) & BMISTA_ERROR, 0);

    // Alias entry point behaves identically.
    let mut device2 = [0u8; 6];
    pci.run_prd_write_stub(&mut device2, |phys| mem[phys as usize])
        .expect("alias");
    assert_eq!(device2, [1, 2, 3, 4, 5, 6]);
}

#[test]
fn write_direction_concatenates_regions_and_stops_at_buffer_end() {
    let mut pci = PciConfig::new();
    program_bmide(&mut pci, true);

    const PRDT: u32 = 0x0000_1000;
    const SRC_A: u32 = 0x0000_2000;
    const SRC_B: u32 = 0x0000_2800;
    set_prdt(&mut pci, PRDT);

    let mut mem = vec![0xEEu8; 0x4000];
    store_prd(&mut mem, PRDT, 0, SRC_A, 4, false);
    store_prd(&mut mem, PRDT, 1, SRC_B, 8, true);
    mem[SRC_A as usize..SRC_A as usize + 4].copy_from_slice(&[0xA0, 0xA1, 0xA2, 0xA3]);
    mem[SRC_B as usize..SRC_B as usize + 8].copy_from_slice(&[0xB0; 8]);

    let mut device = [0u8; 6];
    let xfer = pci
        .start_bm_write(&mut device, |phys| mem[phys as usize])
        .expect("two-entry PRDT");

    assert_eq!(device, [0xA0, 0xA1, 0xA2, 0xA3, 0xB0, 0xB0]);
    assert_eq!(xfer.entries_walked, 2);
    assert_eq!(xfer.bytes_copied, 6);
    assert_eq!(xfer.entry.phys_addr, SRC_B);
}

/// Spec: Rev 1.0 §1.2 — a zero byte-count field means 64 KiB.
#[test]
fn write_direction_zero_count_consumes_64k_before_the_next_prd() {
    let mut pci = PciConfig::new();
    program_bmide(&mut pci, true);

    const PRDT: u32 = 0x0000_1000;
    const SRC_A: u32 = 0x0001_0000;
    const SRC_B: u32 = 0x0002_2000;
    set_prdt(&mut pci, PRDT);

    let mut mem = vec![0u8; 0x0002_3000];
    store_prd(&mut mem, PRDT, 0, SRC_A, 0, false);
    store_prd(&mut mem, PRDT, 1, SRC_B, 2, true);
    mem[SRC_A as usize] = 0x5A;
    mem[SRC_A as usize + PCI_PIIX_IDE_PRD_BYTE_COUNT_64K as usize - 1] = 0xA5;
    mem[SRC_B as usize] = 0x77;

    let mut device = vec![0u8; PCI_PIIX_IDE_PRD_BYTE_COUNT_64K as usize + 1];
    let xfer = pci
        .start_bm_write(&mut device, |phys| mem[phys as usize])
        .expect("64 KiB region");

    assert_eq!(device[0], 0x5A);
    assert_eq!(device[PCI_PIIX_IDE_PRD_BYTE_COUNT_64K as usize - 1], 0xA5);
    assert_eq!(device[PCI_PIIX_IDE_PRD_BYTE_COUNT_64K as usize], 0x77);
    assert_eq!(xfer.entries_walked, 2);
}

#[test]
fn write_direction_missing_eot_stops_at_the_cap_and_latches_error() {
    let mut pci = PciConfig::new();
    program_bmide(&mut pci, true);

    const PRDT: u32 = 0x0000_1000;
    set_prdt(&mut pci, PRDT);

    let mut mem = vec![0u8; 0x4000];
    for index in 0..300usize {
        if PRDT as usize + (index + 1) * PCI_PIIX_IDE_PRD_ENTRY_SIZE > mem.len() {
            break;
        }
        store_prd(&mut mem, PRDT, index, 0x0000_3000, 2, false);
    }

    let mut device = vec![0u8; 8];
    let err = pci
        .start_bm_write(&mut device, |phys| mem[phys as usize])
        .expect_err("no EOT");
    match err {
        BmidePrdError::MissingEot { entries_walked, .. } => assert_eq!(entries_walked, 256),
        other => panic!("unexpected error {other:?}"),
    }
    assert_eq!(bmista(&pci) & BMISTA_ERROR, BMISTA_ERROR);
    assert_eq!(bmista(&pci) & PCI_PIIX_IDE_BMISTA_ACTIVE, 0);
    assert_eq!(bmicom(&pci) & PCI_PIIX_IDE_BMICOM_SSBM, 0);
}

#[test]
fn write_direction_rejects_regions_that_wrap_32_bit_space() {
    let mut pci = PciConfig::new();
    program_bmide(&mut pci, true);

    const PRDT: u32 = 0x0000_1000;
    set_prdt(&mut pci, PRDT);

    let mut mem = vec![0u8; 0x2000];
    store_prd(&mut mem, PRDT, 0, 0xFFFF_FFFE, 8, true);

    let mut device = [0u8; 8];
    let err = pci
        .start_bm_write(&mut device, |phys| {
            mem.get(phys as usize).copied().unwrap_or(0)
        })
        .expect_err("wrapping region");
    match err {
        BmidePrdError::GuestAddressOverflow {
            phys_addr,
            bytes_requested,
        } => {
            assert_eq!(phys_addr, 0xFFFF_FFFE);
            assert_eq!(bytes_requested, 8);
        }
        other => panic!("unexpected error {other:?}"),
    }
    assert_eq!(device, [0u8; 8], "no partial copy from a rejected region");
    assert_eq!(bmista(&pci) & BMISTA_ERROR, BMISTA_ERROR);
}

#[test]
fn write_direction_requires_bus_master_and_a_buffer() {
    let mut pci = PciConfig::new();
    program_bmide(&mut pci, false);
    set_prdt(&mut pci, 0x0000_1000);
    let mem = vec![0u8; 0x2000];

    let mut device = [0u8; 4];
    assert_eq!(
        pci.start_bm_write(&mut device, |phys| mem[phys as usize]),
        Err(BmidePrdError::BusMasterDisabled)
    );
    assert_eq!(bmicom(&pci) & PCI_PIIX_IDE_BMICOM_SSBM, 0);
    assert_eq!(bmista(&pci), 0);

    program_bmide(&mut pci, true);
    let mut empty: [u8; 0] = [];
    assert_eq!(
        pci.start_bm_write(&mut empty, |phys| mem[phys as usize]),
        Err(BmidePrdError::EmptyBuffer)
    );
    assert_eq!(bmicom(&pci) & PCI_PIIX_IDE_BMICOM_SSBM, 0);
}

/// The read direction still clears RWCON, so the two directions do not share
/// latched state beyond the register file.
#[test]
fn read_direction_still_clears_rwcon_after_a_write_transfer() {
    let mut pci = PciConfig::new();
    program_bmide(&mut pci, true);

    const PRDT: u32 = 0x0000_1000;
    const BUF: u32 = 0x0000_3000;
    set_prdt(&mut pci, PRDT);

    let mut mem = vec![0u8; 0x4000];
    store_prd(&mut mem, PRDT, 0, BUF, 4, true);

    let mut device = [0u8; 4];
    pci.start_bm_write(&mut device, |phys| mem[phys as usize])
        .expect("write direction");
    assert_eq!(
        bmicom(&pci) & PCI_PIIX_IDE_BMICOM_RWCON,
        PCI_PIIX_IDE_BMICOM_RWCON
    );

    let source = [0x11u8, 0x22, 0x33, 0x44];
    {
        use std::cell::RefCell;
        let ram = RefCell::new(mem);
        pci.start_bm_read(
            &source,
            |phys| ram.borrow()[phys as usize],
            |phys, byte| ram.borrow_mut()[phys as usize] = byte,
        )
        .expect("read direction");
        assert_eq!(&ram.borrow()[BUF as usize..BUF as usize + 4], &source);
    }
    assert_eq!(bmicom(&pci) & PCI_PIIX_IDE_BMICOM_RWCON, 0);
}
