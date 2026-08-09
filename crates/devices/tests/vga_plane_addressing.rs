//! Device-level tests for the VGA Sequencer Memory Mode plane-addressing model
//! through the public `devices` surface (port I/O in, host queries out).
//!
//! Spec: IBM PS/2 Hardware Interface Technical Reference — Video Subsystems
//! (Sep 1992) Figure 2-29 (Map Mask), Figure 2-33 (Memory Mode: Extended
//! Memory / Odd-Even / Chain 4) and Figure 2-34 (Map Selection, Chain 4);
//! OSDev VGA Hardware "Addressing Logic" for the per-map offset forms.
//!
//! The plane-decode value types are not re-exported from `devices`, so these
//! tests assert through `VgaText` methods and integer results only. See
//! `docs/vga-plane-memory-model.md`.

use devices::{PortDevice, VgaText, VGA_SEQ_DATA, VGA_SEQ_INDEX, VGA_TEXT_BASE, VGA_TEXT_END};

/// Sequencer Map Mask register index (`0x02`).
const SEQ_MAP_MASK: u32 = 0x02;
/// Sequencer Memory Mode register index (`0x04`).
const SEQ_MEMORY_MODE: u32 = 0x04;
/// Memory Mode bit1 Extended Memory.
const MEMORY_MODE_EXTENDED: u32 = 0x02;
/// Memory Mode bit2 Odd/Even disable.
const MEMORY_MODE_ODD_EVEN_DISABLE: u32 = 0x04;
/// Memory Mode bit3 Chain 4.
const MEMORY_MODE_CHAIN4: u32 = 0x08;

fn write_seq(vga: &mut VgaText, index: u32, value: u32) {
    vga.port_write(VGA_SEQ_INDEX, 1, index);
    vga.port_write(VGA_SEQ_DATA, 1, value);
}

#[test]
fn mode03h_reset_state_decodes_text_character_and_attribute_maps() {
    let vga = VgaText::new();
    assert!(vga.seq_odd_even_enabled());
    assert!(!vga.seq_chain4_enabled());
    assert!(vga.seq_extended_memory());
    assert_eq!(vga.seq_map_mask(), 0x03);

    // Map Mask 0x03 narrows the odd/even pairs to map 0 (character) and map 1
    // (attribute); both share one map offset.
    assert_eq!(vga.plane_write_mask(VGA_TEXT_BASE), 0b0001);
    assert_eq!(vga.plane_write_mask(VGA_TEXT_BASE + 1), 0b0010);
    assert_eq!(vga.plane_offset(VGA_TEXT_BASE), Some(0));
    assert_eq!(vga.plane_offset(VGA_TEXT_BASE + 1), Some(0));
    assert_eq!(vga.plane_offset(VGA_TEXT_BASE + 2), Some(2));
}

#[test]
fn chain4_programming_selects_one_map_per_low_address_pair() {
    let mut vga = VgaText::new();
    write_seq(
        &mut vga,
        SEQ_MEMORY_MODE,
        MEMORY_MODE_EXTENDED | MEMORY_MODE_CHAIN4,
    );
    write_seq(&mut vga, SEQ_MAP_MASK, 0x0F);
    assert!(vga.seq_chain4_enabled());

    assert_eq!(vga.plane_write_mask(VGA_TEXT_BASE), 0b0001);
    assert_eq!(vga.plane_write_mask(VGA_TEXT_BASE + 1), 0b0010);
    assert_eq!(vga.plane_write_mask(VGA_TEXT_BASE + 2), 0b0100);
    assert_eq!(vga.plane_write_mask(VGA_TEXT_BASE + 3), 0b1000);
    assert_eq!(vga.plane_offset(VGA_TEXT_BASE + 3), Some(0));
    assert_eq!(vga.plane_offset(VGA_TEXT_BASE + 4), Some(4));
}

#[test]
fn planar_programming_reports_map_mask_and_unshifted_offset() {
    let mut vga = VgaText::new();
    // Graphics Controller Miscellaneous must also drop Chain Odd/Even
    // (IBM Figure 2-74 OE) for planar host addressing; keep the B8000 window.
    vga.port_write(devices::VGA_GC_INDEX, 1, 0x06);
    vga.port_write(devices::VGA_GC_DATA, 1, 0x0D);
    write_seq(
        &mut vga,
        SEQ_MEMORY_MODE,
        MEMORY_MODE_EXTENDED | MEMORY_MODE_ODD_EVEN_DISABLE,
    );
    write_seq(&mut vga, SEQ_MAP_MASK, 0b0110);
    assert!(!vga.seq_odd_even_enabled());

    assert_eq!(vga.plane_write_mask(VGA_TEXT_BASE + 0x11), 0b0110);
    assert_eq!(vga.plane_offset(VGA_TEXT_BASE + 0x11), Some(0x11));
}

#[test]
fn extended_memory_clear_shrinks_the_addressable_map() {
    let mut vga = VgaText::new();
    write_seq(&mut vga, SEQ_MEMORY_MODE, MEMORY_MODE_ODD_EVEN_DISABLE);
    assert!(!vga.seq_extended_memory());
    assert_eq!(vga.plane_size_bytes(), 0x4000);
    assert_eq!(vga.plane_offset(VGA_TEXT_BASE + 0x4010), Some(0x10));
}

#[test]
fn display_window_bounds_the_plane_decode() {
    let vga = VgaText::new();
    assert_eq!(vga.display_window(), (VGA_TEXT_BASE, VGA_TEXT_END));
    assert_eq!(vga.plane_offset(VGA_TEXT_END), None);
    assert_eq!(vga.plane_write_mask(VGA_TEXT_END), 0);
}
