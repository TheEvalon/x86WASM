//! Device-level tests for the Graphics Controller Miscellaneous register
//! (`0x3CE` index `0x06`): Memory Map Select (bits 3:2) choosing the CPU
//! display window, and Chain Odd/Even (bit1) forcing odd/even host addressing.
//!
//! Spec: IBM PS/2 Hardware Interface Technical Reference — Video Subsystems
//! (Sep 1992) Figure 2-74 Miscellaneous Register, index hex 06, and Figure 2-75
//! Video Memory Assignments (`00` = A0000 for 128 KB, `01` = A0000 for 64 KB,
//! `10` = B0000 for 32 KB, `11` = B8000 for 32 KB); Figure 2-74 OE — "directs
//! the system address bit, A0, to be replaced by a higher-order bit. The odd
//! map is then selected when A0 is 1, and the even map when A0 is 0."
//!
//! See `docs/vga-plane-memory-model.md`.

use devices::{
    PortDevice, VgaText, VGA_GC_DATA, VGA_GC_INDEX, VGA_GC_MEMORY_MAP_A0000_128K,
    VGA_GC_MEMORY_MAP_A0000_64K, VGA_GC_MEMORY_MAP_B0000_32K, VGA_GC_MEMORY_MAP_B8000_32K,
    VGA_GC_MISC, VGA_GC_MISC_CHAIN_ODD_EVEN, VGA_GC_MISC_GRAPHICS_MODE,
    VGA_GC_MISC_MEMORY_MAP_SHIFT, VGA_MISC_OUTPUT_DEFAULT, VGA_MISC_OUTPUT_WRITE,
    VGA_MISC_RAM_ENABLE, VGA_SEQ_DATA, VGA_SEQ_INDEX, VGA_SEQ_MEMORY_MODE,
    VGA_SEQ_MEMORY_MODE_EXTENDED, VGA_SEQ_MEMORY_MODE_ODD_EVEN_DISABLE, VGA_TEXT_BASE,
    VGA_TEXT_END, VGA_WINDOW_A0000_BASE, VGA_WINDOW_B0000_BASE,
};

/// Graphics Controller Miscellaneous register index.
const GC_MISC: u32 = VGA_GC_MISC as u32;
/// Sequencer Memory Mode register index.
const SEQ_MEMORY_MODE: u32 = VGA_SEQ_MEMORY_MODE as u32;

/// Misc values: graphics-mode bit plus a Memory Map Select field, with Chain
/// Odd/Even clear so only the window under test varies.
const fn misc_window(memory_map: u8) -> u32 {
    VGA_GC_MISC_GRAPHICS_MODE as u32 | ((memory_map as u32) << VGA_GC_MISC_MEMORY_MAP_SHIFT)
}
const MISC_MAP_A0000_128K: u32 = misc_window(VGA_GC_MEMORY_MAP_A0000_128K);
const MISC_MAP_A0000_64K: u32 = misc_window(VGA_GC_MEMORY_MAP_A0000_64K);
const MISC_MAP_B0000_32K: u32 = misc_window(VGA_GC_MEMORY_MAP_B0000_32K);
const MISC_MAP_B8000_32K: u32 = misc_window(VGA_GC_MEMORY_MAP_B8000_32K);

fn write_gc(vga: &mut VgaText, index: u32, value: u32) {
    vga.port_write(VGA_GC_INDEX, 1, index);
    vga.port_write(VGA_GC_DATA, 1, value);
}

fn write_seq(vga: &mut VgaText, index: u32, value: u32) {
    vga.port_write(VGA_SEQ_INDEX, 1, index);
    vga.port_write(VGA_SEQ_DATA, 1, value);
}

#[test]
fn reset_default_selects_the_b8000_text_window() {
    let vga = VgaText::new();
    assert_eq!(vga.gc_memory_map_select(), VGA_GC_MEMORY_MAP_B8000_32K);
    assert!(vga.gc_chain_odd_even());
    assert!(!vga.gc_graphics_mode());
    assert_eq!(vga.display_window(), (VGA_TEXT_BASE, VGA_TEXT_END));
    assert!(vga.owns_display_addr(VGA_TEXT_BASE));
    assert!(!vga.owns_display_addr(VGA_TEXT_BASE - 1));
}

#[test]
fn memory_map_select_moves_the_display_window() {
    let mut vga = VgaText::new();

    write_gc(&mut vga, GC_MISC, MISC_MAP_A0000_128K);
    assert_eq!(
        vga.display_window(),
        (VGA_WINDOW_A0000_BASE, VGA_WINDOW_A0000_BASE + 0x2_0000)
    );

    write_gc(&mut vga, GC_MISC, MISC_MAP_A0000_64K);
    assert_eq!(
        vga.display_window(),
        (VGA_WINDOW_A0000_BASE, VGA_WINDOW_B0000_BASE)
    );

    write_gc(&mut vga, GC_MISC, MISC_MAP_B0000_32K);
    assert_eq!(vga.display_window(), (VGA_WINDOW_B0000_BASE, VGA_TEXT_BASE));

    write_gc(&mut vga, GC_MISC, MISC_MAP_B8000_32K);
    assert_eq!(vga.display_window(), (VGA_TEXT_BASE, VGA_TEXT_END));
}

