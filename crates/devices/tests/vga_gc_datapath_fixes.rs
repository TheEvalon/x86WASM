//! Device-level tests for the two Graphics Controller data-path gaps left by
//! round 1 and recorded in `plan.md` §21 Milestone 2:
//!
//! 1. Write mode 3 did not pass the Set/Reset value through Function Select.
//! 2. Graphics Mode bit 4 (Host Odd/Even Memory Read Addressing) did not steer
//!    read-mode-0 map selection.
//!
//! Spec: IBM PS/2 Hardware Interface Technical Reference — Video Subsystems
//! (Sep 1992) chapter 2 "VGA Function": Figures 2-69 / 2-70 Data Rotate and
//! Operation (Function) Select, Figures 2-72 / 2-73 Graphics Mode and Write
//! Mode Definitions, Figure 2-71 Read Map Select, Figures 2-33 / 2-34
//! Sequencer Memory Mode addressing. Michael Abrash, *Graphics Programming
//! Black Book* chapter 26 "VGA Write Mode 3" — the write-mode-3 helper
//! documents that it "Forces ALU function to 'move'", i.e. the Function Select
//! ALU stage stays in the data path under write mode 3. FreeVGA Graphics
//! Registers, Graphics Mode bit 4 "Host Odd/Even Memory Read Addressing
//! Enable" — "selects the odd/even addressing mode used by the IBM
//! Color/Graphics Monitor Adapter"; the host address bit A0 replaces bit 0 of
//! Read Map Select for system reads.
//!
//! See `docs/vga-r2-gc-datapath-fixes.md`.

use devices::{
    PortDevice, VgaText, VGA_GC_BIT_MASK, VGA_GC_COLOR_COMPARE, VGA_GC_COLOR_DONT_CARE,
    VGA_GC_DATA, VGA_GC_DATA_ROTATE, VGA_GC_ENABLE_SET_RESET, VGA_GC_FUNCTION_AND,
    VGA_GC_FUNCTION_OR, VGA_GC_FUNCTION_REPLACE, VGA_GC_FUNCTION_XOR, VGA_GC_INDEX,
    VGA_GC_MEMORY_MAP_A0000_64K, VGA_GC_MISC, VGA_GC_MISC_GRAPHICS_MODE,
    VGA_GC_MISC_MEMORY_MAP_SHIFT, VGA_GC_MODE, VGA_GC_MODE_DEFAULT, VGA_GC_MODE_READ,
    VGA_GC_READ_MAP_SELECT, VGA_GC_SET_RESET, VGA_SEQ_DATA, VGA_SEQ_INDEX, VGA_SEQ_MAP_MASK,
    VGA_SEQ_MAP_MASK_PLANES, VGA_SEQ_MEMORY_MODE, VGA_SEQ_MEMORY_MODE_CHAIN4,
    VGA_SEQ_MEMORY_MODE_EXTENDED, VGA_SEQ_MEMORY_MODE_ODD_EVEN_DISABLE, VGA_WINDOW_A0000_BASE,
};

/// Graphics Mode bit 4 — Host Odd/Even Memory Read Addressing Enable. Set in
/// the mode-03h reset default `0x10`.
const GC_MODE_HOST_ODD_EVEN_READ: u8 = 0x10;
const _: () = assert!(VGA_GC_MODE_DEFAULT == GC_MODE_HOST_ODD_EVEN_READ);

/// Write mode 3.
const WRITE_MODE_3: u8 = 0x03;

fn write_gc(vga: &mut VgaText, index: u8, value: u8) {
    vga.port_write(VGA_GC_INDEX, 1, u32::from(index));
    vga.port_write(VGA_GC_DATA, 1, u32::from(value));
}

fn write_seq(vga: &mut VgaText, index: u8, value: u8) {
    vga.port_write(VGA_SEQ_INDEX, 1, u32::from(index));
    vga.port_write(VGA_SEQ_DATA, 1, u32::from(value));
}

