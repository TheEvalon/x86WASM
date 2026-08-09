//! Device-level tests for the VGA Graphics Controller read/write data path
//! (write modes 0–3, read modes 0–1, and the four 8-bit latches) over plane
//! memory, driven through port I/O.
//!
//! Spec: IBM PS/2 Hardware Interface Technical Reference — Video Subsystems
//! (Sep 1992) chapter 2 "VGA Function": Figure 2-66 Set/Reset, Figure 2-67
//! Enable Set/Reset, Figure 2-68 Color Compare, Figures 2-69/2-70 Data Rotate /
//! Function Select, Figure 2-71 Read Map Select, Figures 2-72/2-73 Graphics
//! Mode and Write Mode Definitions, Figure 2-76 Color Don't Care, Figure 2-77
//! Bit Mask. OSDev VGA Hardware "Read/Write logic" for the per-step ordering.
//!
//! See `docs/vga-plane-memory-model.md`.

use devices::{
    PortDevice, VgaText, VGA_GC_BIT_MASK, VGA_GC_COLOR_COMPARE, VGA_GC_COLOR_DONT_CARE,
    VGA_GC_DATA, VGA_GC_DATA_ROTATE, VGA_GC_ENABLE_SET_RESET, VGA_GC_INDEX,
    VGA_GC_MEMORY_MAP_B8000_32K, VGA_GC_MISC, VGA_GC_MISC_GRAPHICS_MODE,
    VGA_GC_MISC_MEMORY_MAP_SHIFT, VGA_GC_MODE, VGA_GC_READ_MAP_SELECT, VGA_GC_SET_RESET,
    VGA_SEQ_DATA, VGA_SEQ_INDEX, VGA_SEQ_MAP_MASK, VGA_SEQ_MAP_MASK_PLANES, VGA_SEQ_MEMORY_MODE,
    VGA_SEQ_MEMORY_MODE_EXTENDED, VGA_SEQ_MEMORY_MODE_ODD_EVEN_DISABLE, VGA_TEXT_BASE,
};

/// Graphics Controller indexes (FreeVGA / IBM VGA).
const GC_SET_RESET: u32 = VGA_GC_SET_RESET as u32;
const GC_ENABLE_SET_RESET: u32 = VGA_GC_ENABLE_SET_RESET as u32;
const GC_COLOR_COMPARE: u32 = VGA_GC_COLOR_COMPARE as u32;
const GC_DATA_ROTATE: u32 = VGA_GC_DATA_ROTATE as u32;
const GC_READ_MAP_SELECT: u32 = VGA_GC_READ_MAP_SELECT as u32;
const GC_MODE: u32 = VGA_GC_MODE as u32;
const GC_COLOR_DONT_CARE: u32 = VGA_GC_COLOR_DONT_CARE as u32;
const GC_MISC: u32 = VGA_GC_MISC as u32;
const GC_BIT_MASK: u32 = VGA_GC_BIT_MASK as u32;
/// Miscellaneous: graphics mode, Chain Odd/Even clear, Memory Map Select `11`.
const GC_MISC_GRAPHICS_B8000: u32 = VGA_GC_MISC_GRAPHICS_MODE as u32
    | ((VGA_GC_MEMORY_MAP_B8000_32K as u32) << VGA_GC_MISC_MEMORY_MAP_SHIFT);

/// Sequencer indexes.
const SEQ_MAP_MASK: u32 = VGA_SEQ_MAP_MASK as u32;
const SEQ_MEMORY_MODE: u32 = VGA_SEQ_MEMORY_MODE as u32;
/// Extended Memory | Odd/Even disable → planar addressing of all four maps.
const MEMORY_MODE_PLANAR: u32 =
    VGA_SEQ_MEMORY_MODE_EXTENDED as u32 | VGA_SEQ_MEMORY_MODE_ODD_EVEN_DISABLE as u32;
/// Map Mask with every map write-enabled.
const MAP_MASK_ALL_PLANES: u32 = VGA_SEQ_MAP_MASK_PLANES as u32;

fn write_gc(vga: &mut VgaText, index: u32, value: u32) {
    vga.port_write(VGA_GC_INDEX, 1, index);
    vga.port_write(VGA_GC_DATA, 1, value);
}

