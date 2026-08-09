//! The configuration-data seam: what a guest actually reads back about memory.
//!
//! `devices` owns the CMOS register layout and the fw_cfg blob format but knows
//! nothing about the machine it is plugged into, so the new bytes stayed zero
//! and `etc/e820` stayed absent until the machine populated them. These tests
//! read the values the way firmware does — CMOS through `0x70`/`0x71`, fw_cfg
//! through `0x510`/`0x511` — and require the answers to describe the RAM the
//! machine was configured with.
//!
//! Spec: RBIL CMOS `15h`/`16h` (base memory in KB), `17h`/`18h` and `30h`/`31h`
//! (extended memory in KB), `34h`/`35h` (memory above 16 MB in 64 KB blocks),
//! `14h` Table C0019 (equipment byte), `2Eh`/`2Fh` (standard checksum over
//! `10h`–`2Dh`); RBIL INT 15h AX=E801h for the 15 MB / 16 MB split; QEMU fw_cfg
//! traditional interface; ACPI Specification §15 Table 15.4 (address range
//! descriptors) and §15.2 (range types).

use devices::{
    CmosRtc, E820Entry, CMOS_CHECKSUM_FIRST, CMOS_CHECKSUM_LAST, CMOS_EXT_MEMORY_MAX_KB,
    E820_TYPE_MEMORY, E820_TYPE_RESERVED, EQUIP_MATH_COPROCESSOR, FW_CFG_E820_ENTRY_SIZE,
    FW_CFG_FILE_E820, REG_BASE_MEM_HIGH, REG_BASE_MEM_LOW, REG_CHECKSUM_HIGH, REG_CHECKSUM_LOW,
    REG_EQUIPMENT, REG_EXT_MEM2_HIGH, REG_EXT_MEM2_LOW, REG_EXT_MEM_HIGH, REG_EXT_MEM_LOW,
    REG_MEM_ABOVE_16M_HIGH, REG_MEM_ABOVE_16M_LOW,
};
use machine_pc::Machine;

/// 32 MiB: above the 16 MB split, so `34h`/`35h` carries a non-zero block count
/// and the KB pairs saturate at RBIL's `3C00h`.
const RAM_BYTES: usize = 32 * 1024 * 1024;

/// Where the guest parks what it read.
const SCRATCH: u16 = 0x2000;

/// 64 KiB BIOS image whose reset vector far-jumps to `F000:0000`.
///
/// Spec: Intel SDM Vol. 3 §9.1.4.
fn bios_image_64k(code: &[u8]) -> Vec<u8> {
    let mut rom = vec![0xF4u8; 64 * 1024];
    rom[..code.len()].copy_from_slice(code);
    rom[0xFFF0..0xFFF5].copy_from_slice(&[0xEA, 0x00, 0x00, 0x00, 0xF0]);
    rom
}

fn run_guest(ram: usize, code: Vec<u8>) -> Machine {
    let rom = bios_image_64k(&code);
    let mut m = Machine::with_bios_rom(ram, &rom).expect("map BIOS image");
    m.reset();
    m.run(200_000).expect("guest runs to HLT");
    assert!(m.cpu.halted, "guest halted");
    m
}

/// `MOV AL, idx` / `OUT 0x70, AL` / `IN AL, 0x71` / `MOV ES:[DI], AL` / `INC DI`.
///
/// Spec: IBM PC/AT — CMOS index port `0x70`, data port `0x71`. Bit 7 of the
/// index byte is the NMI mask; POST-shaped code leaves it clear here.
#[rustfmt::skip]
fn read_cmos_byte_to_es_di(index: u8) -> Vec<u8> {
    vec![
        0xB0, index,        // MOV AL, index
        0xE6, 0x70,         // OUT 0x70, AL
        0xE4, 0x71,         // IN  AL, 0x71
        0x26, 0x88, 0x05,   // MOV ES:[DI], AL
        0x47,               // INC DI
    ]
}

/// The CMOS indices this test reads, in the order the guest stores them.
const CMOS_INDICES: [u8; 11] = [
    REG_BASE_MEM_LOW,
    REG_BASE_MEM_HIGH,
    REG_EXT_MEM_LOW,
    REG_EXT_MEM_HIGH,
    REG_EXT_MEM2_LOW,
    REG_EXT_MEM2_HIGH,
    REG_MEM_ABOVE_16M_LOW,
    REG_MEM_ABOVE_16M_HIGH,
    REG_EQUIPMENT,
    REG_CHECKSUM_HIGH,
    REG_CHECKSUM_LOW,
];