/// A window that excludes `0xB8000` must not claim CPU accesses there.
#[test]
fn text_window_accesses_are_not_claimed_outside_the_selected_window() {
    let mut vga = VgaText::new();
    assert!(vga.write_u8(VGA_TEXT_BASE, b'Q'));

    write_gc(&mut vga, GC_MISC, MISC_MAP_B0000_32K);
    assert_eq!(vga.read_u8(VGA_TEXT_BASE), None);
    assert!(!vga.write_u8(VGA_TEXT_BASE, b'!'));
    assert!(vga.gc_read_u8(VGA_TEXT_BASE).is_none());
    assert!(!vga.gc_write_u8(VGA_TEXT_BASE, 0xFF));

    // The B0000 window itself decodes to map offset 0.
    assert!(vga.owns_display_addr(VGA_WINDOW_B0000_BASE));
    assert_eq!(vga.plane_offset(VGA_WINDOW_B0000_BASE), Some(0));

    write_gc(&mut vga, GC_MISC, MISC_MAP_B8000_32K);
    assert_eq!(vga.read_u8(VGA_TEXT_BASE), Some(b'Q'), "buffer preserved");
    assert!(vga.write_u8(VGA_TEXT_BASE, b'R'));
    assert_eq!(vga.read_u8(VGA_TEXT_BASE), Some(b'R'));
}

/// Map offsets are relative to the base of the selected window.
#[test]
fn map_offsets_are_relative_to_the_window_base() {
    let mut vga = VgaText::new();
    write_gc(&mut vga, GC_MISC, MISC_MAP_A0000_128K);
    assert_eq!(vga.plane_offset(VGA_WINDOW_A0000_BASE), Some(0));
    assert_eq!(vga.plane_offset(VGA_WINDOW_A0000_BASE + 0x10), Some(0x10));

    write_gc(&mut vga, GC_MISC, MISC_MAP_A0000_64K);
    assert_eq!(vga.plane_offset(VGA_WINDOW_A0000_BASE + 0x10), Some(0x10));
    assert_eq!(
        vga.plane_offset(VGA_WINDOW_B0000_BASE),
        None,
        "64 KB window ends"
    );

    write_gc(&mut vga, GC_MISC, MISC_MAP_B8000_32K);
    assert_eq!(vga.plane_offset(VGA_TEXT_BASE + 0x10), Some(0x10));
}

/// Spec: Figure 2-74 OE — Chain Odd/Even selects the odd map when A0 is 1 and
/// the even map when A0 is 0, independently of Sequencer Memory Mode bit2.
#[test]
fn chain_odd_even_forces_odd_even_addressing() {
    let mut vga = VgaText::new();
    // Sequencer says "sequential" (Odd/Even disable set) but GC Misc chains.
    write_seq(
        &mut vga,
        SEQ_MEMORY_MODE,
        VGA_SEQ_MEMORY_MODE_EXTENDED as u32 | VGA_SEQ_MEMORY_MODE_ODD_EVEN_DISABLE as u32,
    );
    write_gc(
        &mut vga,
        GC_MISC,
        MISC_MAP_B8000_32K | VGA_GC_MISC_CHAIN_ODD_EVEN as u32,
    );
    assert!(!vga.seq_odd_even_enabled());
    assert!(vga.gc_chain_odd_even());
    assert_eq!(vga.plane_write_mask(VGA_TEXT_BASE), 0b0001);
    assert_eq!(vga.plane_write_mask(VGA_TEXT_BASE + 1), 0b0010);
    assert_eq!(vga.plane_offset(VGA_TEXT_BASE + 1), Some(0));

    // Clearing both odd/even sources restores planar addressing.
    write_gc(&mut vga, GC_MISC, MISC_MAP_B8000_32K);
    assert!(!vga.gc_chain_odd_even());
    assert_eq!(vga.plane_offset(VGA_TEXT_BASE + 1), Some(1));
}

/// Spec: FreeVGA / IBM Misc Output bit1 — RAM Enable still gates CPU access
/// inside the selected window.
#[test]
fn ram_enable_still_gates_the_selected_window() {
    let mut vga = VgaText::new();
    let ram_disabled = (VGA_MISC_OUTPUT_DEFAULT & !VGA_MISC_RAM_ENABLE) as u32;
    vga.port_write(VGA_MISC_OUTPUT_WRITE, 1, ram_disabled);
    assert!(!vga.owns_display_addr(VGA_TEXT_BASE));
    assert_eq!(vga.read_u8(VGA_TEXT_BASE), None);
    assert!(!vga.write_u8(VGA_TEXT_BASE, 0x41));

    vga.port_write(VGA_MISC_OUTPUT_WRITE, 1, VGA_MISC_OUTPUT_DEFAULT as u32);
    assert!(vga.owns_display_addr(VGA_TEXT_BASE));
    assert!(vga.write_u8(VGA_TEXT_BASE, 0x41));
}

#[test]
fn reset_restores_the_b8000_window() {
    let mut vga = VgaText::new();
    write_gc(&mut vga, GC_MISC, MISC_MAP_A0000_64K);
    assert_eq!(
        vga.display_window(),
        (VGA_WINDOW_A0000_BASE, VGA_WINDOW_B0000_BASE)
    );
    vga.reset();
    assert_eq!(vga.gc_memory_map_select(), VGA_GC_MEMORY_MAP_B8000_32K);
    assert_eq!(vga.display_window(), (VGA_TEXT_BASE, VGA_TEXT_END));
    assert_eq!(vga.read_u8(VGA_TEXT_BASE), Some(b' '));
}