/// Planar addressing, all maps write-enabled, `0xA0000` 64 KB graphics window.
fn planar_graphics_window() -> VgaText {
    let mut vga = VgaText::new();
    write_seq(
        &mut vga,
        VGA_SEQ_MEMORY_MODE,
        VGA_SEQ_MEMORY_MODE_EXTENDED | VGA_SEQ_MEMORY_MODE_ODD_EVEN_DISABLE,
    );
    write_seq(&mut vga, VGA_SEQ_MAP_MASK, VGA_SEQ_MAP_MASK_PLANES);
    write_gc(
        &mut vga,
        VGA_GC_MISC,
        VGA_GC_MISC_GRAPHICS_MODE | (VGA_GC_MEMORY_MAP_A0000_64K << VGA_GC_MISC_MEMORY_MAP_SHIFT),
    );
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

/// Load the latches at `offset` with `values`.
fn latch(vga: &mut VgaText, offset: usize, values: [u8; 4]) {
    seed_planes(vga, offset, values);
    vga.gc_read_u8(VGA_WINDOW_A0000_BASE + offset as u64)
        .expect("claimed read");
}

// ---------------------------------------------------------------------------
// 1. Write mode 3 applies Function Select.
// ---------------------------------------------------------------------------

/// Function Select XOR mixes the expanded Set/Reset value with the latched
/// byte before the synthesized bit mask selects between them.
#[test]
fn write_mode3_applies_function_select_xor() {
    let mut vga = planar_graphics_window();
    latch(&mut vga, 0, [0xF0; 4]);

    write_gc(&mut vga, VGA_GC_MODE, WRITE_MODE_3);
    write_gc(&mut vga, VGA_GC_SET_RESET, 0b0101);
    write_gc(&mut vga, VGA_GC_BIT_MASK, 0xFF);
    write_gc(&mut vga, VGA_GC_DATA_ROTATE, VGA_GC_FUNCTION_XOR);

    // Synthesized mask = rotated data (0xFF) AND Bit Mask (0xFF) = 0xFF, so
    // every bit comes from the ALU: maps 0/2 see 0xFF ^ 0xF0, maps 1/3 see
    // 0x00 ^ 0xF0.
    assert!(vga.gc_write_u8(VGA_WINDOW_A0000_BASE, 0xFF));
    assert_eq!(planes_at(&vga, 0), [0x0F, 0xF0, 0x0F, 0xF0]);
}

/// Function Select AND against the latches.
#[test]
fn write_mode3_applies_function_select_and() {
    let mut vga = planar_graphics_window();
    latch(&mut vga, 0x10, [0xF0; 4]);

    write_gc(&mut vga, VGA_GC_MODE, WRITE_MODE_3);
    write_gc(&mut vga, VGA_GC_SET_RESET, VGA_SEQ_MAP_MASK_PLANES);
    write_gc(&mut vga, VGA_GC_BIT_MASK, 0xFF);
    write_gc(&mut vga, VGA_GC_DATA_ROTATE, VGA_GC_FUNCTION_AND);

    assert!(vga.gc_write_u8(VGA_WINDOW_A0000_BASE + 0x10, 0xFF));
    assert_eq!(planes_at(&vga, 0x10), [0xF0; 4]);
}

/// Function Select OR against the latches.
#[test]
fn write_mode3_applies_function_select_or() {
    let mut vga = planar_graphics_window();
    latch(&mut vga, 0x20, [0x0F; 4]);

    write_gc(&mut vga, VGA_GC_MODE, WRITE_MODE_3);
    write_gc(&mut vga, VGA_GC_SET_RESET, 0b0010);
    write_gc(&mut vga, VGA_GC_BIT_MASK, 0xFF);
    write_gc(&mut vga, VGA_GC_DATA_ROTATE, VGA_GC_FUNCTION_OR);

    assert!(vga.gc_write_u8(VGA_WINDOW_A0000_BASE + 0x20, 0xFF));
    assert_eq!(planes_at(&vga, 0x20), [0x0F, 0xFF, 0x0F, 0x0F]);
}

/// The synthesized mask still selects the latch for clear bits, and the ALU
/// only affects the bits the mask lets through.
#[test]
fn write_mode3_function_select_respects_the_synthesized_mask() {
    let mut vga = planar_graphics_window();
    latch(&mut vga, 0x30, [0xFF; 4]);

    write_gc(&mut vga, VGA_GC_MODE, WRITE_MODE_3);
    write_gc(&mut vga, VGA_GC_SET_RESET, 0b0001);
    write_gc(&mut vga, VGA_GC_BIT_MASK, 0xFF);
    write_gc(&mut vga, VGA_GC_DATA_ROTATE, VGA_GC_FUNCTION_XOR);

    // Mask = 0xF0: high nibble from the ALU, low nibble from the latch.
    assert!(vga.gc_write_u8(VGA_WINDOW_A0000_BASE + 0x30, 0b1111_0000));
    assert_eq!(planes_at(&vga, 0x30), [0x0F, 0xFF, 0xFF, 0xFF]);
}

/// Rotate Count still forms the mask, and Function Select `00` (replace/move —
/// what a well-behaved driver programs) keeps the round-1 behavior.
#[test]
fn write_mode3_replace_function_is_unchanged() {
    let mut vga = planar_graphics_window();
    latch(&mut vga, 0x40, [0xFF; 4]);

    write_gc(&mut vga, VGA_GC_MODE, WRITE_MODE_3);
    write_gc(&mut vga, VGA_GC_SET_RESET, 0b0101);
    write_gc(&mut vga, VGA_GC_BIT_MASK, 0xFF);
    // Rotate right 4 with Function Select = replace.
    write_gc(&mut vga, VGA_GC_DATA_ROTATE, VGA_GC_FUNCTION_REPLACE | 0x04);

    // 0x0F rotated right 4 = 0xF0 → mask 0xF0.
    assert!(vga.gc_write_u8(VGA_WINDOW_A0000_BASE + 0x40, 0x0F));
    assert_eq!(planes_at(&vga, 0x40), [0xFF, 0x0F, 0xFF, 0x0F]);
}

/// Enable Set/Reset is still ignored by write mode 3 (IBM Figure 2-73).
#[test]
fn write_mode3_still_ignores_enable_set_reset() {
    let mut vga = planar_graphics_window();
    latch(&mut vga, 0x50, [0xF0; 4]);

    write_gc(&mut vga, VGA_GC_MODE, WRITE_MODE_3);
    write_gc(&mut vga, VGA_GC_ENABLE_SET_RESET, 0x00);
    write_gc(&mut vga, VGA_GC_SET_RESET, 0b0101);
    write_gc(&mut vga, VGA_GC_BIT_MASK, 0xFF);
    write_gc(&mut vga, VGA_GC_DATA_ROTATE, VGA_GC_FUNCTION_XOR);

    assert!(vga.gc_write_u8(VGA_WINDOW_A0000_BASE + 0x50, 0xFF));
    assert_eq!(planes_at(&vga, 0x50), [0x0F, 0xF0, 0x0F, 0xF0]);
}

/// Function Select must not leak into write mode 1, which copies the latches.
#[test]
fn write_mode1_ignores_function_select() {
    let mut vga = planar_graphics_window();
    latch(&mut vga, 0x60, [0x12, 0x34, 0x56, 0x78]);

    write_gc(&mut vga, VGA_GC_MODE, 0x01);
    write_gc(&mut vga, VGA_GC_DATA_ROTATE, VGA_GC_FUNCTION_XOR);
    assert!(vga.gc_write_u8(VGA_WINDOW_A0000_BASE + 0x70, 0xFF));
    assert_eq!(planes_at(&vga, 0x70), [0x12, 0x34, 0x56, 0x78]);
}

// ---------------------------------------------------------------------------
// 2. Graphics Mode bit 4 steers read-mode-0 map selection.
// ---------------------------------------------------------------------------

/// With Host Odd/Even read addressing set, host address bit A0 replaces bit 0
/// of Read Map Select: even addresses read the even map of the pair, odd
/// addresses the odd map.
#[test]
fn read_mode0_host_odd_even_substitutes_a0_for_read_map_select_bit0() {
    let mut vga = planar_graphics_window();
    seed_planes(&mut vga, 0, [0xA0, 0xB1, 0xC2, 0xD3]);
    seed_planes(&mut vga, 1, [0xA4, 0xB5, 0xC6, 0xD7]);

    write_gc(&mut vga, VGA_GC_MODE, GC_MODE_HOST_ODD_EVEN_READ);

    write_gc(&mut vga, VGA_GC_READ_MAP_SELECT, 0);
    assert_eq!(vga.gc_read_u8(VGA_WINDOW_A0000_BASE), Some(0xA0));
    assert_eq!(vga.gc_read_u8(VGA_WINDOW_A0000_BASE + 1), Some(0xB5));

    write_gc(&mut vga, VGA_GC_READ_MAP_SELECT, 2);
    assert_eq!(vga.gc_read_u8(VGA_WINDOW_A0000_BASE), Some(0xC2));
    assert_eq!(vga.gc_read_u8(VGA_WINDOW_A0000_BASE + 1), Some(0xD7));

    // Only bit 0 of Read Map Select is replaced, so 3 behaves like 2.
    write_gc(&mut vga, VGA_GC_READ_MAP_SELECT, 3);
    assert_eq!(vga.gc_read_u8(VGA_WINDOW_A0000_BASE), Some(0xC2));
    assert_eq!(vga.gc_read_u8(VGA_WINDOW_A0000_BASE + 1), Some(0xD7));
}

/// With the bit clear, Read Map Select alone picks the map at every address.
#[test]
fn read_mode0_without_host_odd_even_uses_read_map_select_alone() {
    let mut vga = planar_graphics_window();
    seed_planes(&mut vga, 0, [0xA0, 0xB1, 0xC2, 0xD3]);
    seed_planes(&mut vga, 1, [0xA4, 0xB5, 0xC6, 0xD7]);

    write_gc(&mut vga, VGA_GC_MODE, 0x00);
    write_gc(&mut vga, VGA_GC_READ_MAP_SELECT, 0);

    assert_eq!(vga.gc_read_u8(VGA_WINDOW_A0000_BASE), Some(0xA0));
    assert_eq!(vga.gc_read_u8(VGA_WINDOW_A0000_BASE + 1), Some(0xA4));

    write_gc(&mut vga, VGA_GC_READ_MAP_SELECT, 3);
    assert_eq!(vga.gc_read_u8(VGA_WINDOW_A0000_BASE + 1), Some(0xD7));
}

/// The latches are still loaded from all four maps regardless of which map the
/// read returns. Spec: OSDev VGA Hardware "The Latches".
#[test]
fn host_odd_even_read_still_loads_all_four_latches() {
    let mut vga = planar_graphics_window();
    seed_planes(&mut vga, 1, [0xA4, 0xB5, 0xC6, 0xD7]);
    write_gc(&mut vga, VGA_GC_MODE, GC_MODE_HOST_ODD_EVEN_READ);
    write_gc(&mut vga, VGA_GC_READ_MAP_SELECT, 0);

    assert_eq!(vga.gc_read_u8(VGA_WINDOW_A0000_BASE + 1), Some(0xB5));
    assert_eq!(vga.gc_latches, [0xA4, 0xB5, 0xC6, 0xD7]);
}

/// Under Sequencer odd/even addressing the offset drops A0, but A0 still picks
/// the odd map for the read. Spec: IBM Figure 2-33 + FreeVGA Graphics Mode
/// bit 4.
#[test]
fn host_odd_even_read_combines_with_sequencer_odd_even_addressing() {
    let mut vga = planar_graphics_window();
    seed_planes(&mut vga, 0, [0xA0, 0xB1, 0xC2, 0xD3]);
    // Extended Memory, Odd/Even addressing enabled (bit 2 clear).
    write_seq(&mut vga, VGA_SEQ_MEMORY_MODE, VGA_SEQ_MEMORY_MODE_EXTENDED);
    write_gc(&mut vga, VGA_GC_MODE, GC_MODE_HOST_ODD_EVEN_READ);
    write_gc(&mut vga, VGA_GC_READ_MAP_SELECT, 0);

    // Both addresses resolve to map offset 0; A0 chooses map 0 vs map 1.
    assert_eq!(vga.gc_read_u8(VGA_WINDOW_A0000_BASE), Some(0xA0));
    assert_eq!(vga.gc_read_u8(VGA_WINDOW_A0000_BASE + 1), Some(0xB1));
}

/// Chain 4 still selects the map from A1:A0 and ignores the bit.
/// Spec: IBM Figures 2-33 / 2-34.
#[test]
fn chain4_read_addressing_takes_precedence_over_host_odd_even() {
    let mut vga = planar_graphics_window();
    seed_planes(&mut vga, 0, [0x10, 0x20, 0x30, 0x40]);
    write_seq(
        &mut vga,
        VGA_SEQ_MEMORY_MODE,
        VGA_SEQ_MEMORY_MODE_EXTENDED | VGA_SEQ_MEMORY_MODE_CHAIN4,
    );
    write_gc(&mut vga, VGA_GC_MODE, GC_MODE_HOST_ODD_EVEN_READ);
    write_gc(&mut vga, VGA_GC_READ_MAP_SELECT, 0);

    assert_eq!(vga.gc_read_u8(VGA_WINDOW_A0000_BASE), Some(0x10));
    assert_eq!(vga.gc_read_u8(VGA_WINDOW_A0000_BASE + 1), Some(0x20));
    assert_eq!(vga.gc_read_u8(VGA_WINDOW_A0000_BASE + 2), Some(0x30));
    assert_eq!(vga.gc_read_u8(VGA_WINDOW_A0000_BASE + 3), Some(0x40));
}

/// Read mode 1 compares maps and never uses map selection, so the bit has no
/// effect there. Spec: IBM Figures 2-68 / 2-72 / 2-76.
#[test]
fn read_mode1_is_unaffected_by_host_odd_even_read_addressing() {
    let mut vga = planar_graphics_window();
    seed_planes(&mut vga, 1, [0xFF, 0xFF, 0x00, 0x00]);
    write_gc(
        &mut vga,
        VGA_GC_MODE,
        VGA_GC_MODE_READ | GC_MODE_HOST_ODD_EVEN_READ,
    );
    write_gc(&mut vga, VGA_GC_COLOR_COMPARE, 0b0011);
    write_gc(&mut vga, VGA_GC_COLOR_DONT_CARE, 0b1111);

    assert_eq!(vga.gc_read_u8(VGA_WINDOW_A0000_BASE + 1), Some(0xFF));
}

/// The accessor reports the bit, including the mode-03h reset default.
#[test]
fn host_odd_even_read_accessor_tracks_the_register() {
    let mut vga = VgaText::new();
    assert!(vga.gc_host_odd_even_read());

    write_gc(&mut vga, VGA_GC_MODE, 0x00);
    assert!(!vga.gc_host_odd_even_read());

    vga.reset();
    assert!(vga.gc_host_odd_even_read());
}