fn write_seq(vga: &mut VgaText, index: u32, value: u32) {
    vga.port_write(VGA_SEQ_INDEX, 1, index);
    vga.port_write(VGA_SEQ_DATA, 1, value);
}

/// Planar addressing with every map write-enabled — the mode-11h-class setup
/// the graphics write path is specified against. Graphics Controller
/// Miscellaneous also has to leave Chain Odd/Even clear (IBM Figure 2-74 OE);
/// the window stays at `0xB8000` so the tests keep one address base.
fn planar_all_maps() -> VgaText {
    let mut vga = VgaText::new();
    write_seq(&mut vga, SEQ_MEMORY_MODE, MEMORY_MODE_PLANAR);
    write_seq(&mut vga, SEQ_MAP_MASK, MAP_MASK_ALL_PLANES);
    write_gc(&mut vga, GC_MISC, GC_MISC_GRAPHICS_B8000);
    vga
}

fn planes_at(vga: &VgaText, offset: usize) -> [u8; 4] {
    [
        vga.plane_byte(0, offset).unwrap(),
        vga.plane_byte(1, offset).unwrap(),
        vga.plane_byte(2, offset).unwrap(),
        vga.plane_byte(3, offset).unwrap(),
    ]
}

fn seed_planes(vga: &mut VgaText, offset: usize, values: [u8; 4]) {
    for (plane, value) in values.iter().enumerate() {
        assert!(vga.set_plane_byte(plane, offset, *value));
    }
}

#[test]
fn reset_clears_plane_memory_and_latches() {
    let mut vga = planar_all_maps();
    seed_planes(&mut vga, 0, [0x11, 0x22, 0x33, 0x44]);
    assert!(vga.gc_read_u8(VGA_TEXT_BASE).is_some());
    assert_eq!(vga.gc_latches, [0x11, 0x22, 0x33, 0x44]);

    vga.reset();
    assert_eq!(vga.gc_latches, [0, 0, 0, 0]);
    assert_eq!(planes_at(&vga, 0), [0, 0, 0, 0]);
}

/// Spec: OSDev VGA Hardware "The Latches" — a load from video memory fills all
/// four latches; Figure 2-71 Read Map Select picks the map returned in read
/// mode 0.
#[test]
fn read_mode0_loads_four_latches_and_returns_the_selected_map() {
    let mut vga = planar_all_maps();
    seed_planes(&mut vga, 0x40, [0xA0, 0xB1, 0xC2, 0xD3]);

    write_gc(&mut vga, GC_READ_MAP_SELECT, 2);
    assert_eq!(vga.gc_read_u8(VGA_TEXT_BASE + 0x40), Some(0xC2));
    assert_eq!(vga.gc_latches, [0xA0, 0xB1, 0xC2, 0xD3]);

    write_gc(&mut vga, GC_READ_MAP_SELECT, 0);
    assert_eq!(vga.gc_read_u8(VGA_TEXT_BASE + 0x40), Some(0xA0));
}

/// Spec: Figures 2-33 / 2-34 + Figure 2-72 RM — in chain-4 the two low-order
/// address bits select the map read instead of Read Map Select.
#[test]
fn read_mode0_chain4_selects_the_map_from_the_address() {
    let mut vga = planar_all_maps();
    seed_planes(&mut vga, 0, [0x10, 0x20, 0x30, 0x40]);
    // Extended Memory | Chain 4.
    write_seq(&mut vga, SEQ_MEMORY_MODE, 0x0A);
    write_gc(&mut vga, GC_READ_MAP_SELECT, 0);

    assert_eq!(vga.gc_read_u8(VGA_TEXT_BASE), Some(0x10));
    assert_eq!(vga.gc_read_u8(VGA_TEXT_BASE + 1), Some(0x20));
    assert_eq!(vga.gc_read_u8(VGA_TEXT_BASE + 2), Some(0x30));
    assert_eq!(vga.gc_read_u8(VGA_TEXT_BASE + 3), Some(0x40));
}