fn guest_bytes(m: &Machine, count: usize) -> Vec<u8> {
    (0..count)
        .map(|i| {
            m.mem
                .read_u8(u64::from(SCRATCH) + i as u64)
                .expect("scratch is RAM")
        })
        .collect()
}

fn le16(bytes: &[u8], index: usize) -> u16 {
    u16::from_le_bytes([bytes[index], bytes[index + 1]])
}

/// A guest reading the memory-size registers sees the configured RAM.
#[test]
fn guest_reads_a_coherent_memory_description_from_cmos() {
    let mut code = vec![
        0x31, 0xC0, // XOR AX, AX
        0x8E, 0xC0, // MOV ES, AX
    ];
    code.extend_from_slice(&[0xBF, SCRATCH.to_le_bytes()[0], SCRATCH.to_le_bytes()[1]]); // MOV DI
    for index in CMOS_INDICES {
        code.extend(read_cmos_byte_to_es_di(index));
    }
    code.push(0xF4); // HLT

    let m = run_guest(RAM_BYTES, code);
    let got = guest_bytes(&m, CMOS_INDICES.len());

    let ram = RAM_BYTES as u64;
    // Base memory stops at the 640 KB conventional limit.
    assert_eq!(le16(&got, 0), 640);
    // Extended memory in KB saturates at RBIL's 3C00h (15 MB).
    let ext_kb = ((ram - 1024 * 1024) / 1024).min(u64::from(CMOS_EXT_MEMORY_MAX_KB)) as u16;
    assert_eq!(le16(&got, 2), ext_kb);
    assert_eq!(le16(&got, 2), CMOS_EXT_MEMORY_MAX_KB);
    // 30h/31h reports the same figure as 17h/18h in this model.
    assert_eq!(le16(&got, 4), le16(&got, 2));
    // Above 16 MB, in 64 KB blocks: 32 MiB - 16 MiB = 256 blocks.
    assert_eq!(le16(&got, 6), 256);

    // The pairs add up to the configured RAM: 640 KB + the 384 KB legacy hole
    // + 15 MB + 16 MB.
    let described = 640 * 1024
        + 384 * 1024
        + u64::from(le16(&got, 2)) * 1024
        + u64::from(le16(&got, 6)) * 64 * 1024;
    assert_eq!(described, ram, "CMOS describes exactly the configured RAM");

    // Equipment byte: EGA/VGA display, display and keyboard enabled, no floppy
    // media, and no coprocessor (there is no x87 here).
    assert_eq!(got[8], m.equipment_byte());
    assert_eq!(got[8] & EQUIP_MATH_COPROCESSOR, 0);

    // Checksum bytes cover 10h-2Dh, which includes everything above.
    let stored = u16::from_be_bytes([got[9], got[10]]);
    assert_eq!(stored, m.cmos.standard_checksum());
    assert!(m.cmos.standard_checksum_valid());
    assert_ne!(stored, 0, "a zeroed CMOS would checksum to zero");
}

/// Attaching floppy media changes the equipment byte and the checksum with it.
#[test]
fn attaching_floppy_media_updates_the_equipment_byte_and_checksum() {
    let mut m = Machine::new(1024 * 1024);
    let before = m.cmos.equipment_byte();
    // No media, so bits 7-6 (drive count) and bit 0 (installed) are all clear.
    assert_eq!(before & 0xC1, CmosRtc::equipment_floppy_field(0));

    m.attach_floppy_image(vec![0u8; devices::FDC_1440_IMAGE_SIZE])
        .expect("attach 1.44MB image");

    assert_eq!(m.cmos.equipment_byte(), m.equipment_byte());
    assert_ne!(m.cmos.equipment_byte(), before);
    assert_eq!(
        m.cmos.equipment_byte() & 0xC1,
        CmosRtc::equipment_floppy_field(1)
    );
    assert!(
        m.cmos.standard_checksum_valid(),
        "the checksum was restored after the equipment byte changed"
    );
}