/// Spec: Figure 2-68 Color Compare + Figure 2-76 Color Don't Care + Figure
/// 2-72 RM=1 — a result bit is 1 where every participating map matches its
/// color-compare bit.
#[test]
fn read_mode1_returns_color_compare_result() {
    let mut vga = planar_all_maps();
    // Pixel columns (bit 7 … bit 0) build 4-bit colors across the maps.
    seed_planes(
        &mut vga,
        0,
        [0b1010_0000, 0b1100_0000, 0b0000_0000, 0b0000_0000],
    );
    write_gc(&mut vga, GC_MODE, 0x08); // read mode 1
    write_gc(&mut vga, GC_COLOR_COMPARE, 0b0011);
    write_gc(&mut vga, GC_COLOR_DONT_CARE, 0b1111);

    // Color 3 (maps 0+1 set, maps 2+3 clear) occurs only in bit 7.
    assert_eq!(vga.gc_read_u8(VGA_TEXT_BASE), Some(0b1000_0000));

    // Ignoring map 1 also matches bit 5 (map 0 set, maps 2/3 clear).
    write_gc(&mut vga, GC_COLOR_DONT_CARE, 0b1101);
    assert_eq!(vga.gc_read_u8(VGA_TEXT_BASE), Some(0b1010_0000));
    assert_eq!(vga.gc_latches, [0b1010_0000, 0b1100_0000, 0, 0]);
}

/// Spec: Figure 2-73 write mode 00 + Figure 2-29 Map Mask — each enabled map
/// receives the rotated system data; disabled maps are untouched.
#[test]
fn write_mode0_writes_only_map_mask_enabled_maps() {
    let mut vga = planar_all_maps();
    write_seq(&mut vga, SEQ_MAP_MASK, 0b0101);
    assert!(vga.gc_write_u8(VGA_TEXT_BASE + 8, 0x5A));
    assert_eq!(planes_at(&vga, 8), [0x5A, 0x00, 0x5A, 0x00]);
}

/// Spec: Figure 2-69 RC — "the number of positions the system data is rotated
/// to the right"; Figure 2-70 — Function Select mixes with the latched data.
#[test]
fn write_mode0_rotates_then_applies_function_select() {
    let mut vga = planar_all_maps();
    write_gc(&mut vga, GC_DATA_ROTATE, 0x02); // rotate right 2, replace
    assert!(vga.gc_write_u8(VGA_TEXT_BASE, 0b0000_0011));
    assert_eq!(planes_at(&vga, 0), [0b1100_0000; 4]);

    // XOR (function 11b) against the latches loaded by a read.
    let mut vga = planar_all_maps();
    seed_planes(&mut vga, 0, [0xFF, 0xFF, 0xFF, 0xFF]);
    vga.gc_read_u8(VGA_TEXT_BASE);
    write_gc(&mut vga, GC_DATA_ROTATE, 0x18);
    assert!(vga.gc_write_u8(VGA_TEXT_BASE, 0x0F));
    assert_eq!(planes_at(&vga, 0), [0xF0; 4]);

    // AND (function 01b).
    write_gc(&mut vga, GC_DATA_ROTATE, 0x08);
    vga.gc_read_u8(VGA_TEXT_BASE); // latches = 0xF0
    assert!(vga.gc_write_u8(VGA_TEXT_BASE, 0x3C));
    assert_eq!(planes_at(&vga, 0), [0x30; 4]);
}

/// Spec: Figures 2-66 / 2-67 — with Enable Set/Reset the map receives the
/// Set/Reset bit expanded to all 8 positions instead of the system data.
#[test]
fn write_mode0_set_reset_replaces_enabled_maps() {
    let mut vga = planar_all_maps();
    write_gc(&mut vga, GC_ENABLE_SET_RESET, 0b0011);
    write_gc(&mut vga, GC_SET_RESET, 0b0001);
    assert!(vga.gc_write_u8(VGA_TEXT_BASE, 0x5A));
    assert_eq!(planes_at(&vga, 0), [0xFF, 0x00, 0x5A, 0x5A]);
}

/// Spec: Figure 2-77 Bit Mask — a clear mask bit keeps the latched bit.
#[test]
fn write_mode0_bit_mask_preserves_latched_bits() {
    let mut vga = planar_all_maps();
    seed_planes(&mut vga, 0, [0xFF, 0xFF, 0xFF, 0xFF]);
    vga.gc_read_u8(VGA_TEXT_BASE);
    write_gc(&mut vga, GC_BIT_MASK, 0x0F);
    assert!(vga.gc_write_u8(VGA_TEXT_BASE, 0x00));
    assert_eq!(planes_at(&vga, 0), [0xF0; 4]);
}