/// A guest reading `etc/e820` through the traditional fw_cfg interface sees a
/// map that covers the configured RAM.
///
/// The guest is handed the selector rather than walking the file directory:
/// the directory walk itself is covered by the device tests. Selectors are
/// below `0x100`, so the single-byte write to `0x510` sets the whole value.
#[test]
fn guest_reads_etc_e820_covering_the_configured_ram() {
    let probe = Machine::new(RAM_BYTES);
    let selector = probe
        .fw_cfg
        .file_selector(FW_CFG_FILE_E820)
        .expect("etc/e820 is published");
    assert!(probe.fw_cfg.file_names().contains(&FW_CFG_FILE_E820));
    assert!(
        selector < 0x100,
        "selector fits one byte write: {selector:#X}"
    );
    let expected: Vec<E820Entry> = probe.e820_entries();
    let blob_len = expected.len() * FW_CFG_E820_ENTRY_SIZE;

    #[rustfmt::skip]
    let mut code = vec![
        0xFC,                                   // CLD
        0x31, 0xC0,                             // XOR AX, AX
        0x8E, 0xC0,                             // MOV ES, AX
        0xBA, 0x10, 0x05,                       // MOV DX, 0x0510 (selector)
        0xB0, selector as u8,                   // MOV AL, selector
        0xEE,                                   // OUT DX, AL
        0xBA, 0x11, 0x05,                       // MOV DX, 0x0511 (data)
    ];
    code.extend_from_slice(&[0xBF, SCRATCH.to_le_bytes()[0], SCRATCH.to_le_bytes()[1]]); // MOV DI
    let [cx_lo, cx_hi] = (blob_len as u16).to_le_bytes();
    code.extend_from_slice(&[0xB9, cx_lo, cx_hi]); // MOV CX, blob_len
    code.extend_from_slice(&[0xF3, 0x6C]); // REP INSB — 0x511 → ES:DI
    code.push(0xF4); // HLT

    let m = run_guest(RAM_BYTES, code);
    let got = guest_bytes(&m, blob_len);

    let want: Vec<u8> = expected
        .iter()
        .flat_map(|e| e.to_descriptor())
        .collect::<Vec<u8>>();
    assert_eq!(got, want, "the guest read the descriptors byte for byte");

    // Decode what the guest holds and check it against the configured RAM.
    let mut usable = 0u64;
    let mut reserved = 0u64;
    for chunk in got.chunks_exact(FW_CFG_E820_ENTRY_SIZE) {
        let base = u64::from_le_bytes(chunk[0..8].try_into().unwrap());
        let length = u64::from_le_bytes(chunk[8..16].try_into().unwrap());
        let kind = u32::from_le_bytes(chunk[16..20].try_into().unwrap());
        match kind {
            E820_TYPE_MEMORY => usable += length,
            E820_TYPE_RESERVED => {
                reserved += length;
                assert_eq!(
                    base, 0x000A_0000,
                    "the only reserved range is the video hole"
                );
                assert_eq!(length, 0x0006_0000);
            }
            other => panic!("unexpected e820 type {other}"),
        }
    }
    assert_eq!(
        usable + reserved,
        RAM_BYTES as u64,
        "usable plus the legacy hole accounts for the configured RAM"
    );
    assert_eq!(usable, RAM_BYTES as u64 - 0x0006_0000);
}

/// The map shrinks honestly on a machine too small to have extended memory.
#[test]
fn e820_on_a_sub_megabyte_machine_describes_only_what_exists() {
    let m = Machine::new(640 * 1024);
    let entries = m.e820_entries();

    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0], E820Entry::new(0, 640 * 1024, E820_TYPE_MEMORY));
    assert_eq!(
        entries[1],
        E820Entry::new(0x000A_0000, 0x0006_0000, E820_TYPE_RESERVED)
    );
    assert_eq!(m.cmos.extended_memory_kb(), 0);
    assert_eq!(m.cmos.memory_above_16m_blocks(), 0);
}

/// The checksum range the machine maintains is the one RBIL documents.
#[test]
fn machine_checksum_covers_the_documented_range() {
    assert_eq!(CMOS_CHECKSUM_FIRST, 0x10);
    assert_eq!(CMOS_CHECKSUM_LAST, 0x2D);
    let m = Machine::new(1024 * 1024);
    let expected: u16 = (CMOS_CHECKSUM_FIRST..=CMOS_CHECKSUM_LAST)
        .map(|i| u16::from(m.cmos.read_reg(i)))
        .sum();
    assert_eq!(m.cmos.standard_checksum(), expected);
    assert!(m.cmos.standard_checksum_valid());
}