/// Spec: Figure 2-73 write mode 01 — "Each memory map is written with the
/// contents of the system latches."
#[test]
fn write_mode1_copies_latches_to_enabled_maps() {
    let mut vga = planar_all_maps();
    seed_planes(&mut vga, 0x100, [0x12, 0x34, 0x56, 0x78]);
    vga.gc_read_u8(VGA_TEXT_BASE + 0x100);

    write_gc(&mut vga, GC_MODE, 0x01);
    write_gc(&mut vga, GC_BIT_MASK, 0x00); // ignored in write mode 1
    assert!(vga.gc_write_u8(VGA_TEXT_BASE + 0x200, 0x00));
    assert_eq!(planes_at(&vga, 0x200), [0x12, 0x34, 0x56, 0x78]);
}

/// Spec: Figure 2-73 write mode 10 — "Memory map n is filled with 8 bits of
/// the value of data bit n"; the bit mask still applies.
#[test]
fn write_mode2_expands_data_bits_into_maps() {
    let mut vga = planar_all_maps();
    write_gc(&mut vga, GC_MODE, 0x02);
    assert!(vga.gc_write_u8(VGA_TEXT_BASE, 0b0000_1001));
    assert_eq!(planes_at(&vga, 0), [0xFF, 0x00, 0x00, 0xFF]);

    seed_planes(&mut vga, 0x20, [0xFF, 0xFF, 0xFF, 0xFF]);
    vga.gc_read_u8(VGA_TEXT_BASE + 0x20);
    write_gc(&mut vga, GC_BIT_MASK, 0b1111_0000);
    assert!(vga.gc_write_u8(VGA_TEXT_BASE + 0x20, 0b0000_0010));
    assert_eq!(planes_at(&vga, 0x20), [0x0F, 0xFF, 0x0F, 0x0F]);
}

/// Spec: Figure 2-73 write mode 11 — each map is written with the Set/Reset
/// value (Enable Set/Reset has no effect) and rotated system data ANDed with
/// the Bit Mask register forms the effective mask.
#[test]
fn write_mode3_masks_set_reset_against_latches() {
    let mut vga = planar_all_maps();
    seed_planes(&mut vga, 0, [0xFF, 0xFF, 0xFF, 0xFF]);
    vga.gc_read_u8(VGA_TEXT_BASE);

    write_gc(&mut vga, GC_MODE, 0x03);
    write_gc(&mut vga, GC_ENABLE_SET_RESET, 0x00);
    write_gc(&mut vga, GC_SET_RESET, 0b0101);
    write_gc(&mut vga, GC_BIT_MASK, 0xFF);
    assert!(vga.gc_write_u8(VGA_TEXT_BASE, 0b1111_0000));
    assert_eq!(planes_at(&vga, 0), [0xFF, 0x0F, 0xFF, 0x0F]);
}

/// Spec: FreeVGA / IBM Miscellaneous Output bit1 RAM Enable — CPU accesses to
/// video RAM are disabled when it is clear.
#[test]
fn ram_disable_blocks_the_graphics_data_path() {
    let mut vga = planar_all_maps();
    seed_planes(&mut vga, 0, [0x11, 0x22, 0x33, 0x44]);
    vga.port_write(0x3C2, 1, 0x65); // Misc Output with RAM Enable clear

    assert_eq!(vga.gc_read_u8(VGA_TEXT_BASE), None);
    assert!(!vga.gc_write_u8(VGA_TEXT_BASE, 0xFF));
    assert_eq!(planes_at(&vga, 0), [0x11, 0x22, 0x33, 0x44]);
    assert_eq!(vga.gc_latches, [0, 0, 0, 0]);
}

/// Addresses outside the CPU display window are not claimed.
#[test]
fn graphics_data_path_ignores_addresses_outside_the_window() {
    let mut vga = planar_all_maps();
    assert_eq!(vga.gc_read_u8(VGA_TEXT_BASE - 1), None);
    assert!(!vga.gc_write_u8(VGA_TEXT_BASE - 1, 0xFF));
}
